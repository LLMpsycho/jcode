use super::*;

#[path = "tools/dap.rs"]
mod dap;

#[test]
fn test_summarize_apply_patch_input_ignores_begin_marker() {
    let patch = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n";
    let summary = tools_ui::summarize_apply_patch_input(patch);
    assert_eq!(summary, "src/lib.rs (6 lines)");
}

#[test]
fn test_summarize_apply_patch_input_multiple_files() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-a\n+b\n*** Update File: b.txt\n@@\n-c\n+d\n*** End Patch\n";
    let summary = tools_ui::summarize_apply_patch_input(patch);
    assert_eq!(summary, "2 files (10 lines)");
}

#[test]
fn test_extract_apply_patch_primary_file() {
    let patch = "*** Begin Patch\n*** Add File: new/file.rs\n+fn main() {}\n*** End Patch\n";
    let file = tools_ui::extract_apply_patch_primary_file(patch);
    assert_eq!(file.as_deref(), Some("new/file.rs"));
}

#[test]
fn test_tool_summary_gmail_actions() {
    let search = ToolCall {
        id: "call_gmail_search".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({
            "action": "search",
            "query": "from:alice subject:invoice",
            "max_results": 5
        }),
        intent: None,
        thought_signature: None,
    };
    let summary = tools_ui::get_tool_summary_with_budget(&search, 50, Some(50));
    assert!(summary.starts_with("search "), "summary={summary:?}");
    assert!(summary.contains("from:alice"), "summary={summary:?}");

    let read = ToolCall {
        id: "call_gmail_read".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({
            "action": "read",
            "message_id": "18f2ab34cd56ef78"
        }),
        intent: None,
        thought_signature: None,
    };
    let summary = tools_ui::get_tool_summary_with_budget(&read, 50, Some(50));
    assert!(summary.starts_with("read "), "summary={summary:?}");

    let send = ToolCall {
        id: "call_gmail_send".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({
            "action": "send",
            "to": "bob@example.com",
            "subject": "hello"
        }),
        intent: None,
        thought_signature: None,
    };
    let summary = tools_ui::get_tool_summary_with_budget(&send, 50, Some(50));
    assert!(
        summary.contains("send") && summary.contains("bob@example.com"),
        "summary={summary:?}"
    );

    let bare = ToolCall {
        id: "call_gmail_labels".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({ "action": "labels" }),
        intent: None,
        thought_signature: None,
    };
    let summary = tools_ui::get_tool_summary_with_budget(&bare, 50, Some(50));
    assert_eq!(summary, "labels");
}

#[test]
fn test_tool_activity_detail_prefixes_intent_for_gmail_and_browser() {
    tools_ui::tests_tool_call_details_override::set(true);
    let gmail = ToolCall {
        id: "call_gmail_intent".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({
            "action": "search",
            "query": "is:unread",
            "intent": "Check unread mail"
        }),
        intent: Some("Check unread mail".to_string()),
        thought_signature: None,
    };
    let detail = tools_ui::get_tool_activity_detail(&gmail);
    assert!(detail.starts_with("Check unread mail"), "detail={detail:?}");
    assert!(detail.contains("is:unread"), "detail={detail:?}");

    let browser = ToolCall {
        id: "call_browser_intent".to_string(),
        name: "browser".to_string(),
        input: serde_json::json!({
            "action": "open",
            "url": "https://example.com",
            "intent": "Open docs page"
        }),
        intent: Some("Open docs page".to_string()),
        thought_signature: None,
    };
    let detail = tools_ui::get_tool_activity_detail(&browser);
    assert!(detail.starts_with("Open docs page"), "detail={detail:?}");
    assert!(detail.contains("example.com"), "detail={detail:?}");
    tools_ui::tests_tool_call_details_override::set(false);
}

/// By default (tool_call_details off) the activity detail is the intent alone.
#[test]
fn test_tool_activity_detail_hides_technical_summary_by_default() {
    let gmail = ToolCall {
        id: "call_gmail_intent_only".to_string(),
        name: "gmail".to_string(),
        input: serde_json::json!({
            "action": "search",
            "query": "is:unread",
            "intent": "Check unread mail"
        }),
        intent: Some("Check unread mail".to_string()),
        thought_signature: None,
    };
    let detail = tools_ui::get_tool_activity_detail(&gmail);
    assert_eq!(detail, "Check unread mail");
}

