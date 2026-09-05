use super::*;

fn advisor_test_route(method: &str, available: bool) -> crate::provider::ModelRoute {
    crate::provider::ModelRoute {
        model: "gpt-5".into(),
        provider: "OpenAI".into(),
        api_method: method.into(),
        available,
        detail: "authenticated".into(),
        cheapness: None,
    }
}

fn advisor_test_result(
    selection: Option<crate::provider::RouteSelection>,
) -> crate::protocol::AdvisorControlResult {
    crate::protocol::AdvisorControlResult {
        model_options: Some(crate::protocol::AdvisorModelOptions {
            selection,
            reasoning_effort: Some("medium".into()),
            available_routes: vec![
                advisor_test_route("openai-oauth", true),
                advisor_test_route("openai-api", false),
            ],
            available_selections: vec![],
            available_efforts: vec!["low".into(), "medium".into(), "high".into()],
        }),
        ..Default::default()
    }
}

async fn advisor_read_request(
    app: &mut App,
    remote: &mut crate::tui::backend::RemoteConnection,
    reader: &mut tokio::io::BufReader<crate::transport::ReadHalf>,
) -> (u64, serde_json::Value) {
    use tokio::io::AsyncBufReadExt;
    app.forward_pending_advisor_request(remote).await;
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let wire: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(wire["type"], "advisor");
    (wire["id"].as_u64().unwrap(), wire["request"].clone())
}

#[test]
fn advisor_command_opens_picker_and_preserves_controls() {
    use crate::protocol::AdvisorRequest;
    use crate::tui::app::advisor_picker::command;
    assert_eq!(
        command("/advisor").unwrap().unwrap(),
        AdvisorRequest::ModelOptions { selection: None }
    );
    assert_eq!(
        command("/advisor model").unwrap().unwrap(),
        AdvisorRequest::ModelOptions { selection: None }
    );
    assert_eq!(
        command("/advisor inherit").unwrap().unwrap(),
        AdvisorRequest::UsePrimary
    );
    assert_eq!(
        command("/advisor status").unwrap().unwrap(),
        AdvisorRequest::Status
    );
    assert_eq!(
        command("/advisor off").unwrap().unwrap(),
        AdvisorRequest::Disable
    );
    assert_eq!(
        command("/advisor ack adv_1").unwrap().unwrap(),
        AdvisorRequest::Acknowledge {
            note_id: "adv_1".into()
        }
    );
    assert!(command("/advisor ack").unwrap().is_err());
    assert!(command("/advisor status trailing").unwrap().is_err());
    assert!(command("/advisory").is_none());
}

#[test]
fn advisor_picker_selects_oauth_route_and_effort_without_switching_main() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_provider_model = Some("main-model".into());
        app.remote_reasoning_effort = Some("low".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = crate::tui::backend::RemoteConnection::dummy();
            let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
            let mut reader = tokio::io::BufReader::new(peer);
            app.input = "/advisor".into();
            app.cursor_pos = app.input.len();
            app.handle_remote_key(KeyCode::Enter, KeyModifiers::NONE, &mut remote)
                .await
                .unwrap();
            let (id, request) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            assert_eq!(request["action"], "model_options");
            app.handle_advisor_result(id, advisor_test_result(None));
            let picker = app.inline_interactive_state.as_mut().unwrap();
            assert!(picker.is_advisor_picker());
            assert_eq!(
                picker.entries.len(),
                2,
                "inherit plus authenticated OAuth route; no API key needed"
            );
            assert_eq!(picker.entries[1].options[0].api_method, "openai-oauth");
            assert!(!picker.shows_default_shortcut_hint());
            picker.selected = 1;
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            let (id, request) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            assert_eq!(
                request["selection"]["runtime_key"],
                serde_json::to_value(crate::provider::RuntimeKey::OpenAIOAuth).unwrap()
            );
            let mut route = crate::provider::RouteSelection::from_model_route(&advisor_test_route(
                "openai-oauth",
                true,
            ));
            route.detail.clear();
            app.handle_advisor_result(id, advisor_test_result(Some(route)));
            let picker = app.inline_interactive_state.as_mut().unwrap();
            assert_eq!(picker.entries.len(), 3);
            assert_eq!(picker.selected, 1, "current effort selected");
            picker.selected = 2;
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            let (_, request) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            assert_eq!(request["action"], "select_model");
            assert_eq!(request["reasoning_effort"], "high");
            assert_eq!(
                request["selection"]["runtime_key"],
                serde_json::to_value(crate::provider::RuntimeKey::OpenAIOAuth).unwrap()
            );
            assert!(app.pending_model_switch.is_none());
            assert!(app.pending_route_selection.is_none());
            assert!(app.pending_reasoning_effort.is_none());
            assert_eq!(app.remote_provider_model.as_deref(), Some("main-model"));
            assert_eq!(app.remote_reasoning_effort.as_deref(), Some("low"));
        });
    });
}

