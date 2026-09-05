//! Safe-boundary integration between the primary turn and its background advisors.

use super::*;
use crate::advisor::{AdvisorManager, AdvisorTurnInput, AdvisorUpdateContext};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PRIMARY_TURN: AtomicU64 = AtomicU64::new(1);
const TRANSCRIPT_BYTES: usize = 16 * 1024;
const BLOCK_BYTES: usize = 4 * 1024;
const TERMINAL_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ADVISOR_CORRECTIONS: usize = 3;

pub(super) struct AdvisorTurnState {
    objective: String,
    primary_turn_id: u64,
    cursor: usize,
    bootstrap: Option<String>,
    config: crate::config::AdvisorConfig,
    investigation: Option<Arc<crate::advisor::investigation::AdvisorInvestigation>>,
    corrections: usize,
}

/// Fences every exit path, including dropping a cancelled server request future.
pub(super) struct AdvisorTurnGuard {
    manager: Arc<AdvisorManager>,
    session_id: String,
}

impl Drop for AdvisorTurnGuard {
    fn drop(&mut self) {
        self.manager.cancel_turn(&self.session_id);
    }
}

fn excerpt(value: &str, limit: usize) -> String {
    crate::advisor::investigation::bounded_excerpt(value, limit)
}

fn input_excerpt(input: &serde_json::Value) -> String {
    crate::advisor::investigation::bounded_json_excerpt(input, BLOCK_BYTES)
}

/// Observe visible content only. Provider reasoning, signatures, images and
/// encrypted compaction items never cross the primary/advisor boundary.
fn visible_delta(messages: &[StoredMessage]) -> String {
    let mut blocks = VecDeque::new();
    let mut total = 0usize;
    let mut omitted = false;
    for message in messages {
        for block in &message.content {
            let text = match block {
                ContentBlock::Text { text, .. } => {
                    if message.display_role == Some(StoredDisplayRole::System)
                        && text.starts_with("[ADVISOR ")
                    {
                        continue;
                    }
                    format!("{:?}: {}", message.role, excerpt(text, BLOCK_BYTES))
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    format!(
                        "Tool request {}: {}",
                        excerpt(name, 128),
                        input_excerpt(input)
                    )
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    format!(
                        "Tool result {} (error={}): {}",
                        excerpt(tool_use_id, 128),
                        is_error.unwrap_or(false),
                        excerpt(content, BLOCK_BYTES)
                    )
                }
                _ => continue,
            };
            total += text.len() + 1;
            blocks.push_back(format!("{text}\n"));
            while total > TRANSCRIPT_BYTES && blocks.len() > 1 {
                if let Some(old) = blocks.pop_front() {
                    total -= old.len();
                    omitted = true;
                }
            }
        }
    }
    let mut result = if omitted {
        "[older visible context omitted]\n".to_string()
    } else {
        String::new()
    };
    for block in blocks {
        result.push_str(&block);
    }
    result
}

fn task_context(messages: &[StoredMessage], current: &str) -> String {
    let earlier = messages
        .iter()
        .filter(|message| message.role == Role::User && message.display_role.is_none())
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. }
                if !text.starts_with("<system-reminder>")
                    && !text.starts_with("[NOTIFICATION]") =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .find(|text| *text != current);
    match earlier {
        Some(earlier) => format!(
            "Earlier user objective (later instructions may supersede it):\n{}\n\nCurrent user request:\n{}",
            excerpt(earlier, 1800),
            excerpt(current, 1800)
        ),
        None => excerpt(current, BLOCK_BYTES),
    }
}

use std::collections::VecDeque;

