use super::*;
use crate::protocol::ServerEvent;
use crate::tui::backend::RemoteConnection;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

async fn request<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> serde_json::Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("wire request timeout")
        .expect("wire read");
    serde_json::from_str(&line).expect("request JSON")
}

async fn no_request<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) {
    let mut line = String::new();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), reader.read_line(&mut line))
            .await
            .is_err(),
        "unexpected wire request: {line}"
    );
}

async fn submit(app: &mut App, remote: &mut RemoteConnection, command: &str, generic: bool) {
    if generic {
        remote::submit_remote_slash_input(
            app,
            remote,
            input::PreparedInput {
                raw_input: command.into(),
                expanded: command.into(),
                images: vec![],
            },
        )
        .await
        .unwrap();
    } else {
        app.input = command.into();
        app.cursor_pos = app.input.len();
        app.handle_remote_key(KeyCode::Enter, KeyModifiers::NONE, remote)
            .await
            .unwrap();
    }
}

fn remote_app() -> App {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_session_id = Some(app.session.id.clone());
    app.auto_poke_incomplete_todos = false;
    app.autoreview_enabled = false;
    app.autojudge_enabled = false;
    app
}

#[test]
fn post_merge_review_controls_use_wire_from_both_input_paths_idle_and_busy() {
    with_temp_jcode_home(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for generic in [false, true] {
            for busy in [false, true] {
                for feature in ["autoreview", "autojudge"] {
                    let mut app = remote_app();
                    app.is_processing = busy;
                    app.current_message_id = busy.then_some(42);
                    app.active_skill = Some("existing-skill".into());
                    rt.block_on(async {
                        let mut remote = RemoteConnection::dummy();
                        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
                        let mut reader = BufReader::new(peer);
                        for enabled in [true, false] {
                            let command =
                                format!("/{feature} {}", if enabled { "on" } else { "off" });
                            submit(&mut app, &mut remote, &command, generic).await;
                            let wire = request(&mut reader).await;
                            assert_eq!(wire["type"], "set_feature");
                            assert_eq!(wire["feature"], feature);
                            assert_eq!(wire["enabled"], enabled);
                            assert_eq!(app.is_processing, busy);
                            assert_eq!(app.current_message_id, busy.then_some(42));
                            assert_eq!(app.active_skill.as_deref(), Some("existing-skill"));
                            assert!(!app.pending_turn);
                            assert!(app.queued_messages.is_empty());
                        }
                    });
                }
            }
        }
    });
}

#[test]
fn post_merge_review_manual_launches_use_remote_split_from_both_paths() {
    with_temp_jcode_home(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for generic in [false, true] {
            for busy in [false, true] {
                for (command, label) in [
                    ("/review", "Review"),
                    ("/judge", "Judge"),
                    ("/autoreview now", "Autoreview"),
                    ("/autojudge now", "Autojudge"),
                ] {
                    let mut app = remote_app();
                    app.is_processing = busy;
                    app.current_message_id = busy.then_some(42);
                    rt.block_on(async {
                        let mut remote = RemoteConnection::dummy();
                        let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
                        let mut reader = BufReader::new(peer);
                        submit(&mut app, &mut remote, command, generic).await;
                        assert_eq!(app.pending_split_label.as_deref(), Some(label));
                        assert_eq!(app.pending_split_parent_session_id, app.remote_session_id);
                        assert!(app.pending_split_role_selection.is_some());
                        assert!(!app.pending_turn);
                        if busy {
                            assert!(app.pending_split_request);
                            assert_eq!(app.current_message_id, Some(42));
                            no_request(&mut reader).await;
                        } else {
                            assert_eq!(request(&mut reader).await["type"], "split");
                            assert!(!app.pending_split_request);
                        }
                    });
                }
            }
        }
    });
}

#[test]
fn post_merge_review_collision_preserves_first_launch_and_other_split_owners() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        commands_review::queue_review_spawn_remote(
            &mut app,
            "Review",
            "parent-a".into(),
            "first".into(),
        )
        .unwrap();
        assert!(
            commands_review::queue_review_spawn_remote(
                &mut app,
                "Judge",
                "parent-b".into(),
                "second".into()
            )
            .is_err()
        );
        assert_eq!(
            app.pending_split_parent_session_id.as_deref(),
            Some("parent-a")
        );
        assert_eq!(app.pending_split_startup_message.as_deref(), Some("first"));
        assert_eq!(app.pending_split_label.as_deref(), Some("Review"));
        // Once sent, the label/startup metadata still belongs to that handshake.
        app.pending_split_request = false;
        assert!(
            commands_review::queue_review_spawn_remote(
                &mut app,
                "Judge",
                "parent-b".into(),
                "second".into()
            )
            .is_err()
        );
        app.pending_split_label = None;
        app.pending_split_startup_message = None;
        app.pending_transfer_request = true;
        assert!(
            commands_review::queue_review_spawn_remote(
                &mut app,
                "Judge",
                "parent-b".into(),
                "second".into()
            )
            .is_err()
        );
    });
}