#[test]
fn advisor_picker_cancel_and_session_change_ignore_late_results() {
    let mut app = create_test_app();
    app.is_remote = true;
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        for switch_session in [false, true] {
            app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
                selection: None,
            });
            let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            if switch_session {
                app.remote_session_id = Some("another-session".into());
                app.cancel_advisor_picker();
            } else {
                app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
                    .unwrap();
            }
            app.handle_advisor_result(id, advisor_test_result(None));
            assert!(app.inline_interactive_state.is_none());
        }
    });
}

#[test]
fn advisor_picker_uses_supported_efforts_and_does_not_save_main_defaults() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = crate::tui::backend::RemoteConnection::dummy();
            let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
            let mut reader = tokio::io::BufReader::new(peer);
            let route = crate::provider::RouteSelection::from_model_route(&advisor_test_route(
                "openai-oauth",
                true,
            ));
            app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
                selection: Some(route.clone()),
            });
            let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            let mut result = advisor_test_result(Some(route));
            result.model_options.as_mut().unwrap().available_efforts =
                vec!["swarm".into(), "swarm-deep".into()];
            result.model_options.as_mut().unwrap().reasoning_effort = None;
            app.handle_advisor_result(id, result);
            let picker = app.inline_interactive_state.as_ref().unwrap();
            assert_eq!(picker.entries.len(), 1);
            assert!(picker.entries[0].name.contains("no effort setting"));
            let before = crate::config::Config::load().provider.default_model.clone();
            app.handle_inline_interactive_key(KeyCode::Char('o'), KeyModifiers::CONTROL)
                .unwrap();
            app.handle_inline_interactive_key(KeyCode::Char('n'), KeyModifiers::CONTROL)
                .unwrap();
            assert_eq!(crate::config::Config::load().provider.default_model, before);
            assert!(!app.inline_interactive_state.as_ref().unwrap().entries[0].is_favorite);
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            let (_, request) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            assert!(request["reasoning_effort"].is_null());
        });
    });
}

#[test]
fn advisor_picker_displays_server_rejection_and_ignores_old_session_controls() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        app.queue_advisor_request(crate::protocol::AdvisorRequest::UsePrimary);
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        app.handle_advisor_result(
            id,
            crate::protocol::AdvisorControlResult {
                error: Some("This route is not permitted for the advisor".into()),
                ..Default::default()
            },
        );
        assert!(
            app.display_messages()
                .last()
                .unwrap()
                .content
                .contains("not permitted")
        );
        let before = app.display_messages().len();
        app.queue_advisor_request(crate::protocol::AdvisorRequest::UsePrimary);
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        app.remote_session_id = Some("another-session".into());
        app.handle_advisor_result(
            id,
            crate::protocol::AdvisorControlResult {
                message: "Advisor enabled for this session".into(),
                ..Default::default()
            },
        );
        assert_eq!(app.display_messages().len(), before);
        assert!(app.pending_model_switch.is_none());
    });
}

#[test]
fn advisor_picker_ignores_cancelled_and_superseded_errors() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        for generic_error in [false, true] {
            for superseded in [false, true] {
                app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
                    selection: None,
                });
                let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
                if superseded {
                    app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
                        selection: None,
                    });
                } else {
                    app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
                        .unwrap();
                }
                let before = app.display_messages().len();
                if generic_error {
                    app.handle_server_event(
                        crate::protocol::ServerEvent::Error {
                            id,
                            message: "stale picker failure".into(),
                            retry_after_secs: None,
                        },
                        &mut remote,
                    );
                } else {
                    app.handle_advisor_result(
                        id,
                        crate::protocol::AdvisorControlResult {
                            error: Some("stale picker failure".into()),
                            ..Default::default()
                        },
                    );
                }
                assert_eq!(app.display_messages().len(), before);
                assert_eq!(app.inline_interactive_state.is_some(), superseded);
                app.cancel_advisor_picker();
            }
        }
    });
}