#[test]
fn test_tool_summary_covers_action_shaped_tools_and_fallback() {
    let cases: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "schedule",
            serde_json::json!({ "action": "create", "task": "check CI status" }),
            "create",
        ),
        (
            "schedule",
            serde_json::json!({ "action": "cancel", "schedule_id": "sched_123" }),
            "cancel",
        ),
        (
            "skill_manage",
            serde_json::json!({ "action": "load", "name": "frontend-design" }),
            "load /frontend-design",
        ),
        (
            "invalid",
            serde_json::json!({ "tool": "bash", "error": "missing command" }),
            "bash: missing command",
        ),
        (
            "integration_tools",
            serde_json::json!({ "category": "databases", "reason": "need a db" }),
            "search databases",
        ),
        (
            "integration_tools",
            serde_json::json!({
                "action": "suggest",
                "category": "payments",
                "suggestion_kind": "known_product",
                "product_name": "Stripe sandbox MCP"
            }),
            "suggest Stripe sandbox MCP",
        ),
        // Unknown/unmatched tools fall back to the action field.
        (
            "request_permission",
            serde_json::json!({ "action": "push", "description": "push commits" }),
            "push",
        ),
    ];
    for (name, input, expected_prefix) in cases {
        let tool = ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            input,
            intent: None,
            thought_signature: None,
        };
        let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(60));
        assert!(
            summary.starts_with(expected_prefix),
            "tool={name} summary={summary:?} expected prefix {expected_prefix:?}"
        );
    }
}

#[test]
fn test_tool_summary_read_supports_start_line_end_line() {
    let tool = ToolCall {
        id: "call_read_range".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({
            "file_path": "src/tool/read.rs",
            "start_line": 10,
            "end_line": 20
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(40));
    assert!(summary.contains("read.rs:10-20"), "summary={summary:?}");
}

#[test]
fn test_render_tool_message_batch_includes_start_end_read_details() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] read ---\nok\n\nCompleted: 1 succeeded, 0 failed".to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: Some(ToolCall {
            id: "call_batch_range".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [
                    {"tool": "read", "file_path": "src/tool/read.rs", "start_line": 10, "end_line": 20}
                ]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines.iter().map(extract_line_text).collect();

    assert_eq!(rendered.len(), 2, "rendered={rendered:?}");
    assert!(
        rendered[0].contains("✓ batch 1 call"),
        "rendered={rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("✓ read src/tool/read.rs:10-20")),
        "missing read subtool in {rendered:?}"
    );
}

