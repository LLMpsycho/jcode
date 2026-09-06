//! Task cleanup.

use super::*;

impl BackgroundTaskManager {
    /// Abort every live in-process task before an exec-based server reload.
    ///
    /// `exec` replaces the process image without running destructors, so
    /// without this the spawned task futures simply vanish: their
    /// `kill_on_drop` children (e.g. cargo builds) are never killed and keep
    /// running orphaned, and their status files stay `Running` until the next
    /// process's reconcile sweep happens to notice. Aborting the handles here
    /// drops the futures (killing children) and persisting a terminal
    /// `Failed` status makes the interruption deterministic and immediately
    /// visible to `bg wait`/`bg status` and self-dev queue reconciliation.
    ///
    /// Returns the number of tasks finalized.
    pub async fn abort_live_tasks_for_reload(&self) -> usize {
        let tasks: Vec<RunningTask> = {
            let mut map = self.tasks.write().await;
            map.drain().map(|(_, task)| task).collect()
        };
        let mut finalized = 0;

        for task in tasks {
            task.handle.abort();
            // Wait (bounded) for the aborted future to actually drop, so
            // kill_on_drop children are killed before the upcoming exec.
            match tokio::time::timeout(Duration::from_secs(2), task.handle).await {
                Ok(Ok(Ok(_))) => {}
                Ok(Ok(Err(_))) => crate::logging::warn(
                    "Background task returned an error while stopping for reload",
                ),
                Ok(Err(error)) if error.is_cancelled() => {}
                Ok(Err(_)) => {
                    crate::logging::warn("Background task failed while being stopped for reload")
                }
                Err(_) => crate::logging::warn(
                    "Background task cancellation exceeded the reload deadline",
                ),
            }

            let (notify_flag, wake_flag) = *task.delivery_flags.borrow();
            let prior_status = self.read_status_file(&task.status_path).await;
            // If the task won the race and finished naturally, keep its real
            // terminal status instead of stamping it as interrupted.
            if prior_status
                .as_ref()
                .is_some_and(|status| status.status != BackgroundTaskStatus::Running)
            {
                continue;
            }
            let error = "Interrupted by server reload: the owning server process was replaced before the task finished".to_string();
            let mut final_status = TaskStatusFile {
                task_id: task.task_id,
                tool_name: task.tool_name,
                display_name: prior_status
                    .as_ref()
                    .and_then(|status| status.display_name.clone())
                    .or(task.display_name),
                session_id: task.session_id,
                status: BackgroundTaskStatus::Failed,
                exit_code: None,
                error: Some(error.clone()),
                started_at: task.started_at_rfc3339,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_secs: Some(task.started_at.elapsed().as_secs_f64()),
                pid: None,
                owner_pid: Some(std::process::id()),
                owner_instance: Some(model::process_instance_token().to_string()),
                detached: false,
                notify: notify_flag,
                wake: wake_flag,
                progress: prior_status
                    .as_ref()
                    .and_then(|status| status.progress.clone()),
                event_history: prior_status
                    .map(|status| status.event_history)
                    .unwrap_or_else(Vec::new),
                stall_wake_seconds: None,
            };
            push_task_event(
                &mut final_status,
                terminal_event_record(BackgroundTaskStatus::Failed, None, Some(&error)),
            );
            self.write_status_file(&task.status_path, &final_status)
                .await;
            finalized += 1;
        }

        finalized
    }

    /// Clean up old task files (older than specified hours)
    pub async fn cleanup(&self, max_age_hours: u64) -> Result<usize> {
        Ok(self
            .cleanup_filtered(max_age_hours, &std::collections::HashSet::new(), false)
            .await?
            .removed_files)
    }

