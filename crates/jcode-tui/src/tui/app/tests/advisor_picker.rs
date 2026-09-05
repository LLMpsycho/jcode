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