#[test]
fn test_tool_summary_path_truncation_keeps_filename_tail() {
    let tool = ToolCall {
        id: "call_read_tail".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({
            "file_path": "src/tui/really/long/nested/location/ui_messages.rs",
            "offset": 120,
            "limit": 40
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(28));

    assert!(summary.contains("ui_messages.rs"), "summary={summary:?}");
    assert!(summary.contains(":120-160"), "summary={summary:?}");
    assert!(summary.contains('…'), "summary={summary:?}");
    assert!(unicode_width::UnicodeWidthStr::width(summary.as_str()) <= 28);
}

#[test]
fn test_tool_summary_grep_truncation_prefers_middle() {
    let tool = ToolCall {
        id: "call_grep_middle".to_string(),
        name: "grep".to_string(),
        input: serde_json::json!({
            "pattern": "prefix_[A-Z0-9]+_important_middle_token_[a-z]+_suffix",
            "path": "src/some/really/long/module"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(34));

    assert!(
        summary.contains("importan") || summary.contains("token"),
        "summary={summary:?}"
    );
    assert!(
        summary.contains("suffix") || summary.contains("module"),
        "summary={summary:?}"
    );
    assert!(summary.contains('…'), "summary={summary:?}");
    assert!(unicode_width::UnicodeWidthStr::width(summary.as_str()) <= 34);
}

#[test]
fn test_tool_summary_bash_truncation_keeps_start_and_end() {
    let tool = ToolCall {
        id: "call_bash_middle".to_string(),
        name: "bash".to_string(),
        input: serde_json::json!({
            "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_flat_subcall_params_include_read_details -- --nocapture"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 32, Some(34));

    assert!(summary.starts_with("$ cargo"), "summary={summary:?}");
    assert!(
        summary.contains("nocapture") || summary.contains("read_details"),
        "summary={summary:?}"
    );
    assert!(summary.contains('…'), "summary={summary:?}");
    assert!(unicode_width::UnicodeWidthStr::width(summary.as_str()) <= 34);
}

#[test]
fn test_tool_summary_bash_keeps_full_command_when_width_fits() {
    let tool = ToolCall {
        id: "call_bash_full".to_string(),
        name: "bash".to_string(),
        input: serde_json::json!({
            "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 32, Some(160));

    assert_eq!(
        summary,
        "$ cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture"
    );
    assert!(!summary.contains('…'), "summary={summary:?}");
}

#[test]
fn test_render_batch_subcall_line_keeps_full_bash_summary_when_row_fits() {
    let tool = ToolCall {
        id: "batch-1-bash".to_string(),
        name: "bash".to_string(),
        input: serde_json::json!({
            "command": "cargo test --package jcode --lib tui::ui::tests::render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width -- --nocapture"
        }),
        intent: None,
        thought_signature: None,
    };

    let line =
        tools_ui::render_batch_subcall_line(&tool, "✓", rgb(100, 180, 100), 32, Some(160), None);
    let rendered = extract_line_text(&line);

    assert!(
        rendered.contains("bash $ cargo test --package jcode"),
        "rendered={rendered:?}"
    );
    assert!(rendered.contains("-- --nocapture"), "rendered={rendered:?}");
    assert!(!rendered.contains('…'), "rendered={rendered:?}");
}

#[test]
fn test_render_batch_subcall_line_shows_model_provided_intent() {
    tools_ui::tests_tool_call_details_override::set(true);
    let tool = ToolCall {
        id: "batch-1-read".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({"file_path": "src/tui/ui_messages.rs"}),
        intent: Some("Inspect completed batch rendering".to_string()),
        thought_signature: None,
    };

    let line =
        tools_ui::render_batch_subcall_line(&tool, "✓", rgb(100, 180, 100), 50, Some(120), None);
    let rendered = extract_line_text(&line);

    assert!(
        rendered.contains("read · Inspect completed batch rendering ·"),
        "rendered={rendered:?}"
    );
    assert!(rendered.contains("ui_messages.rs"), "rendered={rendered:?}");
    tools_ui::tests_tool_call_details_override::set(false);
}

/// By default (tool_call_details off) a subcall row with an intent shows only
/// the intent, not the dimmed technical summary.
#[test]
fn test_render_batch_subcall_line_hides_technical_detail_by_default() {
    let tool = ToolCall {
        id: "batch-1-read".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({"file_path": "src/tui/ui_messages.rs"}),
        intent: Some("Inspect completed batch rendering".to_string()),
        thought_signature: None,
    };

    let line =
        tools_ui::render_batch_subcall_line(&tool, "✓", rgb(100, 180, 100), 50, Some(120), None);
    let rendered = extract_line_text(&line);

    assert!(
        rendered.contains("read · Inspect completed batch rendering"),
        "rendered={rendered:?}"
    );
    assert!(
        !rendered.contains("ui_messages.rs"),
        "technical detail should be hidden by default: {rendered:?}"
    );
}

#[test]
fn test_agentgrep_summary_uses_default_grep_mode_query() {
    let tool = ToolCall {
        id: "agentgrep-default-mode".to_string(),
        name: "agentgrep".to_string(),
        input: serde_json::json!({
            "query": "pending_soft_interrupt",
            "path": "src/tui"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(120));

    assert_eq!(summary, "grep 'pending_soft_interrupt'");
}

#[test]
fn test_render_batch_subcall_line_shows_first_subcall_token_badge() {
    let tool = ToolCall {
        id: "agentgrep-default-mode".to_string(),
        name: "agentgrep".to_string(),
        input: serde_json::json!({
            "query": "pending_soft_interrupt",
            "path": "src/tui"
        }),
        intent: None,
        thought_signature: None,
    };

    let line = tools_ui::render_batch_subcall_line(
        &tool,
        "✓",
        rgb(100, 180, 100),
        50,
        Some(120),
        Some("query: pending_soft_interrupt\nmatches: 1 in 1 files\n"),
    );
    let rendered = extract_line_text(&line);

    assert!(
        rendered.contains("agentgrep grep 'pending_soft_interrupt'"),
        "rendered={rendered:?}"
    );
    assert!(rendered.contains("tok"), "rendered={rendered:?}");
}

#[test]
fn test_common_tool_summaries_keep_full_text_when_row_budget_fits() {
    let cases = vec![
        (
            ToolCall {
                id: "read-wide".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({
                    "file_path": "src/tui/ui_messages.rs",
                    "offset": 120,
                    "limit": 40
                }),
                intent: None,
                thought_signature: None,
            },
            "src/tui/ui_messages.rs:120-160",
        ),
        (
            ToolCall {
                id: "grep-wide".to_string(),
                name: "grep".to_string(),
                input: serde_json::json!({
                    "pattern": "render_batch_subcall_line",
                    "path": "src/tui"
                }),
                intent: None,
                thought_signature: None,
            },
            "'render_batch_subcall_line' in src/tui",
        ),
        (
            ToolCall {
                id: "glob-wide".to_string(),
                name: "glob".to_string(),
                input: serde_json::json!({
                    "pattern": "src/tui/**/*.rs"
                }),
                intent: None,
                thought_signature: None,
            },
            "'src/tui/**/*.rs'",
        ),
        (
            ToolCall {
                id: "webfetch-wide".to_string(),
                name: "webfetch".to_string(),
                input: serde_json::json!({
                    "url": "https://example.com/docs/api/reference"
                }),
                intent: None,
                thought_signature: None,
            },
            "https://example.com/docs/api/reference",
        ),
        (
            ToolCall {
                id: "open-wide".to_string(),
                name: "open".to_string(),
                input: serde_json::json!({
                    "action": "open",
                    "target": "src/tui/ui.rs"
                }),
                intent: None,
                thought_signature: None,
            },
            "open src/tui/ui.rs",
        ),
        (
            ToolCall {
                id: "memory-wide".to_string(),
                name: "memory".to_string(),
                input: serde_json::json!({
                    "action": "recall",
                    "query": "tool summary truncation"
                }),
                intent: None,
                thought_signature: None,
            },
            "recall 'tool summary truncation'",
        ),
        (
            ToolCall {
                id: "codesearch-wide".to_string(),
                name: "codesearch".to_string(),
                input: serde_json::json!({
                    "query": "rust unicode width truncation examples"
                }),
                intent: None,
                thought_signature: None,
            },
            "'rust unicode width truncation examples'",
        ),
        (
            ToolCall {
                id: "debug-wide".to_string(),
                name: "debug_socket".to_string(),
                input: serde_json::json!({
                    "command": "tester:list"
                }),
                intent: None,
                thought_signature: None,
            },
            "tester:list",
        ),
    ];

    for (tool, expected) in cases {
        let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
        assert_eq!(summary, expected, "tool={tool:?} summary={summary:?}");
        assert!(!summary.contains('…'), "tool={tool:?} summary={summary:?}");
    }
}

#[test]
fn test_debug_socket_summary_hides_transient_missing_input() {
    let tool = ToolCall {
        id: "debug-start".to_string(),
        name: "debug_socket".to_string(),
        input: serde_json::Value::Null,
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "");
}

#[test]
fn test_tool_summary_browser_open_shows_url() {
    let tool = ToolCall {
        id: "browser-open".to_string(),
        name: "browser".to_string(),
        input: serde_json::json!({
            "action": "open",
            "url": "https://example.com/docs/reference/browser-tool"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(
        summary,
        "open https://example.com/docs/reference/browser-tool"
    );
}

#[test]
fn test_tool_summary_browser_type_hides_typed_text() {
    let tool = ToolCall {
        id: "browser-type".to_string(),
        name: "browser".to_string(),
        input: serde_json::json!({
            "action": "type",
            "selector": "#password",
            "text": "super-secret-value"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "type #password (18 chars)");
    assert!(
        !summary.contains("super-secret-value"),
        "summary={summary:?}"
    );
}

#[test]
fn test_tool_summary_browser_type_without_selector_still_hides_text() {
    let tool = ToolCall {
        id: "browser-type-no-selector".to_string(),
        name: "browser".to_string(),
        input: serde_json::json!({
            "action": "type",
            "text": "secret-token-123"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "type (16 chars)");
    assert!(!summary.contains("secret-token-123"), "summary={summary:?}");
}

#[test]
fn test_tool_summary_browser_eval_truncates_script() {
    let tool = ToolCall {
        id: "browser-eval".to_string(),
        name: "browser".to_string(),
        input: serde_json::json!({
            "action": "eval",
            "script": "return window.__APP_STATE__?.reallyLongNestedValue?.items?.map(item => item.name).join(', ')"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(34));
    assert!(summary.starts_with("eval "), "summary={summary:?}");
    assert!(summary.contains('…'), "summary={summary:?}");
    assert!(unicode_width::UnicodeWidthStr::width(summary.as_str()) <= 34);
}

#[test]
fn test_tool_summary_agentgrep_smart_uses_terms_subject_relation() {
    let tool = ToolCall {
        id: "agentgrep-smart-terms".to_string(),
        name: "agentgrep".to_string(),
        input: serde_json::json!({
            "mode": "smart",
            "terms": ["subject:agentgrep", "relation:build_args", "path:src/tool"]
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "smart agentgrep:build_args");
}

#[test]
fn test_tool_summary_agentgrep_smart_uses_query_subject_relation() {
    let tool = ToolCall {
        id: "agentgrep-smart-query".to_string(),
        name: "agentgrep".to_string(),
        input: serde_json::json!({
            "mode": "smart",
            "query": "subject:agentgrep relation:build_args path:src/tool"
        }),
        intent: None,
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "smart agentgrep:build_args");
}

#[test]
fn test_tool_summary_bg_infers_wait_from_intent_when_action_missing() {
    let tool = ToolCall {
        id: "bg-intent-only".to_string(),
        name: "bg".to_string(),
        input: serde_json::json!({
            "intent": "Wait for library tests",
            "latest": true
        }),
        intent: Some("Wait for library tests".to_string()),
        thought_signature: None,
    };

    let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
    assert_eq!(summary, "wait");
}

#[test]
fn test_render_tool_message_batch_rows_do_not_soft_wrap_on_narrow_width() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] read ---\nok\n\nCompleted: 1 succeeded, 0 failed".to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: Some(ToolCall {
            id: "call_batch_narrow".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [
                    {
                        "tool": "read",
                        "file_path": "src/tui/really/long/nested/location/ui_messages.rs",
                        "offset": 120,
                        "limit": 40
                    }
                ]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 32, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines.iter().map(extract_line_text).collect();

    assert_eq!(rendered.len(), 2, "rendered={rendered:?}");
    assert!(
        rendered.iter().all(|line| line.width() <= 31),
        "rendered={rendered:?}"
    );
    assert!(rendered[1].contains('…'), "rendered={rendered:?}");
    assert!(rendered[1].contains("tok"), "rendered={rendered:?}");
}

#[test]
fn test_render_tool_message_keeps_token_badge_when_intent_is_truncated() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "ok".to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: Some(ToolCall {
            id: "call_long_intent".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargo test --package jcode --lib tui::ui::tests::very_long_test_name -- --nocapture"
            }),
            intent: Some(
                "Inspect and validate the extremely long wrapping behavior for tool rows"
                    .to_string(),
            ),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 48, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines.iter().map(extract_line_text).collect();

    assert!(!rendered.is_empty(), "rendered={rendered:?}");
    assert!(rendered[0].width() <= 47, "rendered={rendered:?}");
    assert!(rendered[0].contains('…'), "rendered={rendered:?}");
    assert!(rendered[0].contains("tok"), "rendered={rendered:?}");
}

/// With an intent present, the bash command preview must never spill onto a
/// second `$ ...` line. It renders inline when it fits and is dropped when it
/// does not.
#[test]
fn test_render_tool_message_with_intent_never_adds_second_command_line() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "ok".to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: Some(ToolCall {
            id: "call_intent_no_wrap".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "set -euo pipefail; python -c 'import modal' && echo ready"
            }),
            intent: Some("Launch exactly one paid Opus canary".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 60, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines.iter().map(extract_line_text).collect();

    assert!(!rendered.is_empty(), "rendered={rendered:?}");
    assert_eq!(
        rendered.len(),
        1,
        "Bash output is hidden by default: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.trim_start().starts_with('$')),
        "rendered={rendered:?}"
    );
}

#[test]
fn test_render_tool_message_keeps_bash_command_visible_when_row_is_narrow() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "2\n".to_string(),
        tool_calls: vec![],
        duration_secs: None,
        title: None,
        tool_data: Some(ToolCall {
            id: "call_narrow_bash".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "grep -rn \"unwrap()\" src/ --include=\"*.rs\" | wc -l"
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 18, crate::config::DiffDisplayMode::Off);
    let rendered: Vec<String> = lines.iter().map(extract_line_text).collect();

    assert!(
        rendered.iter().any(|line| line.contains("bash")),
        "rendered={rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| line.contains('$')),
        "narrow bash tool rows should include a command preview: {rendered:?}"
    );
}

/// Regression for https://github.com/1jehuang/jcode/issues/284:
/// While a tool call is still streaming, its arguments arrive separately and
/// `input` is `null` (or an empty object) for many render frames. The summary
/// must not show "action missing" / "command missing" placeholders in that
/// window; it should be empty so only the tool name renders.
#[test]
fn test_action_tools_hide_missing_placeholder_for_streaming_input() {
    let action_tools = [
        "bg",
        "swarm",
        "initiative",
        "selfdev",
        "side_panel",
        "memory",
    ];
    let transient_inputs = [serde_json::Value::Null, serde_json::json!({})];

    for name in action_tools {
        for input in &transient_inputs {
            let tool = ToolCall {
                id: format!("{name}-streaming"),
                name: name.to_string(),
                input: input.clone(),
                intent: None,
                thought_signature: None,
            };

            let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
            assert!(
                !summary.contains("missing"),
                "tool={name} input={input} summary={summary:?}"
            );
            assert_eq!(
                summary, "",
                "transient streaming input should yield an empty summary: tool={name} input={input}"
            );
        }
    }
}

/// Even when a tool call carries a populated, valid input object, a missing
/// `action` field must degrade to the tool name rather than the alarming
/// "action missing" placeholder.
#[test]
fn test_action_tools_degrade_to_tool_name_when_action_absent() {
    let cases = [
        ("bg", serde_json::json!({ "task_id": "abc" })),
        ("swarm", serde_json::json!({ "to_session": "worker-1" })),
        ("initiative", serde_json::json!({ "id": "plan-1" })),
        ("memory", serde_json::json!({ "query": "notes" })),
    ];

    for (name, input) in cases {
        let tool = ToolCall {
            id: format!("{name}-no-action"),
            name: name.to_string(),
            input,
            intent: None,
            thought_signature: None,
        };

        let summary = tools_ui::get_tool_summary_with_budget(&tool, 50, Some(200));
        assert!(
            !summary.contains("missing"),
            "tool={name} summary={summary:?}"
        );
    }
}

/// The live activity line should surface the model-provided `intent` for any
/// tool (including swarm) ahead of the technical summary when tool call
/// details are enabled.
#[test]
fn test_activity_detail_prefers_intent_and_appends_summary() {
    tools_ui::tests_tool_call_details_override::set(true);
    let tool = ToolCall {
        id: "swarm-1".to_string(),
        name: "swarm".to_string(),
        input: serde_json::json!({
            "intent": "Spin up a worker for the parser fix",
            "action": "spawn",
            "prompt": "Fix the parser bug in crates/parser"
        }),
        intent: Some("Spin up a worker for the parser fix".to_string()),
        thought_signature: None,
    };

    let detail = tools_ui::get_tool_activity_detail(&tool);
    assert!(
        detail.starts_with("Spin up a worker for the parser fix"),
        "intent should lead the activity detail: {detail:?}"
    );
    assert!(
        detail.contains("spawn"),
        "technical summary should still appear: {detail:?}"
    );
    tools_ui::tests_tool_call_details_override::set(false);
}

/// When the `ToolCall.intent` field is not populated yet (e.g. streamed input
/// parsed but intent refresh missed), fall back to the raw `intent` input key.
#[test]
fn test_activity_detail_falls_back_to_input_intent_field() {
    let tool = ToolCall {
        id: "swarm-2".to_string(),
        name: "swarm".to_string(),
        input: serde_json::json!({
            "intent": "Check on worker progress",
            "action": "status",
            "target_session": "worker-1"
        }),
        intent: None,
        thought_signature: None,
    };

    let detail = tools_ui::get_tool_activity_detail(&tool);
    assert!(
        detail.starts_with("Check on worker progress"),
        "input intent should be used when the field is unset: {detail:?}"
    );
}

/// Without an intent, the activity detail matches the plain technical summary.
#[test]
fn test_activity_detail_without_intent_matches_summary() {
    let tool = ToolCall {
        id: "swarm-3".to_string(),
        name: "swarm".to_string(),
        input: serde_json::json!({ "action": "dm", "to_session": "worker-1", "message": "hello" }),
        intent: None,
        thought_signature: None,
    };

    let detail = tools_ui::get_tool_activity_detail(&tool);
    let summary = tools_ui::get_tool_summary(&tool);
    assert_eq!(detail, summary);
    assert!(!detail.is_empty());
}

include!("tools/batch.rs");
