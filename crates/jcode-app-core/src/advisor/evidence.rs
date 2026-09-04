use super::*;
use crate::tool::ToolOutput;
use serde_json::Value;
use std::path::Path;
use tokio::io::AsyncReadExt;

const MAX_ITEMS: usize = 32;

pub(super) fn grounded(input: &AdvisorTurnInput, note: &AdvisorNote) -> bool {
    let sources = [
        &input.objective,
        &input.latest_primary_turn,
        &input.diff_summary,
        &input.diagnostics,
        &input.verification_status,
        &input.outstanding_todos,
        &input.acceptance_criteria,
    ];
    !note.evidence.is_empty()
        && note.evidence.iter().all(|quote| {
            !quote.trim().is_empty()
                && (sources.iter().any(|source| source.contains(quote))
                    || input.tools.iter().any(|tool| tool.result.contains(quote)))
        })
}

#[derive(Default)]
pub(super) struct TurnEvidence {
    tools: VecDeque<AdvisorToolInput>,
    changes: Vec<String>,
    diagnostics: Vec<String>,
    verification: Vec<String>,
}

fn clean(value: &str) -> String {
    truncate_utf8(redact_secrets(value), 512)
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(|v| clean(v.as_str().unwrap_or_else(|| "")))
        .unwrap_or_default()
}

fn push(items: &mut Vec<String>, value: String) {
    if items.len() < MAX_ITEMS {
        items.push(clean(&value));
    }
}

impl AdvisorManager {
    pub fn begin_capture(&self, session: &str, config: &AdvisorConfig) {
        if config.max_reviews_per_session == 0
            || config.max_notes_per_turn == 0
            || !self.is_enabled(session, config.enabled)
        {
            return;
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.entry(session.to_string()).or_default().capture =
                Some(TurnEvidence::default());
        }
    }

    /// Consume only producer-owned metadata. Never scan/replay raw terminal
    /// output, infer a successful test from a successful agent turn, or load a
    /// previous turn's tool summaries into this turn's evidence.
    pub fn capture_tool(
        &self,
        session: &str,
        name: &str,
        input: &Value,
        result: &anyhow::Result<ToolOutput>,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let Some(runtime) = sessions.get_mut(session) else {
            return;
        };
        let Some(capture) = runtime.capture.as_mut() else {
            return;
        };
        capture.tools.push_back(AdvisorToolInput {
            name: clean(name),
            intent: input.get("intent").and_then(Value::as_str).map(clean),
            result: match result {
                Ok(output) => format!(
                    "completed; {} output bytes (raw output omitted)",
                    output.output.len()
                ),
                Err(_) => "tool failed; no successful verification result".into(),
            },
        });
        while capture.tools.len() > MAX_TOOLS {
            capture.tools.pop_front();
        }
        let explicit_check = input.get("verification").and_then(Value::as_bool) == Some(true);
        let Ok(output) = result else {
            if explicit_check {
                push(
                    &mut capture.verification,
                    format!("{}: tool failed", field(input, "intent")),
                );
            }
            return;
        };
        let Some(metadata) = output.metadata.as_ref() else {
            if explicit_check {
                push(
                    &mut capture.verification,
                    "check has no recorded process result".into(),
                );
            }
            return;
        };
        for file in metadata
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_ITEMS)
        {
            let revision = |key| {
                file.get(key)
                    .and_then(|r| r.get("revision"))
                    .and_then(Value::as_u64)
            };
            push(
                &mut capture.changes,
                format!(
                    "{}: revision {:?} -> {:?}",
                    field(file, "path"),
                    revision("revision_before"),
                    revision("revision_after")
                ),
            );
        }
        if let Some(execution) = metadata.get("execution") {
            let code = execution.get("exit_code").and_then(Value::as_i64);
            let status = match code {
                Some(0) => "passed",
                Some(_) => "failed",
                None => "unknown/signal termination",
            };
            push(
                &mut capture.verification,
                format!(
                    "{} {}: {status}, exit {code:?}",
                    if explicit_check {
                        "Declared check"
                    } else {
                        "Process (not declared as a verification check)"
                    },
                    input
                        .get("intent")
                        .and_then(Value::as_str)
                        .map(clean)
                        .unwrap_or_else(|| clean(name))
                ),
            );
        } else if explicit_check {
            push(
                &mut capture.verification,
                if metadata.get("background") == Some(&Value::Bool(true)) {
                    "Declared check is still running in background; no completion result".into()
                } else {
                    "Declared check has no process completion evidence".into()
                },
            );
        }
        // Phase 2's semantic_verification contract contains diagnostic deltas.
        for file in metadata
            .pointer("/semantic_verification/files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_ITEMS)
        {
            push(
                &mut capture.verification,
                format!("LSP {}: {}", field(file, "path"), field(file, "status")),
            );
            record_diagnostics(
                capture,
                &mut runtime.seen_diagnostics,
                &field(file, "path"),
                file.get("diagnostics"),
            );
        }
        // Direct LSP reads are snapshots; suppress findings already observed.
        if let Some(diagnostics) = metadata
            .pointer("/items/diagnostic_evidence")
            .or_else(|| metadata.get("diagnostic_evidence"))
        {
            push(
                &mut capture.verification,
                format!(
                    "LSP snapshot {}: {}",
                    field(diagnostics, "path"),
                    field(diagnostics, "freshness")
                ),
            );
            record_diagnostics(
                capture,
                &mut runtime.seen_diagnostics,
                &field(diagnostics, "path"),
                diagnostics.get("items"),
            );
        }
    }

