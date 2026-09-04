use crate::message::ToolCall;

use super::{
    canonical_tool_name, tool_output_looks_failed, truncate_end_display,
    truncate_identifier_display, truncate_middle_display, truncate_path_display,
    truncate_path_with_suffix,
};

fn string<'a>(input: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn number(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = input.get(*key)?;
        value
            .as_i64()
            .map(|number| number.to_string())
            .or_else(|| value.as_u64().map(|number| number.to_string()))
            .or_else(|| value.as_str().map(str::to_string))
    })
}

pub(super) fn summarize(tool: &ToolCall, max_width: usize) -> String {
    let action = tool
        .input
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("dap");
    let detail = match action {
        "launch" => string(&tool.input, &["program", "executable", "command"])
            .map(|value| truncate_path_display(value, max_width.saturating_sub(7))),
        // Attach uses an opaque, brokered target. Never render a PID, even if a
        // legacy or malicious payload includes one.
        "attach" => string(&tool.input, &["program"])
            .map(|value| truncate_path_display(value, max_width.saturating_sub(7))),
        "set_breakpoint" => {
            let path = string(&tool.input, &["source"]);
            let line = number(&tool.input, &["line"]);
            match (path, line) {
                (Some(path), Some(line)) => Some(truncate_path_with_suffix(
                    path,
                    &format!(":{line}"),
                    max_width.saturating_sub(action.len() + 1),
                )),
                (Some(path), None) => Some(truncate_path_display(
                    path,
                    max_width.saturating_sub(action.len() + 1),
                )),
                _ => None,
            }
        }
        "remove_breakpoint" => {
            string(&tool.input, &["breakpoint"]).map(|value| truncate_identifier_display(value, 18))
        }
        "continue" | "pause" | "step_over" | "step_in" | "step_out" | "stack_trace" => {
            number(&tool.input, &["thread_id", "threadId"]).map(|value| format!("thread {value}"))
        }
        "scopes" => string(&tool.input, &["frame"])
            .map(|value| format!("frame {}", truncate_identifier_display(value, 18))),
        "variables" => string(&tool.input, &["variables"])
            .map(|value| format!("ref {}", truncate_identifier_display(value, 18))),
        "evaluate" => string(&tool.input, &["expression"]).map(|value| {
            format!(
                "‘{}’",
                truncate_end_display(value, max_width.saturating_sub(action.len() + 3))
            )
        }),
        "output" => {
            let cursor = string(&tool.input, &["cursor"])
                .map(|value| truncate_identifier_display(value, 14));
            let count = number(&tool.input, &["count"]);
            match (cursor, count) {
                (Some(cursor), Some(count)) => Some(format!("after {cursor}, {count}")),
                (Some(cursor), None) => Some(format!("after {cursor}")),
                (None, Some(count)) => Some(format!("{count} entries")),
                (None, None) => None,
            }
        }
        "terminate" => {
            string(&tool.input, &["session"]).map(|value| truncate_identifier_display(value, 18))
        }
        "threads" | "sessions" => None,
        _ => None,
    };
    truncate_end_display(
        &detail
            .map(|detail| format!("{action} {detail}"))
            .unwrap_or_else(|| action.to_string()),
        max_width,
    )
}

fn field<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn text(value: &serde_json::Value, max_width: usize) -> Option<String> {
    let text = match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => return None,
        serde_json::Value::Array(values) => format!("{} items", values.len()),
        serde_json::Value::Object(values) => format!("{} fields", values.len()),
    };
    Some(truncate_middle_display(text.trim(), max_width))
}

fn count(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    field(value, keys).and_then(|value| match value {
        serde_json::Value::Array(values) => Some(values.len()),
        serde_json::Value::Number(number) => number.as_u64().map(|value| value as usize),
        _ => None,
    })
}

