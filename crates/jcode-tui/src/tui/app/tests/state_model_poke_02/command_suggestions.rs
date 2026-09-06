#[test]
fn test_fuzzy_command_suggestions() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/mdl");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/model"));
}
#[test]
fn test_refresh_model_list_command_suggestions() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/refresh");
    assert!(
        suggestions
            .iter()
            .any(|(cmd, _)| cmd == "/refresh-model-list")
    );
    assert!(!suggestions.iter().any(|(cmd, _)| cmd == "/refresh-models"));

    let spaced = app.get_suggestions_for("/refresh ");
    assert!(spaced.is_empty());
}
#[test]
fn test_command_suggestion_arrow_and_ctrl_navigation_accepts_highlighted_row() {
    let mut app = create_test_app();
    app.input = "/con".to_string();
    app.cursor_pos = app.input.len();
    let suggestions = app.command_suggestions();
    assert!(suggestions.len() >= 2);

    app.handle_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 1);
    app.handle_key(KeyCode::Char('k'), KeyModifiers::CONTROL)
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 0);
    app.handle_key(KeyCode::Char('j'), KeyModifiers::CONTROL)
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 1);

    let expected = suggestions[1].0.clone();
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.input, expected);
    assert_eq!(app.cursor_pos, app.input.len());
}
#[test]
fn test_command_suggestion_navigation_moves_through_all_rows_and_allows_shift_arrow_noise() {
    let mut app = create_test_app();
    app.input = "/".to_string();
    app.cursor_pos = app.input.len();
    let suggestion_count = app.command_suggestions().len();
    assert!(suggestion_count > crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT);

    for expected in 1..=crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT {
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
        assert_eq!(app.command_suggestion_selected, expected);
    }

    app.handle_key(KeyCode::Down, KeyModifiers::SHIFT).unwrap();
    assert_eq!(
        app.command_suggestion_selected,
        crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT + 1
    );
    app.handle_key(KeyCode::Up, KeyModifiers::SHIFT).unwrap();
    assert_eq!(
        app.command_suggestion_selected,
        crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT
    );

    for _ in 0..suggestion_count {
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
    }
    assert_eq!(
        app.command_suggestion_selected,
        crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT
    );
}
#[test]
fn test_command_suggestion_render_highlights_selected_row_by_color() {
    let _lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.input = "/con".to_string();
    app.cursor_pos = app.input.len();
    let suggestions = app.command_suggestions();
    assert!(suggestions.len() >= 2);
    let first = suggestions[0].0.clone();
    let second = suggestions[1].0.clone();

    let selected_base = crate::tui::color_support::rgb(255, 213, 128);
    let unselected_base = crate::tui::color_support::rgb(128, 203, 196);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
        .expect("failed to create test terminal");
    render_and_snap(&app, &mut terminal);
    assert_command_match_recolored(&terminal, &first, selected_base);
    assert_command_match_recolored(&terminal, &second, unselected_base);

    app.handle_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    render_and_snap(&app, &mut terminal);
    assert_command_match_recolored(&terminal, &first, unselected_base);
    assert_command_match_recolored(&terminal, &second, selected_base);
}
#[test]
fn test_single_command_suggestion_uses_selected_color_only() {
    let _lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.input = "/review".to_string();
    app.cursor_pos = app.input.len();
    let suggestions = app.command_suggestions();
    assert_eq!(suggestions.len(), 1);
    let command = suggestions[0].0.clone();

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
        .expect("failed to create test terminal");
    render_and_snap(&app, &mut terminal);
    // A single suggestion still uses the selected-row base color; the fuzzy
    // match recoloring dims the '/' and brightens matched characters of it.
    assert_command_match_recolored(
        &terminal,
        &command,
        crate::tui::color_support::rgb(255, 213, 128),
    );
}
#[test]
fn test_command_suggestion_render_window_scrolls_with_selection() {
    let _lock = scroll_render_test_lock();
    let mut app = create_test_app();
    app.input = "/".to_string();
    app.cursor_pos = app.input.len();
    let suggestions = app.command_suggestions();
    let limit = crate::tui::app::COMMAND_SUGGESTION_VISIBLE_LIMIT;
    assert!(suggestions.len() > limit);
    let first = suggestions[0].0.clone();
    let selected_after_scroll = suggestions[limit].0.clone();

    for _ in 0..limit {
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
    }

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24))
        .expect("failed to create test terminal");
    let rendered = render_and_snap(&app, &mut terminal);
    assert!(
        !rendered.contains(&first),
        "the first suggestion should scroll out of the visible window:\n{rendered}"
    );
    assert!(
        rendered.contains(&selected_after_scroll),
        "the newly selected suggestion should be visible:\n{rendered}"
    );
    assert!(
        rendered.contains("↑"),
        "the scrolled window should indicate suggestions above:\n{rendered}"
    );
    assert_eq!(
        command_cell_fg(&terminal, &selected_after_scroll),
        Some(crate::tui::color_support::rgb(255, 213, 128))
    );
}
#[test]
fn test_remote_command_suggestion_arrow_and_ctrl_navigation_accepts_highlighted_row() {
    let mut app = create_test_app();
    app.input = "/con".to_string();
    app.cursor_pos = app.input.len();
    let suggestions = app.command_suggestions();
    assert!(suggestions.len() >= 2);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    rt.block_on(app.handle_remote_key(KeyCode::Down, KeyModifiers::empty(), &mut remote))
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 1);
    rt.block_on(app.handle_remote_key(KeyCode::Char('k'), KeyModifiers::CONTROL, &mut remote))
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 0);
    rt.block_on(app.handle_remote_key(KeyCode::Char('j'), KeyModifiers::CONTROL, &mut remote))
        .unwrap();
    assert_eq!(app.command_suggestion_selected, 1);

    let expected = suggestions[1].0.clone();
    rt.block_on(app.handle_remote_key(KeyCode::Enter, KeyModifiers::empty(), &mut remote))
        .unwrap();
    assert_eq!(app.input, expected);
    assert_eq!(app.cursor_pos, app.input.len());
}
#[test]
fn test_registered_command_suggestions_include_aliases_and_hide_secret_commands() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/");
    let commands: Vec<&str> = suggestions.iter().map(|(cmd, _)| cmd.as_str()).collect();

    assert_eq!(commands.iter().filter(|cmd| **cmd == "/cancel").count(), 1);
    assert!(commands.contains(&"/models"));
    assert!(commands.contains(&"/sessions"));
    assert!(commands.contains(&"/dictation"));
    assert!(commands.contains(&"/feedback"));
    assert!(commands.contains(&"/plan"));
    assert!(!commands.contains(&"/z"));
    assert!(!commands.contains(&"/zz"));
    assert!(!commands.contains(&"/zzz"));
}
#[test]
fn test_cancel_command_is_available_for_prefix_autocomplete() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/can");

    assert!(suggestions.iter().any(|(cmd, help)| {
        cmd == "/cancel" && *help == "Cancel the current prompt or operation"
    }));
}
#[test]
fn test_auth_doctor_command_suggestion_is_not_shadowed_by_provider_suggestions() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/auth d");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/auth doctor"));
}
#[test]
fn test_top_level_command_suggestions_include_config_and_subscription() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/con");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/config"));
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/context"));

    let suggestions = app.get_suggestions_for("/ali");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/alignment"));

    let suggestions = app.get_suggestions_for("/sub");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/subscription"));
}
#[test]
fn test_top_level_command_suggestions_include_project_local_skills() {
    let mut app = create_test_app();

    // Hermetic project-local skill: the suggestion list must surface skills
    // found under <working_dir>/.jcode/skills, independent of the skills
    // installed on the machine running the tests.
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp
        .path()
        .join(".jcode")
        .join("skills")
        .join("optimization");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: optimization\ndescription: Project-local test skill\n---\n# Optimization\n",
    )
    .expect("write SKILL.md");
    app.session.working_dir = Some(temp.path().to_string_lossy().to_string());
    app.refresh_skills_snapshot();

    let suggestions = app.get_suggestions_for("/optim");

    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/optimization"));
}
#[test]
fn test_top_level_command_suggestions_include_catchup_and_back() {
    let app = create_test_app();

    let suggestions = app.get_suggestions_for("/cat");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/catchup"));

    let suggestions = app.get_suggestions_for("/bac");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/back"));

    let suggestions = app.get_suggestions_for("/gi");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/git"));

    let suggestions = app.get_suggestions_for("/comm");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/commit"));

    let suggestions = app.get_suggestions_for("/tran");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/transcript"));
}
#[test]
fn test_top_level_command_suggestions_include_all_non_hidden_commands() {
    let app = create_test_app();

    let suggestions = app.get_suggestions_for("/logo");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/logout"));

    let suggestions = app.get_suggestions_for("/client");
    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/client-reload"));

    let suggestions = app.get_suggestions_for("/z");
    assert!(!suggestions.iter().any(|(cmd, _)| cmd == "/z"));
    assert!(!suggestions.iter().any(|(cmd, _)| cmd == "/zz"));
}
#[test]
fn test_logout_clear_anthropic_accounts_removes_all_accounts_once() {
    with_temp_jcode_home(|| {
        let mut created_labels = Vec::new();
        for index in 1..=3 {
            let label =
                crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
                    label: format!("requested-{index}"),
                    access: format!("access-{index}"),
                    refresh: format!("refresh-{index}"),
                    expires: 100 + index,
                    email: None,
                    subscription_type: None,
                    scopes: Vec::new(),
                })
                .unwrap();
            created_labels.push(label);
        }
        crate::auth::claude::set_active_account(&created_labels[2]).unwrap();

        let labels: Vec<_> = crate::auth::claude::list_accounts()
            .unwrap()
            .into_iter()
            .map(|account| account.label)
            .collect();
        assert_eq!(labels, created_labels);
        assert_eq!(
            crate::auth::claude::active_account_label().as_deref(),
            Some(labels[2].as_str())
        );

        assert_eq!(crate::auth::claude::clear_accounts().unwrap(), 3);
        assert!(crate::auth::claude::list_accounts().unwrap().is_empty());
        assert!(crate::auth::claude::active_account_label().is_none());
    });
}
#[test]
fn test_transcript_command_suggestions_include_path_variant() {
    let app = create_test_app();

    let suggestions = app.get_suggestions_for("/transcript p");

    assert!(suggestions.iter().any(|(cmd, _)| cmd == "/transcript path"));
}
#[test]
fn test_help_topic_suggestions_are_contextual() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/help fi");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/help fix")
    );
}
