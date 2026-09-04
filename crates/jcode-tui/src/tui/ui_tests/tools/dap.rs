use super::*;

fn call(action: &str, input: serde_json::Value) -> ToolCall {
    let mut input = input.as_object().cloned().unwrap_or_default();
    input.insert("action".into(), action.into());
    ToolCall {
        id: format!("dap-{action}"),
        name: "dap".into(),
        input: input.into(),
        intent: None,
        thought_signature: None,
    }
}

#[test]
fn all_mvp_actions_have_bounded_input_summaries() {
    let cases = [
        (
            "launch",
            serde_json::json!({"program":"/very/long/workspace/target/debug/example-binary"}),
        ),
        (
            "attach",
            serde_json::json!({"program":"/very/long/workspace/target/debug/example-binary","pid":4242}),
        ),
        (
            "set_breakpoint",
            serde_json::json!({"source":"/very/long/workspace/src/important_module.rs","line":123}),
        ),
        (
            "remove_breakpoint",
            serde_json::json!({"breakpoint":"breakpoint-handle-that-is-long"}),
        ),
        ("continue", serde_json::json!({"thread_id":1})),
        ("pause", serde_json::json!({"thread_id":1})),
        ("step_over", serde_json::json!({"thread_id":1})),
        ("step_in", serde_json::json!({"thread_id":1})),
        ("step_out", serde_json::json!({"thread_id":1})),
        ("threads", serde_json::json!({})),
        ("stack_trace", serde_json::json!({"thread_id":1})),
        (
            "scopes",
            serde_json::json!({"frame":"opaque-frame-handle-that-is-long"}),
        ),
        (
            "variables",
            serde_json::json!({"variables":"opaque-variable-handle-that-is-long"}),
        ),
        (
            "evaluate",
            serde_json::json!({"expression":"some.deeply.nested.value + another.deeply.nested.value"}),
        ),
        (
            "output",
            serde_json::json!({"cursor":"opaque-cursor-that-is-long","count":20}),
        ),
        ("sessions", serde_json::json!({})),
        (
            "terminate",
            serde_json::json!({"session":"opaque-session-handle-that-is-long"}),
        ),
    ];
    for (action, input) in cases {
        let summary = tools_ui::get_tool_summary_with_budget(&call(action, input), 50, Some(32));
        assert!(!summary.is_empty(), "missing summary for {action}");
        assert!(
            unicode_width::UnicodeWidthStr::width(summary.as_str()) <= 32,
            "unbounded summary for {action}: {summary}"
        );
        if action == "attach" {
            assert!(!summary.to_ascii_lowercase().contains("pid"));
            assert!(!summary.contains("4242"));
        }
    }
}

#[test]
fn all_mvp_actions_render_labeled_versioned_results() {
    let cases = [
        (
            "launch",
            serde_json::json!({"session":"s-1","state":"stopped"}),
            "Session:",
        ),
        (
            "attach",
            serde_json::json!({"session":"s-2","state":"stopped"}),
            "Session:",
        ),
        (
            "set_breakpoint",
            serde_json::json!({"breakpoint":"bp-1","verified":true}),
            "Breakpoint:",
        ),
        (
            "remove_breakpoint",
            serde_json::json!({"removed":"bp-1"}),
            "Breakpoint:",
        ),
        ("continue", serde_json::json!({"state":"running"}), "State:"),
        ("pause", serde_json::json!({"state":"stopped"}), "State:"),
        (
            "step_over",
            serde_json::json!({"state":"stopped"}),
            "State:",
        ),
        ("step_in", serde_json::json!({"state":"stopped"}), "State:"),
        ("step_out", serde_json::json!({"state":"stopped"}), "State:"),
        (
            "threads",
            serde_json::json!({"threads":[{},{}]}),
            "Threads:",
        ),
        (
            "stack_trace",
            serde_json::json!({"frames":[{},{}]}),
            "Frames:",
        ),
        ("scopes", serde_json::json!({"scopes":[{}]}), "Scopes:"),
        (
            "variables",
            serde_json::json!({"variables":[{},{}]}),
            "Variables:",
        ),
        (
            "evaluate",
            serde_json::json!({"result":"a very long debugger value","type":"String"}),
            "Result:",
        ),
        (
            "output",
            serde_json::json!({"records":[{},{}],"cursor":"c-2"}),
            "Output entries:",
        ),
        ("sessions", serde_json::json!([{}, {}]), "Sessions:"),
        (
            "terminate",
            serde_json::json!({"state":"terminated"}),
            "State:",
        ),
    ];
    for (action, result, label) in cases {
        let content =
            serde_json::json!({"protocol":"jcode.dap.v1","action":action,"result":result})
                .to_string();
        let lines =
            tools_ui::dap_result_summary_lines(&call(action, serde_json::json!({})), &content, 36)
                .unwrap();
        assert!(
            lines.iter().any(|line| line.starts_with(label)),
            "missing label for {action}: {lines:?}"
        );
        assert!(lines.len() <= 3);
        assert!(
            lines
                .iter()
                .all(|line| unicode_width::UnicodeWidthStr::width(line.as_str()) <= 36)
        );
    }
}

#[test]
fn mismatched_versioned_result_is_identified_not_rendered_as_raw_json() {
    let content =
        serde_json::json!({"protocol":"jcode.dap.v1","action":"threads","result":{"threads":[]}})
            .to_string();
    let lines =
        tools_ui::dap_result_summary_lines(&call("sessions", serde_json::json!({})), &content, 80)
            .unwrap();
    assert_eq!(lines, vec!["Result: unsupported DAP result envelope"]);
}