#[test]
fn post_merge_review_prefixes_do_not_claim_other_skill_names() {
    let mut app = create_test_app();
    assert!(!commands_review::handle_review_command_local(
        &mut app,
        "/reviewer"
    ));
    assert!(!commands_review::handle_judge_command_local(
        &mut app,
        "/judge-tools"
    ));
    assert!(!commands_review::handle_autoreview_command_local(
        &mut app,
        "/autoreviewer"
    ));
    assert!(!commands_review::handle_autojudge_command_local(
        &mut app,
        "/autojudgement"
    ));
}

#[test]
fn post_merge_review_invalid_controls_do_not_send_model_turns() {
    with_temp_jcode_home(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for generic in [false, true] {
            let mut app = remote_app();
            app.is_processing = true;
            app.current_message_id = Some(42);
            rt.block_on(async {
                let mut remote = RemoteConnection::dummy();
                let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
                let mut reader = BufReader::new(peer);
                for command in [
                    "/review extra",
                    "/judge now",
                    "/autoreview on extra",
                    "/autojudge bogus",
                ] {
                    submit(&mut app, &mut remote, command, generic).await;
                    assert!(
                        app.display_messages
                            .last()
                            .unwrap()
                            .content
                            .contains("Usage:")
                    );
                    assert_eq!(app.current_message_id, Some(42));
                    assert!(app.is_processing);
                    assert!(!app.pending_turn);
                }
                no_request(&mut reader).await;
            });
        }
    });
}

// Complete the transport handshake without opening a real terminal in a unit test.
fn finish_split_fixture(app: &mut App) {
    app.last_completed_split_request_id = app.pending_split_request_id.take();
    remote::finish_remote_split_launch(app);
    app.pending_split_label = None;
    app.pending_split_startup_message = None;
    app.pending_split_parent_session_id = None;
    app.pending_split_role_selection = None;
    app.pending_split_started_at = None;
}

#[test]
fn post_merge_review_done_schedules_both_roles_once_in_serial_order() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        app.autoreview_enabled = true;
        app.autojudge_enabled = true;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            remote.mark_history_loaded();
            let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
            let mut reader = BufReader::new(peer);
            let id = remote::begin_remote_send(
                &mut app,
                &mut remote,
                "implement".into(),
                vec![],
                false,
                None,
                false,
                0,
            )
            .await
            .unwrap();
            assert_eq!(request(&mut reader).await["type"], "message");
            app.handle_server_event(ServerEvent::Done { id: id + 90 }, &mut remote);
            remote::process_remote_followups(&mut app, &mut remote).await;
            no_request(&mut reader).await;
            assert!(app.is_processing);
            app.handle_server_event(ServerEvent::Done { id }, &mut remote);
            remote::process_remote_followups(&mut app, &mut remote).await;
            assert_eq!(request(&mut reader).await["type"], "split");
            assert_eq!(app.pending_split_label.as_deref(), Some("Autoreview"));
            // A duplicate main completion cannot consume the other role or launch twice.
            app.handle_server_event(ServerEvent::Done { id }, &mut remote);
            remote::process_remote_followups(&mut app, &mut remote).await;
            no_request(&mut reader).await;
            finish_split_fixture(&mut app);
            remote::process_remote_followups(&mut app, &mut remote).await;
            assert_eq!(request(&mut reader).await["type"], "split");
            assert_eq!(app.pending_split_label.as_deref(), Some("Autojudge"));
            finish_split_fixture(&mut app);
            remote::process_remote_followups(&mut app, &mut remote).await;
            no_request(&mut reader).await;
        });
    });
}

#[test]
fn post_merge_review_system_disabled_and_observer_turns_do_not_schedule() {
    with_temp_jcode_home(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for (system, enabled, observer) in [
            (true, true, false),
            (false, false, false),
            (false, true, true),
        ] {
            let mut app = remote_app();
            app.autoreview_enabled = enabled;
            app.autojudge_enabled = enabled;
            rt.block_on(async {
                let mut remote = RemoteConnection::dummy();
                remote.mark_history_loaded();
                let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
                let mut reader = BufReader::new(peer);
                let id = if observer {
                    app.is_processing = true;
                    app.current_message_id = Some(40);
                    40
                } else {
                    let id = remote::begin_remote_send(
                        &mut app,
                        &mut remote,
                        "turn".into(),
                        vec![],
                        system,
                        None,
                        false,
                        0,
                    )
                    .await
                    .unwrap();
                    request(&mut reader).await;
                    id
                };
                app.handle_server_event(ServerEvent::Done { id }, &mut remote);
                remote::process_remote_followups(&mut app, &mut remote).await;
                no_request(&mut reader).await;
            });
        }
    });
}