    /// Clean up old task files, skipping running tasks and optionally filtering by status.
    pub async fn cleanup_filtered(
        &self,
        max_age_hours: u64,
        status_filter: &std::collections::HashSet<&str>,
        dry_run: bool,
    ) -> Result<BackgroundCleanupResult> {
        let mut result = BackgroundCleanupResult {
            matched_files: 0,
            removed_files: 0,
            skipped_running_files: 0,
        };
        let cutoff =
            std::time::SystemTime::now() - std::time::Duration::from_secs(max_age_hours * 3600);

        if let Ok(mut entries) = fs::read_dir(&self.output_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let Ok(metadata) = fs::metadata(&path).await else {
                    continue;
                };
                let Ok(modified) = metadata.modified() else {
                    continue;
                };
                if modified >= cutoff {
                    continue;
                }

                let mut associated_status = None;
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    associated_status = self.read_status_file(&path).await;
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("output")
                    && let Some(task_id) = path.file_stem().and_then(|stem| stem.to_str())
                {
                    associated_status = self.status(task_id).await;
                }

                if let Some(status) = associated_status.as_ref() {
                    if status.status == BackgroundTaskStatus::Running {
                        result.skipped_running_files += 1;
                        continue;
                    }
                    let status_label = match status.status {
                        BackgroundTaskStatus::Running => "running",
                        BackgroundTaskStatus::Completed => "completed",
                        BackgroundTaskStatus::Superseded => "superseded",
                        BackgroundTaskStatus::Failed => "failed",
                    };
                    if !status_filter.is_empty() && !status_filter.contains(status_label) {
                        continue;
                    }
                } else if !status_filter.is_empty() {
                    continue;
                }

                result.matched_files += 1;
                if !dry_run {
                    fs::remove_file(&path).await?;
                    result.removed_files += 1;
                }
            }
        }

        if dry_run {
            result.removed_files = result.matched_files;
        }

        Ok(result)
    }

    /// Best-effort synchronous snapshot of currently running tasks.
    /// This avoids async calls in render paths.
    pub fn running_snapshot(&self) -> (usize, Vec<String>, Option<RunningBackgroundProgress>) {
        let Ok(tasks) = self.tasks.try_read() else {
            return (0, Vec::new(), None);
        };

        let mut rows: Vec<RunningBackgroundProgress> = Vec::new();
        for task in tasks.values() {
            let status = match std::fs::read_to_string(&task.status_path) {
                Ok(content) => match serde_json::from_str::<TaskStatusFile>(&content) {
                    Ok(status) => Some(status),
                    Err(_) => {
                        crate::logging::debug("Background task status snapshot is invalid");
                        None
                    }
                },
                Err(_) => {
                    crate::logging::debug("Background task status snapshot could not be read");
                    None
                }
            };
            let progress = status.as_ref().and_then(|status| status.progress.clone());
            let label = status
                .as_ref()
                .and_then(|status| status.display_name.clone())
                .or_else(|| task.display_name.clone())
                .unwrap_or_else(|| task.tool_name.clone());

            rows.push(RunningBackgroundProgress {
                task_id: task.task_id.clone(),
                tool_name: task.tool_name.clone(),
                label,
                detail: progress.map(|progress| format_progress_display(&progress, 10)),
            });
        }

        rows.sort_by(|a, b| b.task_id.cmp(&a.task_id));
        let latest = rows.iter().find(|row| row.detail.is_some()).cloned();

        (
            tasks.len(),
            rows.iter().map(|row| row.label.clone()).collect(),
            latest,
        )
    }

    /// Best-effort synchronous lookup of detached tasks that are still running
    /// for a specific session.
    ///
    /// This is primarily used during self-dev reload recovery, where the new
    /// process needs to remind the agent that a previous `bash` command was
    /// persisted into the background instead of being interrupted.
    pub fn persisted_detached_running_tasks_for_session(
        &self,
        session_id: &str,
    ) -> Vec<TaskStatusFile> {
        let mut matches = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.output_dir) else {
            return matches;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(status) = serde_json::from_str::<TaskStatusFile>(&content) else {
                continue;
            };

            if status.session_id != session_id
                || status.status != BackgroundTaskStatus::Running
                || !status.detached
            {
                continue;
            }

            let Some(pid) = status.pid else {
                continue;
            };

            if crate::platform::is_process_running(pid) {
                matches.push(status);
            }
        }

        matches.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        matches
    }
}
