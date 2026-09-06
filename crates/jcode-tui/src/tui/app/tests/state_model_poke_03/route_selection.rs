#[test]
fn test_local_model_picker_surfaces_antigravity_models_from_multiprovider() {
    let mut app = create_antigravity_picker_test_app();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should be open");

    let antigravity_entry = picker
        .entries
        .iter()
        .find(|entry| entry.name == "claude-sonnet-4-6")
        .expect("antigravity model should be shown after login");

    assert!(antigravity_entry.options.iter().any(|route| {
        route.provider == "Antigravity" && route.api_method == "cli" && route.available
    }));
}
#[test]
fn test_local_antigravity_model_picker_selection_preserves_antigravity_provider() {
    let mut app = create_antigravity_picker_test_app();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should be open");

    let model_idx = picker
        .entries
        .iter()
        .position(|entry| entry.name == "claude-sonnet-4-6")
        .expect("antigravity model should be in picker");
    let filtered_pos = picker
        .filtered
        .iter()
        .position(|&i| i == model_idx)
        .expect("antigravity model should be in filtered list");

    app.inline_interactive_state.as_mut().unwrap().selected = filtered_pos;
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.provider.name(), "Antigravity");
    assert_eq!(app.provider.model(), "claude-sonnet-4-6");
    assert!(app.inline_interactive_state.is_none());
}
#[test]
fn test_local_model_picker_openrouter_bare_openai_route_uses_openai_catalog_prefix() {
    let (mut app, set_model_calls) = create_openrouter_spec_capture_test_app();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should be open");
    let model_idx = picker
        .entries
        .iter()
        .position(|entry| entry.name == "gpt-5.4 (high)")
        .expect("openrouter-backed OpenAI effort entry should be in picker");
    let filtered_pos = picker
        .filtered
        .iter()
        .position(|&i| i == model_idx)
        .expect("entry should be in filtered list");

    app.inline_interactive_state.as_mut().unwrap().selected = filtered_pos;
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("model picker selection should succeed");

    assert_eq!(
        set_model_calls.lock().unwrap().as_slice(),
        ["openai/gpt-5.4@OpenAI"]
    );
}
#[test]
fn test_agent_model_picker_openrouter_bare_openai_route_saves_openai_catalog_prefix() {
    with_temp_jcode_home(|| {
        let (mut app, _set_model_calls) = create_openrouter_spec_capture_test_app();

        app.open_agent_model_picker(crate::tui::AgentModelTarget::Swarm);
        wait_for_model_picker_load(&mut app);

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("agent model picker should be open");
        let model_idx = picker
            .entries
            .iter()
            .position(|entry| entry.name == "gpt-5.4 (high)")
            .expect("openrouter-backed OpenAI effort entry should be in picker");
        let filtered_pos = picker
            .filtered
            .iter()
            .position(|&i| i == model_idx)
            .expect("entry should be in filtered list");

        app.inline_interactive_state.as_mut().unwrap().selected = filtered_pos;
        app.handle_key(KeyCode::Enter, KeyModifiers::empty())
            .expect("agent model picker selection should succeed");

        let last = app.display_messages.last().expect("display message");
        assert_eq!(last.role, "system");
        assert!(
            last.content.contains("openai/gpt-5.4@OpenAI"),
            "message should show normalized saved spec, got: {}",
            last.content
        );
    });
}
#[test]
fn test_local_model_picker_render_shows_antigravity_models_exactly_as_user_sees_them() {
    let mut app = create_antigravity_picker_test_app();
    app.display_messages = vec![DisplayMessage::system("seed render state")];
    app.bump_display_messages_version();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let render_filtered = |app: &mut App, filter: &str| {
        let picker = app
            .inline_interactive_state
            .as_mut()
            .expect("model picker should be open");
        picker.filter = filter.to_string();
        App::apply_inline_interactive_filter(picker);
        let _render_lock = scroll_render_test_lock();
        let backend = ratatui::backend::TestBackend::new(90, 14);
        let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");
        render_and_snap(app, &mut terminal)
    };
    let claude_text = render_filtered(&mut app, "claude-sonnet-4-6");
    let gpt_text = render_filtered(&mut app, "gpt-oss-120b-medium");

    assert!(
        claude_text.contains("MODEL")
            && claude_text.contains("PROVIDER")
            && claude_text.contains("METHOD"),
        "rendered /model view should include picker columns, got:
{}",
        claude_text
    );
    assert!(
        claude_text.contains("Claude Sonnet 4.6"),
        "rendered /model view should show the Antigravity Claude row, got:
{}",
        claude_text
    );
    assert!(
        gpt_text.contains("gpt-oss-120b-medium"),
        "rendered /model view should show the Antigravity GPT row, got:
{}",
        gpt_text
    );
    assert!(
        claude_text.contains("Antigravity") && gpt_text.contains("Antigravity"),
        "rendered /model view should show the Antigravity provider column, got:
Claude:
{}
GPT:
{}",
        claude_text,
        gpt_text
    );
    assert!(
        claude_text.contains("cli") && gpt_text.contains("cli"),
        "rendered /model view should show the route transport column, got:
Claude:
{}
GPT:
{}",
        claude_text,
        gpt_text
    );
}
#[test]
fn test_login_smoke_model_picker_renders_unstacked_provider_rows() {
    let mut app = create_login_smoke_model_app();
    app.display_messages = vec![DisplayMessage::system("seed render state")];
    app.bump_display_messages_version();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let render_filtered = |app: &mut App, filter: &str| {
        let picker = app
            .inline_interactive_state
            .as_mut()
            .expect("model picker should be open");
        picker.filter = filter.to_string();
        App::apply_inline_interactive_filter(picker);
        let _render_lock = scroll_render_test_lock();
        let backend = ratatui::backend::TestBackend::new(180, 48);
        let mut terminal = ratatui::Terminal::new(backend).expect("failed to create test terminal");
        render_and_snap(app, &mut terminal)
    };

    // Effort-capable routes now expand into multiple rows. Render focused
    // slices so each provider remains observable without assuming the complete
    // catalog fits in one terminal viewport.
    let openai_text = render_filtered(&mut app, "gpt-5.4");
    let comtegra_text = render_filtered(&mut app, "glm-51-nvfp4");
    let copilot_text = render_filtered(&mut app, "claude-opus-4.6");
    let deepseek_text = render_filtered(&mut app, "deepseek/deepseek-v4-pro");
    let kimi_text = render_filtered(&mut app, "moonshotai/kimi-k2.5");
    let openrouter_openai_text = render_filtered(&mut app, "openai/gpt-5.5");

    assert!(
        openai_text.contains("MODEL")
            && openai_text.contains("PROVIDER")
            && openai_text.contains("METHOD"),
        "rendered /model view should include user-visible picker columns, got:\n{}",
        openai_text
    );
    assert!(
        openai_text.contains("GPT-5.4")
            && openai_text.contains("OpenAI")
            && openai_text.contains("oauth")
            && openai_text.contains("api key"),
        "OpenAI OAuth and API-key routes should be separately visible, got:\n{}",
        openai_text
    );
    let glm_row = comtegra_text
        .lines()
        .find(|line| line.contains("glm-51-nvfp4"))
        .unwrap_or("");
    assert!(
        glm_row.contains("Comtegra GPU Cloud")
            && glm_row.contains("api key")
            && !glm_row.contains("copilot"),
        "Comtegra GLM row should show its provider and API-key method, got row `{}` in:\n{}",
        glm_row,
        comtegra_text
    );
    assert!(
        comtegra_text.contains("glm-51-nvfp4")
            && comtegra_text.contains("Comtegra GPU Cloud")
            && comtegra_text.contains("new"),
        "Comtegra login route should be visible and marked new, got:\n{}",
        comtegra_text
    );
    assert!(
        copilot_text.contains("Claude Opus 4.6") && copilot_text.contains("Copilot"),
        "Copilot route should be visible, got:\n{}",
        copilot_text
    );
    assert!(
        deepseek_text.contains("deepseek/deepseek-v4-pro") && deepseek_text.contains("openrouter"),
        "OpenRouter route should be visible, got:\n{}",
        deepseek_text
    );
    let deepseek_auto_row = deepseek_text
        .lines()
        .find(|line| line.contains("deepseek/deepseek-v4-pro") && line.contains("auto"))
        .unwrap_or("");
    let deepseek_provider_row = deepseek_text
        .lines()
        .find(|line| line.contains("deepseek/deepseek-v4-pro") && line.contains("DeepSeek"))
        .unwrap_or("");
    assert!(
        !deepseek_auto_row.contains('★'),
        "OpenRouter auto route should not carry the recommended marker, got row `{}` in:\n{}",
        deepseek_auto_row,
        deepseek_text
    );
    assert!(
        !deepseek_provider_row.contains('★'),
        "OpenRouter provider-specific routes should not carry the recommended marker, got row `{}` in:\n{}",
        deepseek_provider_row,
        deepseek_text
    );
    let kimi25_row = kimi_text
        .lines()
        .find(|line| line.contains("moonshotai/kimi-k2.5"))
        .unwrap_or("");
    assert!(
        !kimi25_row.contains('★'),
        "Kimi K2.5 should not be recommended, got row `{}` in:\n{}",
        kimi25_row,
        kimi_text
    );
    let openrouter_openai_row = openrouter_openai_text
        .lines()
        .find(|line| line.contains("openai/gpt-5.5"))
        .unwrap_or("");
    assert!(
        openrouter_openai_row.contains("OpenRou")
            && openrouter_openai_row.contains("openrouter")
            && !openrouter_openai_row.contains("api key"),
        "OpenRouter endpoint routes should not look like native OpenAI API-key rows, got row `{}` in:\n{}",
        openrouter_openai_row,
        openrouter_openai_text
    );
    for text in [
        &openai_text,
        &comtegra_text,
        &copilot_text,
        &deepseek_text,
        &kimi_text,
        &openrouter_openai_text,
    ] {
        assert!(
            !text.contains("(2)"),
            "provider routes should not be hidden behind stacked option counts, got:\n{}",
            text
        );
    }
}
#[test]
fn test_model_picker_filter_text_includes_provider_and_method() {
    let entry = crate::tui::PickerEntry {
        name: "glm-51-nvfp4".to_string(),
        options: vec![crate::tui::PickerOption {
            provider: "Comtegra GPU Cloud".to_string(),
            api_method: "openai-compatible:comtegra".to_string(),
            available: true,
            detail: "https://llm.comtegra.cloud/v1".to_string(),
            estimated_reference_cost_micros: None,
        }],
        action: crate::tui::PickerAction::Model,
        selected_option: 0,
        is_current: false,
        is_default: false,
        is_favorite: false,
        recommended: false,
        recommendation_rank: usize::MAX,
        usage_score: 0,
        old: false,
        created_date: None,
        effort: None,
    };

    let filter_text = crate::tui::PickerKind::Model.filter_text(&entry);
    assert!(filter_text.contains("glm-51-nvfp4"));
    assert!(filter_text.contains("Comtegra GPU Cloud"));
    assert!(filter_text.contains("openai-compatible:comtegra"));
}