#[test]
fn post_merge_review_interruption_disarms_completed_turn_followups() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        app.autoreview_enabled = true;
        app.autojudge_enabled = true;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            remote.mark_history_loaded();
            let (peer, _writer) = remote.take_dummy_peer().unwrap().into_split();
            let mut reader = BufReader::new(peer);
            let id = remote::begin_remote_send(
                &mut app,
                &mut remote,
                "turn".into(),
                vec![],
                false,
                None,
                false,
                0,
            )
            .await
            .unwrap();
            request(&mut reader).await;
            app.handle_server_event(ServerEvent::Interrupted, &mut remote);
            app.handle_server_event(ServerEvent::Done { id }, &mut remote);
            remote::process_remote_followups(&mut app, &mut remote).await;
            assert!(!app.autoreview_after_current_turn);
            assert!(!app.autojudge_after_current_turn);
            no_request(&mut reader).await;
        });
    });
}

#[test]
fn post_merge_review_pending_queue_is_bounded_and_session_scoped() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        app.autoreview_enabled = true;
        app.autojudge_enabled = true;
        for _ in 0..20 {
            app.autoreview_after_current_turn = true;
            app.autojudge_after_current_turn = true;
            commands_review::queue_completed_turn_reviews(&mut app);
            assert_eq!(app.pending_automatic_reviews.len(), 2);
        }
        app.remote_session_id = Some("other-session".into());
        commands_review::stage_next_automatic_review(&mut app);
        assert!(!app.pending_split_request);
        assert!(app.pending_automatic_reviews.is_empty());
    });
}

#[test]
fn post_merge_review_disabled_role_is_skipped_without_overwriting_manual_launch() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        app.autoreview_enabled = true;
        app.autojudge_enabled = true;
        app.autoreview_after_current_turn = true;
        app.autojudge_after_current_turn = true;
        commands_review::queue_completed_turn_reviews(&mut app);
        commands_review::queue_review_spawn_remote(
            &mut app,
            "Review",
            "manual".into(),
            "manual prompt".into(),
        )
        .unwrap();
        commands_review::stage_next_automatic_review(&mut app);
        assert_eq!(
            app.pending_split_parent_session_id.as_deref(),
            Some("manual")
        );
        app.pending_split_request = false;
        finish_split_fixture(&mut app);
        app.autoreview_enabled = false;
        commands_review::stage_next_automatic_review(&mut app);
        assert_eq!(app.pending_split_label.as_deref(), Some("Autojudge"));
    });
}

#[test]
fn post_merge_review_queued_role_retains_model_and_effort_snapshot() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        let mut config = crate::config::Config::load();
        config.autoreview.model = Some("selected-review-model".into());
        config.autoreview.effort = Some("high".into());
        config.save().unwrap();
        app.autoreview_enabled = true;
        app.autoreview_after_current_turn = true;
        commands_review::queue_completed_turn_reviews(&mut app);
        config.autoreview.model = Some("new-default-for-next-turn".into());
        config.autoreview.effort = Some("low".into());
        config.save().unwrap();
        commands_review::stage_next_automatic_review(&mut app);
        let mut child = crate::session::Session::create(None, Some("autoreview".into()));
        child.save().unwrap();
        commands_review::prepare_review_spawned_session(
            &child.id,
            app.pending_split_startup_message.take().unwrap(),
            app.pending_split_role_selection.take(),
            Some("autoreview".into()),
            app.pending_split_parent_session_id.take(),
        )
        .unwrap();
        let saved = crate::session::Session::load(&child.id).unwrap();
        assert_eq!(saved.model.as_deref(), Some("selected-review-model"));
        assert_eq!(saved.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(saved.autoreview_enabled, Some(false));
        assert_eq!(saved.autojudge_enabled, Some(false));
        assert_ne!(app.session.model, saved.model);
    });
}

#[test]
fn post_merge_review_new_user_turn_discards_unsent_old_reviews() {
    with_temp_jcode_home(|| {
        let mut app = remote_app();
        app.autoreview_enabled = true;
        app.autoreview_after_current_turn = true;
        commands_review::queue_completed_turn_reviews(&mut app);
        assert_eq!(app.pending_automatic_reviews.len(), 1);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut remote = RemoteConnection::dummy();
            remote::begin_remote_send(
                &mut app,
                &mut remote,
                "new task".into(),
                vec![],
                false,
                None,
                false,
                0,
            )
            .await
            .unwrap();
            assert!(app.pending_automatic_reviews.is_empty());
            assert!(app.autoreview_after_current_turn);
        });
    });
}
