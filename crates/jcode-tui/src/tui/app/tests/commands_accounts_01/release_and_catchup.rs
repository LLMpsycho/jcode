#[test]
fn test_help_topic_shows_btw_command_details() {
    let mut app = create_test_app();
    app.input = "/help btw".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/btw <question>"));
    assert!(msg.content.contains("Forks (splits) the session"));
}
#[test]
fn test_help_topic_shows_fork_command_details() {
    let mut app = create_test_app();
    app.input = "/help fork".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/fork <prompt>"));
    assert!(msg.content.contains("Alias for /fork"));
}
#[test]
fn test_help_topic_shows_git_command_details() {
    let mut app = create_test_app();
    app.input = "/help git".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/git"));
    assert!(msg.content.contains("git status --short --branch"));
    assert!(msg.content.contains("/git status"));
}
#[test]
fn test_help_topic_shows_commit_command_details() {
    let mut app = create_test_app();
    app.input = "/help commit".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/commit"));
    assert!(msg.content.contains("logical commits"));
    assert!(msg.content.contains("preserve unrelated work"));
}
#[test]
fn test_commit_command_starts_synthetic_user_turn() {
    let mut app = create_test_app();
    app.input = "/commit".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(notice.content.contains("Starting logical commits"));
}
#[test]
fn test_commit_push_command_starts_synthetic_user_turn() {
    let mut app = create_test_app();
    app.input = "/commit-push".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(notice.content.contains("Starting logical commits + push"));
}
#[test]
fn test_help_topic_shows_commit_push_command_details() {
    let mut app = create_test_app();
    app.input = "/help commit-push".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/commit-push"));
    assert!(msg.content.contains("push"));
}
#[test]
fn test_fast_release_command_starts_synthetic_user_turn() {
    let mut app = create_test_app();
    app.input = "/fast-release".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(
        notice
            .content
            .contains("Starting logical commits + push + fast local release")
    );
}
#[test]
fn test_triage_command_starts_synthetic_user_turn() {
    let mut app = create_test_app();
    app.input = "/triage".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(notice.content.contains("Starting GitHub issue triage"));
}
#[test]
fn test_triage_command_includes_focus_in_prompt() {
    let prompt = crate::tui::app::commands::build_triage_prompt(" only crash reports");
    assert!(prompt.contains("Triage the open GitHub issues"));
    assert!(prompt.contains("Additional focus from the user: only crash reports"));
}
#[test]
fn test_cut_release_alias_starts_fast_release_turn() {
    let mut app = create_test_app();
    app.input = "/cut-release".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert!(notice.content.contains("fast local release"));
}
#[test]
fn test_fast_release_prompt_uses_selfdev_cache() {
    let fast_prompt = super::commands::build_fast_release_prompt();
    assert!(fast_prompt.contains("quick-release.sh --prepare-fast"));
    assert!(fast_prompt.contains("quick-release.sh --fast-local"));
    assert!(fast_prompt.contains("warm target/selfdev cache"));
    assert!(fast_prompt.contains("Do not run the separate local macOS cross-build"));
    let prepare = fast_prompt.find("--prepare-fast").unwrap();
    let bump = fast_prompt.find("Bump the version").unwrap();
    assert!(prepare < bump);
}
#[test]
fn test_fast_macos_release_command_uses_prepared_cross_build() {
    let mut app = create_test_app();
    app.input = "/fast-macos-release".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert!(notice.content.contains("fast macOS release"));

    let prompt = super::commands::build_fast_macos_release_prompt();
    assert!(prompt.contains("quick-release.sh --prepare-fast-macos"));
    assert!(prompt.contains("quick-release.sh --fast-macos-local"));
    assert!(prompt.contains("macOS arm64"));
    let prepare = prompt.find("--prepare-fast-macos").unwrap();
    let bump = prompt.find("Bump the version").unwrap();
    assert!(prepare < bump);
}
#[test]
fn test_help_topic_shows_fast_macos_release_details() {
    let mut app = create_test_app();
    app.input = "/help fast-macos-release".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/fast-macos-release"));
    assert!(msg.content.contains("--prepare-fast-macos"));
    assert!(msg.content.contains("--fast-macos-local"));
    assert!(msg.content.contains("osxcross"));
}
#[test]
fn test_remote_release_command_uses_tag_only_ci_path() {
    let mut app = create_test_app();
    app.input = "/remote-release".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(
        notice
            .content
            .contains("Starting logical commits + push + remote release")
    );

    let prompt = super::commands::build_remote_release_prompt();
    assert!(prompt.contains("quick-release.sh --remote"));
    assert!(prompt.contains("without any local build"));
    assert!(prompt.contains("publication gated"));
    assert!(prompt.contains("Only use the following Jcode-specific procedure"));
    assert!(prompt.contains("repository's own established release conventions"));
    assert!(prompt.contains("Do not assume the project uses Cargo"));
    assert!(prompt.contains("tag-triggered or workflow-dispatch CI release"));
}
#[test]
fn test_commit_push_release_alias_starts_synthetic_user_turn() {
    let mut app = create_test_app();
    app.input = "/commit-push-release".to_string();
    app.submit_input();

    assert!(app.is_processing);
    assert!(app.pending_turn);
    let notice = app
        .display_messages()
        .last()
        .expect("missing launch notice");
    assert_eq!(notice.role, "system");
    assert!(
        notice
            .content
            .contains("Starting logical commits + push + fast local release")
    );
}
#[test]
fn test_help_topic_shows_cut_release_command_details() {
    let mut app = create_test_app();
    app.input = "/help cut-release".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/fast-release"));
    assert!(msg.content.contains("--prepare-fast"));
    assert!(msg.content.contains("--fast-local"));
    assert!(msg.content.contains("target/selfdev"));
    assert!(msg.content.contains("compatibility alias"));
}
#[test]
fn test_help_topic_shows_remote_release_command_details() {
    let mut app = create_test_app();
    app.input = "/help remote-release".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/remote-release"));
    assert!(msg.content.contains("--remote"));
    assert!(msg.content.contains("without running any local build"));
    assert!(msg.content.contains("remains a draft"));
}
#[test]
fn test_help_topic_shows_catchup_command_details() {
    let mut app = create_test_app();
    app.input = "/help catchup".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/catchup"));
    assert!(msg.content.contains("side panel"));
    assert!(msg.content.contains("/catchup next"));
}
#[test]
fn test_help_topic_shows_back_command_details() {
    let mut app = create_test_app();
    app.input = "/help back".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/back"));
    assert!(msg.content.contains("Catch Up"));
}
#[test]
fn test_catchup_next_queues_resume_for_attention_session() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_session_id = Some(app.session.id.clone());

        let mut target = Session::create(None, Some("catchup target".to_string()));
        target.add_message(
            crate::message::Role::User,
            vec![crate::message::ContentBlock::Text {
                text: "Review the implementation and summarize what changed.".to_string(),
                cache_control: None,
            }],
        );
        target.add_message(
            crate::message::Role::Assistant,
            vec![crate::message::ContentBlock::Text {
                text: "I finished the work and need your decision on the next step.".to_string(),
                cache_control: None,
            }],
        );
        target.mark_closed();
        target.save().expect("save catchup target");

        app.input = "/catchup next".to_string();
        app.submit_input();

        let pending = app
            .pending_catchup_resume
            .clone()
            .expect("missing pending catchup resume");
        assert_eq!(pending.target_session_id, target.id);
        assert_eq!(pending.source_session_id, app.remote_session_id);
        assert_eq!(pending.queue_position, Some((1, 1)));
        assert!(pending.show_brief);

        let msg = app
            .display_messages()
            .last()
            .expect("missing catchup queued message");
        assert_eq!(msg.role, "system");
        assert!(msg.content.contains("Queued Catch Up"));
    });
}
#[test]
fn test_back_command_queues_return_without_showing_brief() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.catchup_return_stack.push("session_prev".to_string());

    app.input = "/back".to_string();
    app.submit_input();

    let pending = app
        .pending_catchup_resume
        .clone()
        .expect("missing pending back resume");
    assert_eq!(pending.target_session_id, "session_prev");
    assert_eq!(pending.source_session_id, None);
    assert_eq!(pending.queue_position, None);
    assert!(!pending.show_brief);
}
#[test]
fn test_maybe_show_catchup_after_history_adds_brief_page_and_marks_seen() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.side_panel = test_side_panel_snapshot("plan", "Plan");

        let source_session_id = app.session.id.clone();
        let mut target = Session::create(None, Some("catchup brief".to_string()));
        target.add_message(
            crate::message::Role::User,
            vec![crate::message::ContentBlock::Text {
                text: "Please review the final diff.".to_string(),
                cache_control: None,
            }],
        );
        target.add_message(
            crate::message::Role::Assistant,
            vec![crate::message::ContentBlock::Text {
                text: "The implementation is complete and needs your approval.".to_string(),
                cache_control: None,
            }],
        );
        target.mark_closed();
        target.save().expect("save catchup brief session");
        let target_id = target.id.clone();

        app.begin_in_flight_catchup_resume(PendingCatchupResume {
            target_session_id: target_id.clone(),
            source_session_id: Some(source_session_id),
            queue_position: Some((1, 1)),
            show_brief: true,
        });
        app.maybe_show_catchup_after_history(&target_id);

        assert!(app.in_flight_catchup_resume.is_none());
        assert_eq!(app.side_panel.focused_page_id.as_deref(), Some("catchup"));
        assert_eq!(app.side_panel.pages.len(), 2);
        assert!(app.side_panel.pages.iter().any(|page| page.id == "plan"));

        let page = app.side_panel.focused_page().expect("missing catchup page");
        assert_eq!(page.id, "catchup");
        assert_eq!(page.file_path, format!("catchup://{}", target_id));
        assert!(page.content.contains("# Catch Up"));
        assert!(page.content.contains("Please review the final diff."));
        assert!(page.content.contains("needs your approval"));

        let persisted = Session::load(&target_id).expect("reload catchup target");
        assert!(!crate::catchup::needs_catchup(
            &target_id,
            persisted.updated_at,
            &persisted.status
        ));
    });
}
#[test]
fn test_help_topic_shows_observe_command_details() {
    let mut app = create_test_app();
    app.input = "/help observe".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/observe"));
    assert!(msg.content.contains("latest tool call or tool result"));
}
#[test]
fn test_help_topic_shows_splitview_command_details() {
    let mut app = create_test_app();
    app.input = "/help splitview".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/splitview"));
    assert!(
        msg.content
            .contains("mirrors the current chat in the side panel")
    );
}
#[test]
fn test_help_topic_shows_refactor_command_details() {
    let mut app = create_test_app();
    app.input = "/help refactor".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/refactor [focus]"));
    assert!(msg.content.contains("independent read-only subagent"));
}
