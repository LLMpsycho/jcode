from pathlib import Path
import re

root = Path('crates/jcode-tui/src/tui/app')

def replace(path, old, new, count=1):
    path = Path(path)
    text = path.read_text()
    assert text.count(old) == count, (str(path), old[:100], text.count(old))
    path.write_text(text.replace(old, new))

replace(root / 'tests/post_merge_review.rs',
        'let child = crate::session::Session::create(None, None);',
        'let mut child = crate::session::Session::create(None, None);')
for item in ['autojudge_status_message', 'autoreview_status_message', 'current_feedback_target_session_id']:
    path = root / 'commands.rs'
    text, n = re.subn(re.escape(item) + r',\s*', '', path.read_text())
    assert n == 1, (item, n)
    path.write_text(text)
replace(root / 'tests/post_merge_review.rs',
        'fn finish_split_fixture(app: &mut App) {',
        'fn finish_split_fixture(app: &mut App) {\n    app.last_completed_split_request_id = app.pending_split_request_id.take();')
replace(root.parent / 'app.rs',
        '    pending_split_request: bool,',
        '    pending_split_request: bool,\n    // Correlate split controls independently from the active model turn.\n    pending_split_request_id: Option<u64>,\n    last_completed_split_request_id: Option<u64>,')
replace(root / 'tui_lifecycle.rs',
        '            pending_split_request: false,',
        '            pending_split_request: false,\n            pending_split_request_id: None,\n            last_completed_split_request_id: None,', count=2)
replace(root / 'commands_review.rs',
        '    app.pending_split_request\n',
        '    app.pending_split_request\n        || app.pending_split_request_id.is_some()\n')
replace(root / 'remote.rs', 'mod review_controls;',
        'mod review_controls;\npub(in crate::tui::app) mod review_launch;')
replace(root / 'remote/review_controls.rs',
        '    if let Err(error) = remote.split().await {',
        '    let split_result = remote.split().await;\n    if let Ok(id) = &split_result {\n        app.pending_split_request_id = Some(*id);\n    }\n    if let Err(error) = split_result {')
replace(root / 'remote/server_events.rs',
        '    let eager_stream_redraw = !crate::perf::tui_policy().enable_decorative_animations;',
        '    if let ServerEvent::Error { id, message, .. } = &event\n        && super::review_launch::handle_error(app, *id, message)\n    {\n        return true;\n    }\n    let eager_stream_redraw = !crate::perf::tui_policy().enable_decorative_animations;')
replace(root / 'remote/server_events.rs',
        '        ServerEvent::SplitResponse {\n            new_session_id,\n            new_session_name,\n            ..\n        } => {',
        '        ServerEvent::SplitResponse {\n            id,\n            new_session_id,\n            new_session_name,\n            ..\n        } => {\n            if !super::review_launch::accept_response(app, id) {\n                return false;\n            }')
replace(root / 'remote/reconnect.rs',
        ') -> Result<PostConnectOutcome> {',
        ') -> Result<PostConnectOutcome> {\n    super::review_launch::reset_connection(app);')
replace(root / 'remote/input_dispatch.rs',
        '    app.route_next_prompt_to_new_session = false;\n    app.pending_split_startup_message = None;',
        '    if super::super::commands_review::review_split_pending(app) {\n        restore_prepared_remote_input(app, prepared);\n        anyhow::bail!("A session launch is already pending; the prompt was restored.");\n    }\n    app.route_next_prompt_to_new_session = false;\n    app.pending_split_startup_message = None;')
replace(root / 'remote/key_handling.rs',
        '                if trimmed == "/transfer" {\n                    if app.pending_transfer_request {',
        '                if trimmed == "/transfer" {\n                    if app_mod::commands_review::review_split_pending(app) {')
replace(root / 'remote/key_handling.rs',
        '"A transfer is already pending.".to_string(),',
        '"A session launch is already pending.".to_string(),')
replace(root / 'tests.rs',
        'mod post_merge_review;',
        'mod post_merge_review;\n#[path = "tests/post_merge_review_launch.rs"]\nmod post_merge_review_launch;')
