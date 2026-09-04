from pathlib import Path

ROOT = Path.cwd()


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"expected text not found in {path}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1))


evidence = r'''//! Bounded evidence extraction for a completed primary turn.
//!
//! This module reads jcode-owned transcript and task-state contracts. It never
//! shells out, replays an unbounded tool result, or retains provider-private
//! context. The parent `AdvisorTurnInput::bounded` pass remains the final size
//! and redaction boundary before retention or provider dispatch.

use super::{AdvisorTurnInput, MAX_FIELD_BYTES};
use crate::message::ContentBlock;
use crate::session::Session;
use serde_json::Value;
use std::collections::HashMap;

const MAX_SOURCE_TOOL_CALLS: usize = 24;
const MAX_SOURCE_RESULT_BYTES: usize = 16 * 1024;
const MAX_SUMMARY_LINE_BYTES: usize = 320;
const MAX_SUMMARY_LINES: usize = 12;
const MAX_OPEN_TODOS: usize = 8;
const MAX_ACCEPTANCE_GOALS: usize = 8;

#[derive(Debug)]
struct SourceResult {
    content: String,
    is_error: bool,
}

pub(super) fn enrich_completed_turn(
    mut input: AdvisorTurnInput,
    session: &Session,
    start_message_index: usize,
) -> AdvisorTurnInput {
    let messages = session
        .messages
        .get(start_message_index..)
        .unwrap_or_default();
    let results = collect_results(messages);
    let calls = collect_calls(messages, &results);

    input.diff_summary = summarize_diff(&calls);
    input.diagnostics = summarize_diagnostics(&calls);
    input.verification_status = summarize_verification(&calls, &input.verification_status);
    input.outstanding_todos = match crate::todo::load_todos(&session.id) {
        Ok(todos) => summarize_todos(&todos),
        Err(_) => "todo state unavailable".to_string(),
    };
    input.acceptance_criteria = match (
        crate::todo::load_plan(&session.id),
        crate::todo::load_goals(&session.id),
    ) {
        (Ok(plan), Ok(goals)) => summarize_acceptance(&plan, &goals),
        _ => "acceptance state unavailable".to_string(),
    };
    input
}

fn collect_results(messages: &[crate::session::StoredMessage]) -> HashMap<String, SourceResult> {
    let mut results = HashMap::new();
    for message in messages {
        for block in &message.content {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            {
                results.insert(
                    tool_use_id.clone(),
                    SourceResult {
                        content: truncate_owned(content, MAX_SOURCE_RESULT_BYTES),
                        is_error: is_error.unwrap_or(false),
                    },
                );
            }
        }
    }
    results
}

fn collect_calls<'a>(
    messages: &'a [crate::session::StoredMessage],
    results: &'a HashMap<String, SourceResult>,
) -> Vec<(&'a str, &'a Value, Option<&'a SourceResult>)> {
    let mut calls = Vec::new();
    for message in messages {
        for block in &message.content {
            if let ContentBlock::ToolUse {
                id, name, input, ..
            } = block
            {
                calls.push((name.as_str(), input, results.get(id)));
                if calls.len() >= MAX_SOURCE_TOOL_CALLS {
                    return calls;
                }
            }
        }
    }
    calls
}

fn summarize_diff(calls: &[(&str, &Value, Option<&SourceResult>)]) -> String {
    let mut lines = Vec::new();
    for (name, input, _) in calls {
        if !is_workspace_change(name, input) {
            continue;
        }
        let mut paths = Vec::new();
        collect_named_strings(
            input,
            &["path", "file_path", "target_path", "old_path", "new_path"],
            &mut paths,
            4,
        );
        paths.sort();
        paths.dedup();
        let target = if paths.is_empty() {
            "target not declared".to_string()
        } else {
            paths.join(", ")
        };
        let (added, removed) = diff_line_counts(input);
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or(name);
        let counts = if added == 0 && removed == 0 {
            String::new()
        } else {
            format!(" (+{added}/-{removed})")
        };
        push_summary_line(
            &mut lines,
            format!("{name}:{action} {target}{counts}"),
            MAX_SUMMARY_LINES,
        );
    }
    if lines.is_empty() {
        "no workspace-changing tool call observed in this turn".to_string()
    } else {
        lines.join("\n")
    }
}

fn summarize_diagnostics(calls: &[(&str, &Value, Option<&SourceResult>)]) -> String {
    let mut lines = Vec::new();
    for (name, input, result) in calls {
        let Some(result) = result else {
            continue;
        };
        let diagnostic_tool = matches!(*name, "lsp" | "dap")
            || input
                .get("action")
                .and_then(Value::as_str)
                .is_some_and(|action| {
                    let action = action.to_ascii_lowercase();
                    action.contains("diagnostic") || action.contains("error")
                });
        if !diagnostic_tool && !result.is_error && !contains_diagnostic_marker(&result.content) {
            continue;
        }

        let excerpts = diagnostic_excerpts(&result.content);
        if excerpts.is_empty() {
            push_summary_line(
                &mut lines,
                format!(
                    "{name}: {} (no concise diagnostic text)",
                    if result.is_error { "error" } else { "diagnostic" }
                ),
                MAX_SUMMARY_LINES,
            );
        } else {
            for excerpt in excerpts.into_iter().take(3) {
                push_summary_line(
                    &mut lines,
                    format!(
                        "{name}: {}: {excerpt}",
                        if result.is_error { "error" } else { "diagnostic" }
                    ),
                    MAX_SUMMARY_LINES,
                );
            }
        }
    }
    if lines.is_empty() {
        "no new diagnostics observed in this turn".to_string()
    } else {
        lines.join("\n")
    }
}

fn summarize_verification(
    calls: &[(&str, &Value, Option<&SourceResult>)],
    primary_status: &str,
) -> String {
    let mut lines = Vec::new();
    push_summary_line(
        &mut lines,
        format!("primary turn: {primary_status}"),
        MAX_SUMMARY_LINES,
    );
    for (name, input, result) in calls {
        let Some(label) = verification_label(name, input) else {
            continue;
        };
        let (outcome, excerpt) = match result {
            None => ("UNKNOWN", "no stored result".to_string()),
            Some(result) => {
                let failed = result.is_error || result_text_indicates_failure(&result.content);
                let excerpt = concise_excerpt(&result.content, 1)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "empty result".to_string());
                (if failed { "FAIL" } else { "PASS" }, excerpt)
            }
        };
        push_summary_line(
            &mut lines,
            format!("{outcome} {name}: {label} — {excerpt}"),
            MAX_SUMMARY_LINES,
        );
    }
    if lines.len() == 1 {
        push_summary_line(
            &mut lines,
            "no explicit verification command observed".to_string(),
            MAX_SUMMARY_LINES,
        );
    }
    lines.join("\n")
}

fn summarize_todos(todos: &[crate::todo::TodoItem]) -> String {
    if todos.is_empty() {
        return "no persisted todos".to_string();
    }
    let completed = todos
        .iter()
        .filter(|todo| todo.status.eq_ignore_ascii_case("completed"))
        .count();
    let open: Vec<_> = todos
        .iter()
        .filter(|todo| !todo.status.eq_ignore_ascii_case("completed"))
        .collect();
    let mut lines = vec![format!(
        "{} total; {} completed; {} outstanding",
        todos.len(),
        completed,
        open.len()
    )];
    for todo in open.into_iter().take(MAX_OPEN_TODOS) {
        let group = todo
            .group
            .as_deref()
            .map(|group| format!(" group={}", collapse_whitespace(group)))
            .unwrap_or_default();
        push_summary_line(
            &mut lines,
            format!(
                "{} [{}] {}{}",
                collapse_whitespace(&todo.id),
                collapse_whitespace(&todo.status),
                collapse_whitespace(&todo.content),
                group
            ),
            MAX_SUMMARY_LINES,
        );
    }
    lines.join("\n")
}

fn summarize_acceptance(
    plan: &crate::todo::TodoPlan,
    goals: &[crate::todo::TodoGoal],
) -> String {
    let mut lines = Vec::new();
    if let Some(intention) = plan
        .user_intention
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_summary_line(
            &mut lines,
            format!("user outcome: {}", collapse_whitespace(intention)),
            MAX_SUMMARY_LINES,
        );
    }
    if let Some(understanding) = plan.understands_user_intent {
        push_summary_line(
            &mut lines,
            format!("intent understanding: {understanding:?}"),
            MAX_SUMMARY_LINES,
        );
    }
    for goal in goals.iter().take(MAX_ACCEPTANCE_GOALS) {
        let label = goal.group.as_deref().unwrap_or("ungrouped");
        let loop_text = goal
            .feedback_loop
            .as_deref()
            .map(collapse_whitespace)
            .unwrap_or_else(|| "not recorded".to_string());
        push_summary_line(
            &mut lines,
            format!(
                "goal {}: check={}; relevance={:?}; coverage={:?}; traceability={:?}; delivery={:?}",
                collapse_whitespace(label),
                loop_text,
                goal.feedback_loop_relevance,
                goal.feedback_loop_coverage,
                goal.feedback_loop_traceability,
                goal.delivery_state,
            ),
            MAX_SUMMARY_LINES,
        );
    }
    if lines.is_empty() {
        "no persisted acceptance criteria; assess only the stated objective".to_string()
    } else {
        lines.join("\n")
    }
}

fn is_workspace_change(name: &str, input: &Value) -> bool {
    match name {
        "write" | "edit" | "multiedit" | "patch" | "apply_patch" | "anchored_edit" => true,
        "lsp" => input.get("apply").and_then(Value::as_bool) == Some(true),
        _ => false,
    }
}

fn collect_named_strings(
    value: &Value,
    keys: &[&str],
    output: &mut Vec<String>,
    limit: usize,
) {
    if output.len() >= limit {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if keys.contains(&key.as_str()) {
                    collect_scalar_strings(value, output, limit);
                }
                if output.len() < limit {
                    collect_named_strings(value, keys, output, limit);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_named_strings(value, keys, output, limit);
                if output.len() >= limit {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn collect_scalar_strings(value: &Value, output: &mut Vec<String>, limit: usize) {
    match value {
        Value::String(value) => {
            if output.len() < limit {
                output.push(truncate_owned(&collapse_whitespace(value), 240));
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_scalar_strings(value, output, limit);
                if output.len() >= limit {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn diff_line_counts(input: &Value) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    visit_named_strings(input, &["patch", "diff"], &mut |value| {
        for line in value.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                added = added.saturating_add(1);
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed = removed.saturating_add(1);
            }
        }
    });
    (added, removed)
}

fn visit_named_strings(value: &Value, keys: &[&str], visitor: &mut impl FnMut(&str)) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if keys.contains(&key.as_str()) {
                    match value {
                        Value::String(value) => visitor(value),
                        Value::Array(values) => {
                            for value in values {
                                if let Some(value) = value.as_str() {
                                    visitor(value);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                visit_named_strings(value, keys, visitor);
            }
        }
        Value::Array(values) => {
            for value in values {
                visit_named_strings(value, keys, visitor);
            }
        }
        _ => {}
    }
}

fn verification_label(name: &str, input: &Value) -> Option<String> {
    match name {
        "bash" => {
            let command = input
                .get("command")
                .or_else(|| input.get("cmd"))
                .and_then(Value::as_str)?;
            if is_verification_command(command) {
                Some(truncate_owned(&collapse_whitespace(command), 240))
            } else {
                None
            }
        }
        "lsp" => {
            let action = input.get("action").and_then(Value::as_str).unwrap_or("diagnostics");
            if input.get("apply").and_then(Value::as_bool) == Some(true) {
                None
            } else {
                Some(format!("language-server {action}"))
            }
        }
        "selfdev" => {
            let action = input.get("action").and_then(Value::as_str)?;
            let action_lower = action.to_ascii_lowercase();
            if action_lower.contains("status")
                || action_lower.contains("check")
                || action_lower.contains("test")
                || action_lower.contains("verify")
            {
                Some(format!("selfdev {action}"))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_verification_command(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    [
        "cargo test",
        "cargo check",
        "cargo clippy",
        "cargo fmt",
        "pytest",
        "python -m unittest",
        "python3 -m unittest",
        "go test",
        "npm test",
        "npm run test",
        "pnpm test",
        "yarn test",
        "bun test",
        "ctest",
        "mvn test",
        "gradle test",
        "ruff check",
        "mypy ",
        "tsc ",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn contains_diagnostic_marker(content: &str) -> bool {
    let content = content.to_ascii_lowercase();
    ["error:", "warning:", "diagnostic", "severity", "failed"]
        .iter()
        .any(|needle| content.contains(needle))
}

fn diagnostic_excerpts(content: &str) -> Vec<String> {
    let marked: Vec<String> = content
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            ["error", "warning", "diagnostic", "severity", "failed"]
                .iter()
                .any(|needle| line.contains(needle))
        })
        .take(3)
        .map(collapse_whitespace)
        .filter(|line| !line.is_empty())
        .map(|line| truncate_owned(&line, MAX_SUMMARY_LINE_BYTES))
        .collect();
    if marked.is_empty() {
        concise_excerpt(content, 1)
    } else {
        marked
    }
}

fn result_text_indicates_failure(content: &str) -> bool {
    let content = content.to_ascii_lowercase();
    [
        "test result: failed",
        "tests failed",
        "command failed",
        "exit code 1",
        "exited with status 1",
        "fatal error",
        "panicked at",
    ]
    .iter()
    .any(|needle| content.contains(needle))
}

fn concise_excerpt(content: &str, line_limit: usize) -> Vec<String> {
    content
        .lines()
        .map(collapse_whitespace)
        .filter(|line| !line.is_empty())
        .take(line_limit)
        .map(|line| truncate_owned(&line, MAX_SUMMARY_LINE_BYTES))
        .collect()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_summary_line(lines: &mut Vec<String>, value: String, line_limit: usize) {
    if lines.len() >= line_limit {
        return;
    }
    let remaining = MAX_FIELD_BYTES.saturating_sub(lines.iter().map(|line| line.len() + 1).sum());
    if remaining == 0 {
        return;
    }
    lines.push(truncate_owned(
        &collapse_whitespace(&value),
        remaining.min(MAX_SUMMARY_LINE_BYTES),
    ));
}

fn truncate_owned(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Role;

    fn message(content: Vec<ContentBlock>) -> crate::session::StoredMessage {
        crate::session::StoredMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Role::Assistant,
            content,
            display_role: None,
            timestamp: None,
            tool_duration_ms: None,
            token_usage: None,
        }
    }

    #[test]
    fn extracts_bounded_diff_diagnostics_and_verification_without_raw_replay() {
        let mut session = Session::create(None, None);
        let start = session.messages.len();
        session.messages.push(message(vec![
            ContentBlock::ToolUse {
                id: "patch-1".to_string(),
                name: "apply_patch".to_string(),
                input: serde_json::json!({
                    "path": "src/lib.rs",
                    "patch": "--- a/src/lib.rs\n+++ b/src/lib.rs\n-old\n+new\n+extra"
                }),
                thought_signature: None,
            },
            ContentBlock::ToolUse {
                id: "test-1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "cargo test -p demo"}),
                thought_signature: None,
            },
            ContentBlock::ToolUse {
                id: "lsp-1".to_string(),
                name: "lsp".to_string(),
                input: serde_json::json!({"action": "diagnostics"}),
                thought_signature: None,
            },
        ]));
        session.messages.push(message(vec![
            ContentBlock::ToolResult {
                tool_use_id: "patch-1".to_string(),
                content: "Done".to_string(),
                is_error: Some(false),
            },
            ContentBlock::ToolResult {
                tool_use_id: "test-1".to_string(),
                content: "test result: ok. 12 passed; 0 failed".to_string(),
                is_error: Some(false),
            },
            ContentBlock::ToolResult {
                tool_use_id: "lsp-1".to_string(),
                content: format!("warning: unused item\n{}", "raw payload ".repeat(10_000)),
                is_error: Some(false),
            },
        ]));

        let input = enrich_completed_turn(
            AdvisorTurnInput {
                verification_status: "turn completed".to_string(),
                ..AdvisorTurnInput::default()
            },
            &session,
            start,
        );

        assert!(input.diff_summary.contains("src/lib.rs"));
        assert!(input.diff_summary.contains("+2/-1"));
        assert!(input.verification_status.contains("PASS bash"));
        assert!(input.verification_status.contains("cargo test -p demo"));
        assert!(input.diagnostics.contains("warning: unused item"));
        assert!(input.diagnostics.len() <= MAX_FIELD_BYTES);
        assert!(!input.diagnostics.contains("raw payload raw payload raw payload"));
    }

    #[test]
    fn summarizes_persisted_todo_and_acceptance_contracts() {
        let todos = vec![
            crate::todo::TodoItem {
                id: "done".to_string(),
                content: "finished".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
                ..Default::default()
            },
            crate::todo::TodoItem {
                id: "open".to_string(),
                content: "run socket acceptance".to_string(),
                status: "in_progress".to_string(),
                priority: "high".to_string(),
                group: Some("acceptance".to_string()),
                ..Default::default()
            },
        ];
        let plan = crate::todo::TodoPlan {
            user_intention: Some("ship verified behavior".to_string()),
            ..Default::default()
        };
        let goals = vec![crate::todo::TodoGoal {
            group: Some("acceptance".to_string()),
            feedback_loop: Some("cargo test and socket smoke".to_string()),
            ..Default::default()
        }];

        let todo_summary = summarize_todos(&todos);
        let acceptance = summarize_acceptance(&plan, &goals);
        assert!(todo_summary.contains("1 outstanding"));
        assert!(todo_summary.contains("run socket acceptance"));
        assert!(acceptance.contains("ship verified behavior"));
        assert!(acceptance.contains("cargo test and socket smoke"));
    }
}
'''

(ROOT / "crates/jcode-app-core/src/advisor").mkdir(parents=True, exist_ok=True)
(ROOT / "crates/jcode-app-core/src/advisor/evidence.rs").write_text(evidence)

replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "//! delivery. Risky-tool gating and user controls remain separate follow-ups.\n\nuse crate::config",
    "//! delivery. Risky-tool gating and user controls remain separate follow-ups.\n\nmod evidence;\n\nuse crate::config",
)

replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    fn bounded(mut self, redact: bool) -> Self {",
    "    pub(crate) fn enrich_from_session(\n        self,\n        session: &crate::session::Session,\n        start_message_index: usize,\n    ) -> Self {\n        evidence::enrich_completed_turn(self, session, start_message_index)\n    }\n\n    fn bounded(mut self, redact: bool) -> Self {",
)

replace_once(
    "crates/jcode-app-core/src/agent/turn_execution.rs",
    "            turn_succeeded,\n        );\n        let _ = manager.schedule_turn(",
    "            turn_succeeded,\n        )\n        .enrich_from_session(&self.session, start_message_index);\n        let _ = manager.schedule_turn(",
)