/// Compact semantic details for successful DAP results. Returning a body for
/// every action prevents useful debugger state from collapsing into the generic
/// success row.
pub(crate) fn result_summary_lines(
    tool: &ToolCall,
    content: &str,
    max_width: usize,
) -> Option<Vec<String>> {
    if canonical_tool_name(&tool.name) != "dap" || tool_output_looks_failed(content) {
        return None;
    }
    let action = tool
        .input
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("dap");
    let parsed = serde_json::from_str::<serde_json::Value>(content.trim()).ok();
    let envelope = parsed.as_ref().filter(|value| {
        value.get("protocol").and_then(|value| value.as_str()) == Some("jcode.dap.v1")
            && value.get("action").and_then(|value| value.as_str()) == Some(action)
    });
    let root = envelope.and_then(|value| value.get("result"));
    let mut details = Vec::new();
    if let Some(value) = root {
        let push = |details: &mut Vec<String>, label: &str, keys: &[&str], budget: usize| {
            if let Some(text) = field(value, keys).and_then(|value| text(value, budget)) {
                details.push(format!("{label}: {text}"));
            }
        };
        match action {
            "launch" | "attach" => {
                push(&mut details, "Session", &["session"], 24);
                push(&mut details, "State", &["state", "status"], 18);
            }
            "set_breakpoint" | "remove_breakpoint" => {
                push(&mut details, "Breakpoint", &["breakpoint", "removed"], 20);
                push(&mut details, "Status", &["verified", "status", "state"], 16);
                push(
                    &mut details,
                    "Location",
                    &["location", "path", "source"],
                    42,
                );
            }
            "continue" | "pause" | "step_over" | "step_in" | "step_out" => {
                push(&mut details, "State", &["state", "status"], 18);
                push(&mut details, "Thread", &["thread_id", "threadId"], 16);
                push(
                    &mut details,
                    "Revision",
                    &["execution_revision", "executionRevision", "revision"],
                    16,
                );
            }
            "threads" => {
                if let Some(count) = count(value, &["threads", "count"]) {
                    details.push(format!("Threads: {count}"));
                }
            }
            "stack_trace" => {
                if let Some(count) = count(
                    value,
                    &[
                        "frames",
                        "stack_frames",
                        "stackFrames",
                        "total_frames",
                        "totalFrames",
                    ],
                ) {
                    details.push(format!("Frames: {count}"));
                }
                push(&mut details, "Thread", &["thread_id", "threadId"], 16);
            }
            "scopes" => {
                if let Some(count) = count(value, &["scopes", "count"]) {
                    details.push(format!("Scopes: {count}"));
                }
            }
            "variables" => {
                if let Some(count) = count(value, &["variables", "count"]) {
                    details.push(format!("Variables: {count}"));
                }
            }
            "evaluate" => {
                push(&mut details, "Result", &["result", "value"], 52);
                push(&mut details, "Type", &["type", "value_type"], 20);
                push(
                    &mut details,
                    "Outcome",
                    &["outcome", "status", "reason"],
                    20,
                );
            }
            "output" => {
                if let Some(count) = count(value, &["records", "count"]) {
                    details.push(format!("Output entries: {count}"));
                }
                push(&mut details, "Cursor", &["cursor"], 20);
                push(&mut details, "Retained", &["retained_events"], 16);
            }
            "sessions" => {
                let count = value
                    .as_array()
                    .map(Vec::len)
                    .or_else(|| count(value, &["sessions", "count"]));
                if let Some(count) = count {
                    details.push(format!("Sessions: {count}"));
                }
            }
            "terminate" => push(&mut details, "State", &["state", "status", "reason"], 20),
            _ => {}
        }
    }
    if details.is_empty() {
        if parsed.is_some() && envelope.is_none() {
            details.push("Result: unsupported DAP result envelope".to_string());
        } else {
            let fallback = content
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("completed");
            details.push(format!(
                "Result: {}",
                truncate_middle_display(fallback.trim(), 60)
            ));
        }
    }
    details.truncate(3);
    Some(
        details
            .into_iter()
            .map(|line| truncate_end_display(&line, max_width))
            .collect(),
    )
}
