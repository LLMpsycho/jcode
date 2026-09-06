#[test]
fn test_observe_command_enables_transient_page_without_persisting() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.input = "/observe on".to_string();
        app.submit_input();

        assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("observe"));
        let page = app.side_panel.focused_page().expect("missing observe page");
        assert_eq!(page.title, "Observe");
        assert_eq!(
            page.source,
            crate::side_panel::SidePanelPageSource::Ephemeral
        );
        assert!(
            page.content
                .contains("Waiting for the next tool call or tool result")
        );

        let persisted = crate::side_panel::snapshot_for_session(&app.session.id)
            .expect("load persisted side panel");
        assert!(persisted.pages.is_empty());
        assert!(persisted.focused_page_id.is_none());
    });
}
#[test]
fn test_splitview_command_enables_transient_page_without_persisting() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.input = "/splitview on".to_string();
        app.submit_input();

        assert_eq!(
            app.side_panel.focused_page_id.as_deref(),
            Some("split_view")
        );
        let page = app
            .side_panel
            .focused_page()
            .expect("missing split view page");
        assert_eq!(page.title, "Split View");
        assert_eq!(
            page.source,
            crate::side_panel::SidePanelPageSource::Ephemeral
        );
        assert!(page.content.contains("Mirror of the current chat"));

        let persisted = crate::side_panel::snapshot_for_session(&app.session.id)
            .expect("load persisted side panel");
        assert!(persisted.pages.is_empty());
        assert!(persisted.focused_page_id.is_none());
    });
}
#[test]
fn test_splitview_command_off_restores_previous_side_panel_page() {
    let mut app = create_test_app();
    app.set_side_panel_snapshot(test_side_panel_snapshot("plan", "Plan"));

    app.input = "/splitview on".to_string();
    app.submit_input();
    assert_eq!(
        app.side_panel.focused_page_id.as_deref(),
        Some("split_view")
    );
    assert!(app.side_panel.pages.iter().any(|page| page.id == "plan"));

    app.input = "/splitview off".to_string();
    app.submit_input();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("plan"));
    assert!(
        !app.side_panel
            .pages
            .iter()
            .any(|page| page.id == "split_view")
    );
}
#[test]
fn test_splitview_mirrors_chat_and_streaming_text() {
    let mut app = create_test_app();
    app.display_messages = vec![
        DisplayMessage::system("System note".to_string()),
        DisplayMessage::user("What did we decide?".to_string()),
        DisplayMessage::assistant("We decided to ship it.".to_string()),
    ];
    app.bump_display_messages_version();
    app.streaming.streaming_text = "Working on the follow-up now...".to_string();
    app.set_split_view_enabled(true, true);

    let page = app
        .side_panel
        .focused_page()
        .expect("missing split view page");
    assert!(page.content.contains("## System"));
    assert!(page.content.contains("## Prompt 1"));
    assert!(page.content.contains("What did we decide?"));
    assert!(page.content.contains("## Response 1"));
    assert!(page.content.contains("We decided to ship it."));
    assert!(page.content.contains("## Live response"));
    assert!(page.content.contains("Working on the follow-up now..."));
}
#[test]
fn test_splitview_does_not_build_cache_while_disabled() {
    let mut app = create_test_app();
    app.display_messages = vec![
        DisplayMessage::user("What did we decide?".to_string()),
        DisplayMessage::assistant("We decided to ship it.".to_string()),
    ];

    app.bump_display_messages_version();

    assert!(!app.split_view_enabled());
    assert!(app.split_view_markdown.is_empty());
}
#[test]
fn test_splitview_disable_clears_cached_markdown() {
    let mut app = create_test_app();
    app.display_messages = vec![
        DisplayMessage::user("What did we decide?".to_string()),
        DisplayMessage::assistant("We decided to ship it.".to_string()),
    ];
    app.bump_display_messages_version();
    app.set_split_view_enabled(true, true);

    assert!(!app.split_view_markdown.is_empty());

    app.set_split_view_enabled(false, false);

    assert!(app.split_view_markdown.is_empty());
}
#[test]
fn test_observe_command_off_restores_previous_side_panel_page() {
    let mut app = create_test_app();
    app.set_side_panel_snapshot(test_side_panel_snapshot("plan", "Plan"));

    app.input = "/observe on".to_string();
    app.submit_input();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("observe"));
    assert!(app.side_panel.pages.iter().any(|page| page.id == "plan"));

    app.input = "/observe off".to_string();
    app.submit_input();
    assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("plan"));
    assert!(!app.side_panel.pages.iter().any(|page| page.id == "observe"));
}
#[test]
fn test_observe_updates_latest_tool_context_only() {
    let mut app = create_test_app();
    app.input = "/observe on".to_string();
    app.submit_input();

    let tool_call = crate::message::ToolCall {
        id: "tool_1".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({"file_path": "src/main.rs", "start_line": 1, "end_line": 10}),
        intent: None,
        thought_signature: None,
    };
    app.observe_tool_call(&tool_call);

    let page = app.side_panel.focused_page().expect("missing observe page");
    assert!(
        page.content
            .contains("Latest tool call emitted by the model")
    );
    assert!(page.content.contains("read"));
    assert!(page.content.contains("src/main.rs"));

    app.observe_tool_result(&tool_call, "1 use std::path::Path;", false, Some("read"));

    let page = app.side_panel.focused_page().expect("missing observe page");
    let token_label = crate::util::format_approx_token_count(crate::util::estimate_tokens(
        "1 use std::path::Path;",
    ));
    assert!(page.content.contains("Latest tool result added to context"));
    assert!(page.content.contains("Status: completed"));
    assert!(page.content.contains("Returned to context"));
    assert!(page.content.contains(&token_label));
    assert!(page.content.contains("1 use std::path::Path;"));
    assert!(
        !page
            .content
            .contains("Latest tool call emitted by the model")
    );
}
#[test]
fn test_observe_ignores_noise_tools_and_preserves_latest_useful_context() {
    let mut app = create_test_app();
    app.input = "/observe on".to_string();
    app.submit_input();

    let read_tool = crate::message::ToolCall {
        id: "tool_read".to_string(),
        name: "read".to_string(),
        input: serde_json::json!({"file_path": "src/main.rs"}),
        intent: None,
        thought_signature: None,
    };
    app.observe_tool_result(&read_tool, "fn main() {}", false, Some("read"));
    let before = app
        .side_panel
        .focused_page()
        .expect("missing observe page")
        .content
        .clone();

    let noise_tool = crate::message::ToolCall {
        id: "tool_side_panel".to_string(),
        name: "side_panel".to_string(),
        input: serde_json::json!({"action": "write", "page_id": "plan"}),
        intent: None,
        thought_signature: None,
    };
    app.observe_tool_call(&noise_tool);
    app.observe_tool_result(&noise_tool, "ok", false, Some("side_panel"));

    let after = app
        .side_panel
        .focused_page()
        .expect("missing observe page")
        .content
        .clone();
    assert_eq!(after, before);
    assert!(after.contains("fn main() {}"));
    assert!(!after.contains("tool_side_panel"));
}