impl Agent {
    pub(super) async fn begin_advisor_turn(&mut self, objective: &str) -> Option<AdvisorTurnGuard> {
        self.advisor_turn = None;
        let config = crate::advisor::config_for_current_session();
        let manager = crate::advisor::advisor_manager();
        if config.max_reviews_per_session == 0
            || config.max_notes_per_turn == 0
            || !crate::advisor::roster::is_enabled(
                &manager,
                &self.session.id,
                &config,
                self.session
                    .working_dir
                    .as_deref()
                    .map(std::path::Path::new),
            )
        {
            return None;
        }
        let guard = AdvisorTurnGuard {
            manager: Arc::clone(&manager),
            session_id: self.session.id.clone(),
        };
        let investigation = self
            .working_dir()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .and_then(
                |working_dir| match crate::advisor::investigation::AdvisorInvestigation::new(
                    self.registry.clone(),
                    self.session.id.clone(),
                    working_dir,
                ) {
                    Ok(tools) => Some(Arc::new(tools)),
                    Err(error) => {
                        logging::warn(&format!("Advisor investigation unavailable: {error}"));
                        None
                    }
                },
            );
        manager.begin_live_capture(&self.session.id);
        self.advisor_turn = Some(AdvisorTurnState {
            objective: task_context(&self.session.messages, objective),
            primary_turn_id: NEXT_PRIMARY_TURN.fetch_add(1, Ordering::Relaxed),
            // Include the current user message. Older history belongs to the
            // advisor's existing conversation, not a repeated full transcript.
            cursor: self.message_count().saturating_sub(1),
            bootstrap: Some(format!(
                "Project instructions captured for this session:\n{}\n\nRecent visible session context (may be incomplete; current user instructions take precedence):\n{}",
                excerpt(
                    self.agents_md_snapshot
                        .0
                        .as_deref()
                        .unwrap_or("No project instructions supplied."),
                    4000
                ),
                visible_delta(
                    &self.session.messages[self.session.messages.len().saturating_sub(64)..]
                ),
            )),
            config,
            investigation,
            corrections: 0,
        });
        self.observe_advisor_step(false).await;
        Some(guard)
    }

    pub(super) async fn observe_advisor_step(&mut self, completed: bool) {
        if self.is_graceful_shutdown() {
            return;
        }
        let Some(state) = self.advisor_turn.as_mut() else {
            return;
        };
        let manager = crate::advisor::advisor_manager();
        if !crate::advisor::roster::is_enabled(
            &manager,
            &self.session.id,
            &state.config,
            self.session
                .working_dir
                .as_deref()
                .map(std::path::Path::new),
        ) {
            // /advisor off takes effect during a running turn, including the
            // potentially expensive Git/LSP evidence collection.
            return;
        }
        let count = self.session.messages.len();
        if !completed && count == state.cursor {
            return;
        }
        let delta = state
            .bootstrap
            .take()
            .unwrap_or_else(|| visible_delta(&self.session.messages[state.cursor.min(count)..]));
        state.cursor = count;
        let mut input = AdvisorTurnInput {
            objective: state.objective.clone(),
            latest_primary_turn: delta,
            verification_status: if completed {
                "primary answer complete; inspect before finalization"
            } else {
                "primary work in progress"
            }
            .to_string(),
            ..Default::default()
        };
        let context = AdvisorUpdateContext {
            completed_primary_turn: completed,
            primary_turn_id: state.primary_turn_id,
            working_dir: self.session.working_dir.as_ref().map(PathBuf::from),
            investigation: state.investigation.clone(),
            instructions: excerpt(
                self.agents_md_snapshot.0.as_deref().unwrap_or_default(),
                4000,
            ),
            ..Default::default()
        };
        let config = state.config.clone();
        manager
            .enrich_input(
                &self.session.id,
                &mut input,
                self.session.working_dir.as_deref(),
            )
            .await;
        let _ = crate::advisor::roster::schedule_updates(
            &manager,
            self.session.id.clone(),
            self.provider_fork(),
            self.soft_interrupt_queue(),
            input,
            config,
            context,
        );
    }

