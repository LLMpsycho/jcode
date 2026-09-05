use super::*;
use crate::protocol::ServerEvent;
use crate::tui::backend::RemoteConnection;
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
            remote::submit_remote_slash_input(
                &mut app,
                &mut remote,
                input::PreparedInput {
                    raw_input: "/review".into(),
                    expanded: "/review".into(),
                    images: vec![],
                },
            )
            .await
            .unwrap();
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                reader.read_line(&mut line),
            )
            .await
            .unwrap()
            .unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = request["id"].as_u64().unwrap();
            assert_eq!(app.pending_split_request_id, Some(id));
            assert!(!remote::review_launch::accept_response(&mut app, id + 100));
            assert_eq!(app.pending_split_label.as_deref(), Some("Review"));
            app.is_processing = true;
            app.current_message_id = Some(id + 1);
            app.active_skill = Some("keep-me".into());
            app.handle_server_event(
                ServerEvent::Error {
                    id,
                    message: "Failed to save split session".into(),
                    retry_after_secs: None,
                },
                &mut remote,
            );
            assert!(!commands_review::review_split_pending(&app));
            assert!(app.is_processing);
            assert_eq!(app.current_message_id, Some(id + 1));
            assert_eq!(app.active_skill.as_deref(), Some("keep-me"));
            assert!(
                app.display_messages
                    .iter()
                    .any(|m| m.content.contains("Failed to launch review"))
            );
            app.handle_server_event(
                ServerEvent::Error {
                    id,
                    message: "late duplicate".into(),
                    retry_after_secs: None,
                },
                &mut remote,
            );
            assert!(app.is_processing);
            assert!(!remote::review_launch::accept_response(&mut app, id));
            commands_review::queue_review_spawn_remote(
                &mut app,
                "Judge",
                "parent".into(),
                "retry".into(),
            )
            .unwrap();
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
        assert!(
            app.display_messages
                .iter()
                .any(|m| m.content.contains("outcome is unknown"))
        );
        assert!(remote::review_launch::accept_response(&mut app, 1));
    });
}

#[test]
fn post_merge_review_prompt_and_transfer_cannot_overwrite_pending_review() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        commands_review::queue_review_spawn_remote(
            &mut app,
            "Review",
            "parent".into(),
            "original".into(),
        )
        .unwrap();
        app.route_next_prompt_to_new_session = true;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            let result = remote::route_prepared_input_to_new_remote_session(
                &mut app,
                &mut remote,
                input::PreparedInput {
                    raw_input: "keep draft".into(),
                    expanded: "keep draft".into(),
                    images: vec![],
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(app.input, "keep draft");
            assert!(app.route_next_prompt_to_new_session);
            assert_eq!(
                app.pending_split_startup_message.as_deref(),
                Some("original")
            );
            app.route_next_prompt_to_new_session = false;
            for command in [
                "/transfer",
                "/split",
                "/fork",
                "/workspace add",
                "/workspace add up",
            ] {
                app.input = command.into();
                app.cursor_pos = app.input.len();
                app.handle_remote_key(KeyCode::Enter, KeyModifiers::NONE, &mut remote)
                    .await
                    .unwrap();
                assert_eq!(app.pending_split_label.as_deref(), Some("Review"));
                assert_eq!(
                    app.pending_split_startup_message.as_deref(),
                    Some("original")
                );
                assert!(!app.pending_transfer_request);
            }
        });
    });
}