    pub async fn enrich_input(
        &self,
        session: &str,
        input: &mut AdvisorTurnInput,
        working_dir: Option<&str>,
    ) {
        let captured = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.get_mut(session).and_then(|r| r.capture.take()))
            .unwrap_or_default();
        input.tools = captured.tools.into_iter().collect();
        input.diff_summary = captured.changes.join("\n");
        let diff = bounded_diff(working_dir.map(Path::new)).await;
        input.diff_summary.push_str(&format!("\n{diff}"));
        input.diagnostics = if captured.diagnostics.is_empty() {
            "No new diagnostic evidence captured (not proof of clean diagnostics).".into()
        } else {
            captured.diagnostics.join("\n")
        };
        input
            .verification_status
            .push_str("; turn completion is not verification.\n");
        if captured.verification.is_empty() {
            input
                .verification_status
                .push_str("No verification result captured.");
        } else {
            input
                .verification_status
                .push_str(&captured.verification.join("\n"));
        }
        // Existing stable todo contracts, not transcript scraping or hidden
        // reasoning. These are declared goals/checks, not evidence of success.
        let todos = crate::todo::load_todos(session);
        input.outstanding_todos = match todos {
            Ok(todos) => todos
                .iter()
                .filter(|todo| {
                    !crate::todo::todo_status_is_completed(&todo.status)
                        && !crate::todo::todo_status_is_cancelled(&todo.status)
                })
                .take(MAX_ITEMS)
                .map(|todo| {
                    format!(
                        "{} [{}]: {}",
                        clean(&todo.id),
                        clean(&todo.status),
                        clean(&todo.content)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Err(_) => "Todo state unavailable.".into(),
        };
        let mut criteria = vec!["Acceptance source: user objective; the following are agent-declared plan/checks, not independently verified outcomes.".to_string()];
        if let Ok(plan) = crate::todo::load_plan(session) {
            for criterion in plan
                .acceptance_criteria
                .as_deref()
                .unwrap_or_default()
                .iter()
                .take(MAX_ITEMS)
            {
                push(
                    &mut criteria,
                    format!("Declared requirement: {}", clean(criterion)),
                );
            }
            if let Some(intention) = plan.user_intention {
                push(
                    &mut criteria,
                    format!("Plan intention: {}", clean(&intention)),
                );
            }
        }
        if let Ok(goals) = crate::todo::load_goals(session) {
            for goal in goals.iter().take(MAX_ITEMS) {
                if let Some(check) = goal.feedback_loop.as_deref() {
                    push(
                        &mut criteria,
                        format!(
                            "{}: {}; relevance {:?}; delivery {:?}",
                            clean(goal.group.as_deref().unwrap_or("goal")),
                            clean(check),
                            goal.feedback_loop_relevance,
                            goal.delivery_state
                        ),
                    );
                }
            }
        }
        input.acceptance_criteria = criteria.join("\n");
    }
}

fn record_diagnostics(
    capture: &mut TurnEvidence,
    seen: &mut VecDeque<u64>,
    path: &str,
    items: Option<&Value>,
) {
    for item in items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ITEMS)
    {
        let text = format!(
            "{}:{} severity {:?}: {}",
            path,
            item.pointer("/range/start/line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(1),
            item.get("severity").and_then(Value::as_u64),
            field(item, "message")
        );
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if !seen.contains(&hash) {
            push(&mut capture.diagnostics, text);
            seen.push_back(hash);
            if seen.len() > 128 {
                seen.pop_front();
            }
        }
    }
}

/// Explicit bounded source: tracked working-tree numstat against HEAD, not a
/// patch or raw file content. It can include earlier edits and omits untracked
/// files; per-tool revision evidence above identifies writes in this turn.
async fn bounded_diff(root: Option<&Path>) -> String {
    let Some(root) = root else {
        return "Working-tree diff unavailable: no session directory.".into();
    };
    let future = async {
        let mut child = tokio::process::Command::new("git")
            .args([
                "--no-pager",
                "-c",
                "core.fsmonitor=false",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "HEAD",
                "--",
            ])
            .current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()?
            .take(8193)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() > 8192 {
            child.kill().await.ok()?;
        } else if !child.wait().await.ok()?.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&bytes);
        Some(format!(
            "Tracked working-tree vs HEAD (may include prior edits; untracked files omitted):\n{}\n{}",
            text.lines()
                .take(MAX_ITEMS)
                .map(clean)
                .collect::<Vec<_>>()
                .join("\n"),
            if bytes.len() > 8192 || text.lines().count() > MAX_ITEMS {
                "[truncated]"
            } else {
                "[end of numstat]"
            }
        ))
    };
    tokio::time::timeout(std::time::Duration::from_millis(350), future)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            "Working-tree diff unavailable (not a git worktree, no HEAD, or bounded deadline)."
                .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_review_rejects_invented_evidence() {
        let input = AdvisorTurnInput {
            verification_status: "Declared check unit tests: failed, exit Some(1)".into(),
            ..AdvisorTurnInput::default()
        };
        let mut note = AdvisorNote {
            severity: AdvisorSeverity::Concern,
            summary: "Acceptance not met".into(),
            evidence: vec!["unit tests: passed".into()],
            recommended_action: "fix tests".into(),
            blocking: false,
        };
        assert!(!grounded(&input, &note));
        note.evidence = vec!["unit tests: failed, exit Some(1)".into()];
        assert!(grounded(&input, &note));
        note.evidence.clear();
        assert!(!grounded(&input, &note));
    }

    #[tokio::test]
    async fn advisor_evidence_is_scoped_redacted_and_does_not_infer_verification() {
        let manager = AdvisorManager::default();
        let config = AdvisorConfig {
            enabled: true,
            ..AdvisorConfig::default()
        };
        manager.begin_capture("evidence", &config);
        let input = serde_json::json!({"intent":"run checks OPENAI_API_KEY=sk-test-openai-example", "verification":true});
        let output = ToolOutput::new("RAW_PRIVATE_OUTPUT").with_metadata(serde_json::json!({
            "files":[{"path":"src/fix.rs","revision_before":{"revision":1},"revision_after":{"revision":2}}],
            "semantic_verification":{"files":[{"path":"src/fix.rs","status":"issues_found","diagnostics":[{"message":"wrong type", "severity":1}]}]}
        })).with_exit_code(Some(1));
        manager.capture_tool("evidence", "check", &input, &Ok(output.clone()));
        manager.capture_tool("evidence", "check", &input, &Ok(output));
        let mut captured = AdvisorTurnInput::default();
        manager.enrich_input("evidence", &mut captured, None).await;
        assert!(captured.diff_summary.contains("src/fix.rs"));
        assert_eq!(captured.diagnostics.matches("wrong type").count(), 1);
        assert!(
            captured
                .verification_status
                .contains("failed, exit Some(1)")
        );
        let encoded = serde_json::to_string(&captured).expect("serialize");
        assert!(!encoded.contains("RAW_PRIVATE_OUTPUT"));
        assert!(!encoded.contains("sk-test-openai-example"));
        manager.begin_capture("evidence", &config);
        let mut next = AdvisorTurnInput::default();
        manager.enrich_input("evidence", &mut next, None).await;
        assert!(next.tools.is_empty());
        assert!(next.verification_status.contains("No verification result"));
    }

    #[tokio::test]
    async fn bounded_diff_reads_actual_changes_and_reports_unavailable_sources() {
        let dir = tempfile::tempdir().expect("directory");
        assert!(bounded_diff(Some(dir.path())).await.contains("unavailable"));
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("git")
        };
        assert!(git(&["init", "-q"]).status.success());
        std::fs::write(dir.path().join("file"), "before\n").expect("fixture");
        assert!(git(&["add", "file"]).status.success());
        assert!(
            git(&[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "fixture"
            ])
            .status
            .success()
        );
        std::fs::write(dir.path().join("file"), "after\n").expect("change");
        assert!(bounded_diff(Some(dir.path())).await.contains("1\t1\tfile"));
    }
}