(root / 'remote/review_launch.rs').write_text('''//! Correlate split controls without consuming an unrelated main-model turn.
use super::{App, DisplayMessage, finish_remote_split_launch};

fn retire(app: &mut App, id: u64) {
    app.pending_split_request_id = None;
    app.last_completed_split_request_id = Some(id);
}

fn clear_launch(app: &mut App) {
    finish_remote_split_launch(app);
    app.pending_split_request = false;
    app.pending_split_started_at = None;
    app.pending_split_startup_message = None;
    app.pending_split_parent_session_id = None;
    app.pending_split_prompt = None;
    app.pending_split_model_override = None;
    app.pending_split_role_selection = None;
    app.pending_split_provider_key_override = None;
    app.pending_split_label = None;
}

pub(in crate::tui::app) fn handle_error(app: &mut App, id: u64, message: &str) -> bool {
    if app.pending_split_request_id != Some(id) {
        return app.last_completed_split_request_id == Some(id)
            && app.current_message_id != Some(id);
    }
    let label = app.pending_split_label.clone().unwrap_or_else(|| "Split".into());
    retire(app, id);
    clear_launch(app);
    app.push_display_message(DisplayMessage::error(format!(
        "Failed to launch {} session: {message}", label.to_lowercase()
    )));
    app.set_status_notice(format!("{label} launch failed"));
    true
}

pub(in crate::tui::app) fn accept_response(app: &mut App, id: u64) -> bool {
    if app.pending_split_request_id.is_some_and(|pending| id != pending)
        || app.last_completed_split_request_id.is_some_and(|completed| id <= completed)
    {
        return false;
    }
    // Existing prompt/workspace split paths also use this response handler.
    // Only dispatcher-owned requests participate in this completion fence.
    if app.pending_split_request_id == Some(id) {
        retire(app, id);
    }
    true
}

pub(in crate::tui::app) fn reset_connection(app: &mut App) {
    super::super::commands_review::cancel_automatic_reviews(app);
    if app.pending_split_request_id.take().is_some() {
        clear_launch(app);
        app.push_display_message(DisplayMessage::system(
            "Connection changed during a session launch. Its outcome is unknown; inspect sessions before retrying. It will not be replayed automatically.".into(),
        ));
    }
    // Request IDs restart on each RemoteConnection. Never compare across sockets.
    app.last_completed_split_request_id = None;
}
''')
(root / 'tests/post_merge_review_launch.rs').write_text('''use super::*;
use tokio::io::{AsyncBufReadExt, BufReader};

#[test]
fn post_merge_review_split_error_releases_launch_and_preserves_main_turn() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_session_id = Some(app.session.id.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
            let mut reader = BufReader::new(peer);
            remote::submit_remote_slash_input(&mut app, &mut remote, input::PreparedInput {
                raw_input: "/review".into(), expanded: "/review".into(), images: vec![],
            }).await.unwrap();
            let mut line = String::new();
            tokio::time::timeout(std::time::Duration::from_secs(2), reader.read_line(&mut line)).await.unwrap().unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = request["id"].as_u64().unwrap();
            assert_eq!(app.pending_split_request_id, Some(id));
            assert!(!remote::review_launch::accept_response(&mut app, id + 100));
            assert_eq!(app.pending_split_label.as_deref(), Some("Review"));
            app.is_processing = true;
            app.current_message_id = Some(id + 1);
            app.active_skill = Some("keep-me".into());
            app.handle_server_event(ServerEvent::Error { id, message: "Failed to save split session".into(), retry_after_secs: None }, &mut remote);
            assert!(!commands_review::review_split_pending(&app));
            assert!(app.is_processing);
            assert_eq!(app.current_message_id, Some(id + 1));
            assert_eq!(app.active_skill.as_deref(), Some("keep-me"));
            assert!(app.display_messages.iter().any(|m| m.content.contains("Failed to launch review")));
            app.handle_server_event(ServerEvent::Error { id, message: "late duplicate".into(), retry_after_secs: None }, &mut remote);
            assert!(app.is_processing);
            assert!(!remote::review_launch::accept_response(&mut app, id));
            commands_review::queue_review_spawn_remote(&mut app, "Judge", "parent".into(), "retry".into()).unwrap();
            assert_eq!(app.pending_split_label.as_deref(), Some("Judge"));
        });
    });
}

#[test]
fn post_merge_review_matching_response_fences_duplicates_and_connection_reset() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.pending_split_request_id = Some(41);
        app.pending_split_label = Some("Review".into());
        assert!(!remote::review_launch::accept_response(&mut app, 40));
        assert!(remote::review_launch::accept_response(&mut app, 41));
        assert!(!remote::review_launch::accept_response(&mut app, 41));
        app.pending_split_request_id = Some(42);
        app.autoreview_after_current_turn = true;
        app.autojudge_after_current_turn = true;
        remote::review_launch::reset_connection(&mut app);
        assert!(!commands_review::review_split_pending(&app));
        assert!(!app.autoreview_after_current_turn);
        assert!(!app.autojudge_after_current_turn);
        assert!(app.display_messages.iter().any(|m| m.content.contains("outcome is unknown")));
        assert!(remote::review_launch::accept_response(&mut app, 1));
    });
}

#[test]
fn post_merge_review_prompt_and_transfer_cannot_overwrite_pending_review() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        commands_review::queue_review_spawn_remote(&mut app, "Review", "parent".into(), "original".into()).unwrap();
        app.route_next_prompt_to_new_session = true;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            let result = remote::route_prepared_input_to_new_remote_session(&mut app, &mut remote, input::PreparedInput {
                raw_input: "keep draft".into(), expanded: "keep draft".into(), images: vec![],
            }).await;
            assert!(result.is_err());
            assert_eq!(app.input, "keep draft");
            assert!(app.route_next_prompt_to_new_session);
            assert_eq!(app.pending_split_startup_message.as_deref(), Some("original"));
            app.route_next_prompt_to_new_session = false;
            app.input = "/transfer".into();
            app.cursor_pos = app.input.len();
            app.handle_remote_key(KeyCode::Enter, KeyModifiers::NONE, &mut remote).await.unwrap();
            assert_eq!(app.pending_split_label.as_deref(), Some("Review"));
            assert_eq!(app.pending_split_startup_message.as_deref(), Some("original"));
            assert!(!app.pending_transfer_request);
        });
    });
}
''')