    /// Return true only when new eligible advice warrants another primary
    /// request. The existing turn stays alive until a bounded drain completes,
    /// so a late blocker cannot disappear behind the server's Done event.
    pub(super) async fn finish_advisor_step(
        &mut self,
        event_tx: Option<&mpsc::UnboundedSender<ServerEvent>>,
        print_output: bool,
    ) -> bool {
        if self.advisor_turn.is_none() || self.is_graceful_shutdown() {
            return false;
        }
        let manager = crate::advisor::advisor_manager();
        manager.prepare_terminal_delivery(&self.session.id);
        self.observe_advisor_step(true).await;
        let drained = tokio::select! {
            biased;
            _ = self.graceful_shutdown.notified() => false,
            idle = manager.wait_for_idle(&self.session.id, TERMINAL_DRAIN_TIMEOUT) => idle,
        };
        self.display_advisor_asides(event_tx, print_output);
        if !drained || self.is_graceful_shutdown() {
            manager.cancel_turn(&self.session.id);
            if !self.is_graceful_shutdown() {
                self.display_advisor_notice("Advisor review reached its time limit; the final review is incomplete. Inspect advisor status before relying on this result.", event_tx, print_output);
            }
            return false;
        }
        let at_limit = self
            .advisor_turn
            .as_ref()
            .is_some_and(|state| state.corrections >= MAX_ADVISOR_CORRECTIONS);
        let has_advice = self.soft_interrupt_queue.lock().is_ok_and(|queue| {
            queue.iter().any(|message| {
                message.source == SoftInterruptSource::System
                    && message.content.starts_with("[ADVISOR ")
            })
        });
        if at_limit && has_advice {
            manager.cancel_turn(&self.session.id);
            self.display_advisor_notice("Advisor correction limit reached; unresolved advice remains available in /advisor inspect.", event_tx, print_output);
            return false;
        }
        let injected = self.inject_soft_interrupts();
        if injected.is_empty() {
            return false;
        }
        if has_advice && let Some(state) = self.advisor_turn.as_mut() {
            state.corrections += 1;
        }
        if let Some(event_tx) = event_tx {
            for event in Self::build_soft_interrupt_events(injected, "advisor_terminal", None) {
                let _ = event_tx.send(event);
            }
        }
        true
    }

    fn display_advisor_notice(
        &self,
        message: &str,
        event_tx: Option<&mpsc::UnboundedSender<ServerEvent>>,
        print_output: bool,
    ) {
        if let Some(event_tx) = event_tx {
            let _ = event_tx.send(ServerEvent::Notification {
                from_session: format!("advisor:{}", self.session.id),
                from_name: Some("Advisor".to_string()),
                notification_type: crate::protocol::NotificationType::Message {
                    scope: Some("dm".to_string()),
                    channel: None,
                    tldr: None,
                },
                message: message.to_string(),
            });
        } else if print_output {
            crate::terminal_println!("\n[ADVISOR] {message}");
        }
    }

    pub(super) fn display_advisor_asides(
        &self,
        event_tx: Option<&mpsc::UnboundedSender<ServerEvent>>,
        print_output: bool,
    ) {
        for note in crate::advisor::advisor_manager().take_asides(&self.session.id) {
            let mut message = format!(
                "[ADVISOR {:?}] {}\nRecommended action: {}\nNote: {}",
                note.severity, note.summary, note.recommended_action, note.id
            );
            for evidence in note.evidence.iter().take(3) {
                message.push_str("\nEvidence: ");
                message.push_str(&excerpt(evidence, 512));
            }
            if let Some(event_tx) = event_tx {
                let _ = event_tx.send(ServerEvent::Notification {
                    from_session: format!("advisor:{}", self.session.id),
                    from_name: Some("Advisor".to_string()),
                    notification_type: crate::protocol::NotificationType::Message {
                        scope: Some("dm".to_string()),
                        channel: None,
                        tldr: Some(note.summary),
                    },
                    message,
                });
            } else if print_output {
                crate::terminal_println!("\n{message}");
            }
        }
    }
}

#[cfg(test)]
#[path = "advisor_live_tests.rs"]
mod tests;
