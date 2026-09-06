use super::*;

fn user_text(text: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }
}

fn tool_result(id: &str, content: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: content.to_string(),
            is_error: None,
        }],
        timestamp: None,
        tool_duration_ms: Some(1),
    }
}

#[test]
fn messages_end_with_tool_result_detects_tool_continuation_context() {
    let messages = vec![
        user_text("tell me about the desktop application"),
        tool_result("functions.read:0", "desktop architecture docs"),
        tool_result("functions.agentgrep:4", "desktop source summary"),
    ];

    assert!(Agent::messages_end_with_tool_result(&messages));
}

#[test]
fn messages_end_with_tool_result_allows_memory_after_tool_results() {
    let messages = vec![
        user_text("tell me about the desktop application"),
        tool_result("functions.read:0", "desktop architecture docs"),
        user_text("<system-reminder>Relevant memory</system-reminder>"),
    ];

    assert!(Agent::messages_end_with_tool_result(&messages));
}

#[test]
fn messages_end_with_tool_result_ignores_plain_user_prompt() {
    let messages = vec![user_text("hello")];

    assert!(!Agent::messages_end_with_tool_result(&messages));
}

#[test]
fn sequential_tool_rounds_trigger_after_three_single_calls() {
    let mut rounds = 0;
    for _ in 0..3 {
        rounds = Agent::update_sequential_tool_rounds(rounds, 1, false);
    }

    assert_eq!(rounds, Agent::SEQUENTIAL_TOOL_ROUNDS_BEFORE_BATCH_NUDGE);
}

#[test]
fn parallel_or_batch_calls_reset_sequential_tool_rounds() {
    assert_eq!(Agent::update_sequential_tool_rounds(2, 2, false), 0);
    assert_eq!(Agent::update_sequential_tool_rounds(2, 1, true), 0);
    assert_eq!(Agent::update_sequential_tool_rounds(2, 0, false), 0);
}

#[test]
fn pending_nudge_is_injected_only_when_batch_is_available() {
    assert!(Agent::should_inject_batch_nudge(true, true));
    assert!(!Agent::should_inject_batch_nudge(false, true));
    assert!(!Agent::should_inject_batch_nudge(true, false));
    assert!(Agent::BATCH_NUDGE.contains("use the batch tool"));
    assert!(Agent::BATCH_NUDGE.contains("result is required"));
}