#[test]
fn advisor_picker_generic_errors_preserve_active_main_turn() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        for request in [
            crate::protocol::AdvisorRequest::ModelOptions { selection: None },
            crate::protocol::AdvisorRequest::Disable,
        ] {
            app.queue_advisor_request(request);
            let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
            app.is_processing = true;
            app.status = ProcessingStatus::Streaming;
            app.current_message_id = Some(999);
            app.processing_started = Some(Instant::now());
            app.last_submitted_input = Some("main task".into());
            app.remote_resume_activity = Some(RemoteResumeActivity {
                session_id: "main session".into(),
                observed_at: Instant::now(),
                current_tool_name: None,
            });
            app.handle_server_event(
                crate::protocol::ServerEvent::Error {
                    id,
                    message: "advisor control rejected".into(),
                    retry_after_secs: Some(60),
                },
                &mut remote,
            );
            assert!(app.is_processing);
            assert!(matches!(app.status, ProcessingStatus::Streaming));
            assert_eq!(app.current_message_id, Some(999));
            assert!(app.processing_started.is_some());
            assert!(app.remote_resume_activity.is_some());
            assert_eq!(app.last_submitted_input.as_deref(), Some("main task"));
            assert!(app.rate_limit_reset.is_none());
            assert!(app.inline_interactive_state.is_none());
            assert!(
                app.display_messages()
                    .last()
                    .unwrap()
                    .content
                    .contains("advisor control rejected")
            );
        }
        app.queue_advisor_request(crate::protocol::AdvisorRequest::Enable);
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        app.remote_session_id = Some("new session".into());
        app.cancel_advisor_picker();
        let before = app.display_messages().len();
        app.handle_server_event(
            crate::protocol::ServerEvent::Error {
                id,
                message: "old session control rejected".into(),
                retry_after_secs: None,
            },
            &mut remote,
        );
        assert_eq!(app.display_messages().len(), before);
        assert!(app.is_processing);
        assert_eq!(app.current_message_id, Some(999));
    });
}

#[test]
fn advisor_picker_cancel_before_dispatch_sends_nothing() {
    use tokio::io::AsyncBufReadExt;
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
            selection: None,
        });
        app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        app.forward_pending_advisor_request(&mut remote).await;
        let mut line = String::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.read_line(&mut line))
                .await
                .is_err()
        );
        assert!(app.inline_interactive_state.is_none());
    });
}

#[test]
fn advisor_picker_disconnect_clears_pending_controls_and_can_reopen_same_session() {
    use tokio::io::AsyncBufReadExt;
    let mut app = create_test_app();
    app.remote_session_id = Some("same session".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
            selection: None,
        });
        let (old_id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        let mut state = remote::RemoteRunState::default();
        remote::handle_disconnect(&mut app, &mut state, None);
        assert!(app.inline_interactive_state.is_none());
        assert_eq!(app.remote_session_id.as_deref(), Some("same session"));
        let before = app.display_messages().len();
        app.handle_advisor_result(old_id, advisor_test_result(None));
        assert_eq!(app.display_messages().len(), before);
        assert!(app.inline_interactive_state.is_none());

        app.queue_advisor_request(crate::protocol::AdvisorRequest::Disable);
        remote::handle_disconnect(&mut app, &mut state, None);
        app.forward_pending_advisor_request(&mut remote).await;
        let mut line = String::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.read_line(&mut line))
                .await
                .is_err(),
            "uncertain advisor controls must not replay after reconnect"
        );
        app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
            selection: None,
        });
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        app.handle_advisor_result(id, advisor_test_result(None));
        assert!(app.inline_interactive_state.as_ref().unwrap().is_advisor_picker());
        assert_eq!(app.inline_interactive_state.as_ref().unwrap().entries.len(), 2);
    });
}

#[test]
fn advisor_picker_preserves_canonical_profile_identity() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
            selection: None,
        });
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        let mut selection = crate::provider::RouteSelection {
            model: "team-model".into(),
            runtime_key: crate::provider::RuntimeKey::OpenAiCompatible {
                profile_id: Some("team-profile".into()),
            },
            api_method: "openai-compatible".into(),
            provider_label: "Display label differs from profile ID".into(),
            detail: "not required on selection".into(),
        };
        let mut result = advisor_test_result(None);
        result.model_options.as_mut().unwrap().available_selections = vec![selection.clone()];
        app.handle_advisor_result(id, result);
        let picker = app.inline_interactive_state.as_mut().unwrap();
        assert_eq!(picker.entries.len(), 2, "canonical catalog replaces legacy routes");
        assert_eq!(picker.entries[1].name, "team-model");
        picker.selected = 1;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert!(
            app.inline_interactive_state.as_ref().unwrap().entries[0]
                .name
                .contains("reasoning efforts")
        );
        let (_, request) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        selection.detail.clear();
        assert_eq!(request["selection"], serde_json::to_value(selection).unwrap());
    });
}

#[test]
fn advisor_picker_explains_empty_permitted_catalog() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
        let mut reader = tokio::io::BufReader::new(peer);
        app.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
            selection: None,
        });
        let (id, _) = advisor_read_request(&mut app, &mut remote, &mut reader).await;
        let mut result = advisor_test_result(None);
        result.model_options.as_mut().unwrap().available_routes.clear();
        app.handle_advisor_result(id, result);
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert_eq!(picker.entries.len(), 2);
        assert!(picker.entries[1].name.contains("No available advisor models"));
        assert!(!picker.entries[1].options[0].available);
        assert!(picker.entries[1].options[0].provider.contains("/login"));
    });
}
