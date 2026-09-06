#[test]
fn test_handle_server_event_history_clears_connection_type_on_session_change_when_missing() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_old".to_string());
    app.connection_type = Some("websocket".to_string());

    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 1,
            session_id: "session_new".to_string(),
            messages: vec![],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    assert_eq!(app.remote_session_id.as_deref(), Some("session_new"));
    assert_eq!(app.connection_type, None);
}
#[test]
fn test_handle_server_event_history_preserves_connection_type_for_same_session_when_missing() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_same".to_string());
    app.connection_type = Some("websocket".to_string());

    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 1,
            session_id: "session_same".to_string(),
            messages: vec![],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    assert_eq!(app.remote_session_id.as_deref(), Some("session_same"));
    assert_eq!(app.connection_type.as_deref(), Some("websocket"));
}
#[test]
fn test_handle_server_event_history_preserves_reasoning_effort_for_same_session_when_missing() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_same".to_string());
    app.remote_reasoning_effort = Some("medium".to_string());

    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 1,
            session_id: "session_same".to_string(),
            messages: vec![],
            images: vec![],
            provider_name: Some("anthropic".to_string()),
            provider_model: Some("claude-opus-4-1".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    assert_eq!(app.remote_reasoning_effort.as_deref(), Some("medium"));
}
#[test]
fn test_handle_server_event_history_session_change_clears_streaming_preview_diagram() {
    // Regression pin: a mermaid preview registered mid-stream (via
    // set_streaming_preview_diagram from the streaming markdown render) must
    // not leak into a different session when a session-changing History event
    // arrives while the stream is still in flight. The History handler's
    // session_changed branch (remote/server_events.rs) calls
    // clear_streaming_render_state() (app/input.rs), which clears the
    // streaming preview slot.
    //
    // Serialize with the other tests that mutate the process-global mermaid
    // preview slot / ACTIVE_DIAGRAMS registry.
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_old".to_string());
    // Simulate a mid-stream turn: in-flight streaming text and processing on.
    app.streaming.streaming_text = "```mermaid\ngraph TD; A-->B\n```".to_string();
    app.is_processing = true;
    // The streaming markdown renderer registered a preview for the in-flight
    // fenced block (markdown_render_full.rs set_streaming_preview_diagram).
    let preview_hash: u64 = 0xDEAD_BEEF_5EAF_0001;
    crate::tui::mermaid::set_streaming_preview_diagram(
        preview_hash,
        320,
        240,
        Some("stream-preview".to_string()),
    );
    assert!(
        crate::tui::mermaid::get_active_diagrams()
            .iter()
            .any(|d| d.hash == preview_hash),
        "test setup: streaming preview should be visible before the History event"
    );

    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 1,
            session_id: "session_new".to_string(),
            messages: vec![],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    assert_eq!(app.remote_session_id.as_deref(), Some("session_new"));
    assert!(
        app.streaming.streaming_text.is_empty(),
        "session-changing History must drop in-flight streaming text"
    );
    assert!(
        !crate::tui::mermaid::get_active_diagrams()
            .iter()
            .any(|d| d.hash == preview_hash),
        "streaming preview diagram leaked across a session-changing History event"
    );
}
#[test]
fn test_handle_server_event_history_same_session_rewind_reapply_clears_streaming_preview_diagram() {
    // Regression pin: a remote /rewind (or /rewind undo) triggers a History
    // redelivery for the SAME session id, so the session_changed branch of the
    // History handler (and its clear_streaming_render_state call) never runs.
    // The forced re-apply path (replace_display_messages) must still drop any
    // registered streaming preview diagram, otherwise a preview registered
    // mid-stream keeps rendering a mermaid block from a message that was just
    // rewound away (it sits at index 0 of get_active_diagrams in Margin mode).
    //
    // Serialize with the other tests that mutate the process-global mermaid
    // preview slot / ACTIVE_DIAGRAMS registry.
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_rewind_preview".to_string());
    remote.set_session_id("session_rewind_preview".to_string());
    // A stale streaming preview is still registered (e.g. the turn ended via a
    // path that did not clear it, or the rewind raced the end of the stream).
    let preview_hash: u64 = 0xDEAD_BEEF_5EAF_0002;
    crate::tui::mermaid::set_streaming_preview_diagram(
        preview_hash,
        320,
        240,
        Some("stream-preview".to_string()),
    );
    assert!(
        crate::tui::mermaid::get_active_diagrams()
            .iter()
            .any(|d| d.hash == preview_hash),
        "test setup: streaming preview should be visible before the History event"
    );
    // The client-side /rewind path arms a pending notice before the server's
    // History redelivery arrives (remote/key_handling.rs).
    app.pending_remote_rewind_notice = Some(crate::tui::app::PendingRemoteRewindNotice {
        undo: false,
        message_index: Some(1),
        changed_messages: 2,
    });

    // Truncated payload after the rewind: same session id, fewer messages.
    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 2,
            session_id: "session_rewind_preview".to_string(),
            messages: vec![crate::protocol::HistoryMessage {
                role: "user".to_string(),
                content: "first message kept by the rewind".to_string(),
                tool_calls: None,
                tool_data: None,
            }],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    // Same session id: this exercised the !session_changed re-apply path.
    assert_eq!(
        app.remote_session_id.as_deref(),
        Some("session_rewind_preview")
    );
    // Payload was applied (transcript rebuilt + rewind notice consumed).
    assert!(
        app.display_messages()
            .iter()
            .any(|m| m.content.contains("first message kept by the rewind")),
        "rewind-truncated History payload must be re-applied for the same session"
    );
    assert!(
        app.pending_remote_rewind_notice.is_none(),
        "pending rewind notice should be consumed by the History re-apply"
    );
    assert!(
        !crate::tui::mermaid::get_active_diagrams()
            .iter()
            .any(|d| d.hash == preview_hash),
        "streaming preview diagram leaked across a same-session rewind History re-apply"
    );
}
#[test]
fn test_handle_server_event_history_same_session_midstream_duplicate_is_dropped_and_keeps_preview()
{
    // Multi-client rewind fan-out pin (server side has NO fan-out: a /rewind
    // History redelivery is written only to the rewinding connection's socket,
    // per-client event channel, server/client_lifecycle.rs:521 and
    // client_state.rs handle_get_history). This test pins the CLIENT side of
    // that contract: if a mid-stream client (is_processing=true, bootstrap
    // already complete: has_loaded_history=true) ever receives a same-session
    // History payload it did not request, the payload must be DROPPED
    // (server_events.rs should_apply_history_payload gate), the local
    // transcript must stay intact, and the live streaming preview diagram must
    // NOT be cleared (the c7612068 preview-clear only runs on the forced
    // re-apply path, which only the rewinding client arms by resetting
    // has_loaded_history in backend.rs rewind()/rewind_undo()).
    //
    // Serialize with the other tests that mutate the process-global mermaid
    // preview slot / ACTIVE_DIAGRAMS registry.
    let _render_lock = scroll_render_test_lock();
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_midstream_dup".to_string());
    remote.set_session_id("session_midstream_dup".to_string());
    remote.mark_history_loaded();

    // Mid-stream state on this (non-rewinding) client.
    app.push_display_message(DisplayMessage::user("kept local message".to_string()));
    app.streaming.streaming_text = "```mermaid\ngraph TD; A-->B\n```".to_string();
    app.is_processing = true;
    let preview_hash: u64 = 0xDEAD_BEEF_5EAF_0003;
    crate::tui::mermaid::set_streaming_preview_diagram(
        preview_hash,
        320,
        240,
        Some("stream-preview".to_string()),
    );

    // Same-session, rewind-truncated payload this client never requested.
    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 3,
            session_id: "session_midstream_dup".to_string(),
            messages: vec![crate::protocol::HistoryMessage {
                role: "user".to_string(),
                content: "truncated payload from another client's rewind".to_string(),
                tool_calls: None,
                tool_data: None,
            }],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    // Payload dropped: local transcript untouched, unsolicited truncation not applied.
    assert!(
        app.display_messages()
            .iter()
            .any(|m| m.content.contains("kept local message")),
        "unsolicited same-session History must not replace a bootstrapped mid-stream transcript"
    );
    assert!(
        !app.display_messages().iter().any(|m| m
            .content
            .contains("truncated payload from another client's rewind")),
        "unsolicited same-session History payload should be dropped, not applied"
    );
    // Live stream state preserved: preview and streaming text survive.
    //
    // The preview slot is process-global and several event paths in OTHER
    // tests clear it without the render lock (Done handlers call
    // clear_streaming_render_state). If the slot looks cleared, re-run the
    // set -> handle_server_event -> check sequence: a genuine regression in
    // should_apply_history_payload clears the preview deterministically on
    // every iteration, while a parallel wipe is transient.
    let preview_survives = || {
        crate::tui::mermaid::get_active_diagrams()
            .iter()
            .any(|d| d.hash == preview_hash)
    };
    let mut survived = preview_survives();
    for _ in 0..50 {
        if survived {
            break;
        }
        crate::tui::mermaid::set_streaming_preview_diagram(
            preview_hash,
            320,
            240,
            Some("stream-preview".to_string()),
        );
        app.handle_server_event(
            crate::protocol::ServerEvent::History {
                id: 3,
                session_id: "session_midstream_dup".to_string(),
                messages: vec![crate::protocol::HistoryMessage {
                    role: "user".to_string(),
                    content: "truncated payload from another client's rewind".to_string(),
                    tool_calls: None,
                    tool_data: None,
                }],
                images: vec![],
                provider_name: Some("claude".to_string()),
                provider_model: Some("claude-sonnet-4-20250514".to_string()),
                subagent_model: None,
                autoreview_enabled: None,
                autojudge_enabled: None,
                available_models: vec![],
                available_model_routes: vec![],
                mcp_servers: vec![],
                skills: vec![],
                total_tokens: None,
                token_usage_totals: None,
                all_sessions: vec![],
                client_count: None,
                is_canary: None,
                reload_recovery: None,
                server_version: None,
                server_name: None,
                server_icon: None,
                server_has_update: None,
                was_interrupted: None,
                connection_type: None,
                status_detail: None,
                upstream_provider: None,
                resolved_credential: None,
                reasoning_effort: None,
                service_tier: None,
                compaction_mode: crate::config::CompactionMode::Reactive,
                activity: None,
                side_panel: crate::side_panel::SidePanelSnapshot::default(),
            },
            &mut remote,
        );
        survived = preview_survives();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        survived,
        "a dropped duplicate History must not clear a live streaming preview diagram"
    );
    assert!(
        !app.streaming.streaming_text.is_empty(),
        "a dropped duplicate History must not clear in-flight streaming text"
    );
    // Cleanup the global preview slot for other tests.
    crate::tui::mermaid::clear_streaming_preview_diagram();
}
#[test]
fn test_handle_server_event_history_same_session_rewind_then_late_done_does_not_resurrect_content()
{
    // Regression pin for the Done-vs-History writer ordering race: a remote
    // /rewind makes the server write the truncated History payload DIRECTLY to
    // the socket (client_lifecycle.rs handle_get_history), while a `Done` from
    // the just-finished turn can still be queued in the per-client mpsc event
    // forwarder. The client can therefore apply the rewind-truncated History
    // FIRST and process the stale Done SECOND. The Done handler flushes
    // stream_buffer and commits any non-empty streaming_text as an assistant
    // message plus a turn footer, resurrecting content that was just rewound
    // away. The same-session force-reapply path must drop stale streaming
    // state (text, buffered ops, tool cards) when a rewind notice is pending
    // so the late Done settles the turn without appending anything.
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_rewind_done_race".to_string());
    remote.set_session_id("session_rewind_done_race".to_string());
    // Stale streaming state left over from the turn whose Done is still queued
    // behind the History payload.
    app.is_processing = true;
    app.status = ProcessingStatus::Streaming;
    app.current_message_id = Some(7);
    app.streaming.streaming_text = "rewound-away assistant text".to_string();
    app.streaming_tool_calls.push(crate::message::ToolCall {
        id: "tool_stale".to_string(),
        name: "bash".to_string(),
        input: serde_json::json!({}),
        intent: None,
        thought_signature: None,
    });
    // The client-side /rewind path arms the pending notice before the server's
    // History redelivery arrives (remote/key_handling.rs).
    app.pending_remote_rewind_notice = Some(crate::tui::app::PendingRemoteRewindNotice {
        undo: false,
        message_index: Some(1),
        changed_messages: 2,
    });

    // Truncated payload after the rewind: same session id, fewer messages.
    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 2,
            session_id: "session_rewind_done_race".to_string(),
            messages: vec![crate::protocol::HistoryMessage {
                role: "user".to_string(),
                content: "first message kept by the rewind".to_string(),
                tool_calls: None,
                tool_data: None,
            }],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            reload_recovery: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    // The re-apply must have dropped the stale streaming state.
    assert!(
        app.streaming.streaming_text.is_empty(),
        "same-session rewind re-apply must drop buffered streaming text"
    );
    assert!(
        app.streaming_tool_calls.is_empty(),
        "same-session rewind re-apply must drop stale streaming tool cards"
    );
    assert!(
        app.pending_remote_rewind_notice.is_none(),
        "pending rewind notice should be consumed by the History re-apply"
    );
    let transcript_len_after_history = app.display_messages().len();

    // The stale Done from the rewound-away turn arrives AFTER the truncated
    // History (mpsc forwarder ordering).
    app.handle_server_event(crate::protocol::ServerEvent::Done { id: 7 }, &mut remote);

    assert!(!app.is_processing, "late Done should settle the turn");
    assert!(
        !app.display_messages()
            .iter()
            .any(|m| m.content.contains("rewound-away assistant text")),
        "late Done must not resurrect assistant text that the rewind removed"
    );
    assert_eq!(
        app.display_messages().len(),
        transcript_len_after_history,
        "late Done must not append messages (assistant text or turn footer) onto the rewind-truncated transcript"
    );
}
#[test]
fn test_handle_server_event_history_session_change_clears_pending_interleaves() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.remote_session_id = Some("session_old".to_string());
    app.queued_messages.push("queued later".to_string());
    app.interleave_message = Some("unsent interleave".to_string());
    app.pending_soft_interrupts = vec!["acked interleave".to_string()];
    app.pending_soft_interrupt_requests = vec![(12, "acked interleave".to_string())];

    app.handle_server_event(
        crate::protocol::ServerEvent::History {
            id: 1,
            session_id: "session_new".to_string(),
            messages: vec![],
            images: vec![],
            provider_name: Some("claude".to_string()),
            provider_model: Some("claude-sonnet-4-20250514".to_string()),
            subagent_model: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            available_models: vec![],
            available_model_routes: vec![],
            mcp_servers: vec![],
            skills: vec![],
            total_tokens: None,
            token_usage_totals: None,
            all_sessions: vec![],
            client_count: None,
            is_canary: None,
            server_version: None,
            server_name: None,
            server_icon: None,
            server_has_update: None,
            was_interrupted: None,
            reload_recovery: None,
            connection_type: None,
            status_detail: None,
            upstream_provider: None,
            resolved_credential: None,
            reasoning_effort: None,
            service_tier: None,
            compaction_mode: crate::config::CompactionMode::Reactive,
            activity: None,
            side_panel: crate::side_panel::SidePanelSnapshot::default(),
        },
        &mut remote,
    );

    assert_eq!(app.remote_session_id.as_deref(), Some("session_new"));
    assert!(app.queued_messages().is_empty());
    assert!(app.interleave_message.is_none());
    assert!(app.pending_soft_interrupts.is_empty());
    assert!(app.pending_soft_interrupt_requests.is_empty());
}
