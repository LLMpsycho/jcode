#[test]
fn test_tui_api_key_auth_refreshes_catalog_shows_diff_without_opening_picker() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let provider = AuthUxStateSpaceProvider {
        authed: StdArc::new(AtomicBool::new(false)),
        refreshes: StdArc::new(AtomicUsize::new(0)),
        model: StdArc::new(StdMutex::new("pre-auth-model".to_string())),
        set_model_requests: StdArc::new(StdMutex::new(Vec::new())),
        provider_id: "state-space",
        provider_label: "StateSpace",
        models: &["state-space-alpha", "state-space-beta"],
        include_wrong_profile_first: true,
        include_generic_profile_duplicate: false,
    };
    let refreshes = provider.refreshes.clone();
    let provider: Arc<dyn Provider> = Arc::new(provider);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    let mut bus_rx = crate::bus::Bus::global().subscribe();
    while bus_rx.try_recv().is_ok() {}

    let _guard = rt.enter();
    app.start_openai_compatible_post_login_activation(
        "state-space".to_string(),
        "StateSpace".to_string(),
    );
    assert_eq!(
        app.status_notice(),
        Some("StateSpace: fetching models...".to_string())
    );
    assert!(
        app.inline_interactive_state.is_none(),
        "auth-triggered discovery should not open /model automatically"
    );

    let activation = rt.block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), bus_rx.recv()).await {
                Ok(Ok(event @ crate::bus::BusEvent::ProviderModelActivated { .. })) => break event,
                Ok(Ok(_)) => continue,
                other => panic!("expected ProviderModelActivated event, got {other:?}"),
            }
        }
    });
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "auth completion must refresh the model catalog exactly once"
    );

    super::local::handle_bus_event(&mut app, Ok(activation));
    assert!(
        app.inline_interactive_state.is_none(),
        "activation completion should still not open /model automatically"
    );
    assert_eq!(app.session.model.as_deref(), Some("state-space-alpha"));
    let last = app.display_messages.last().expect("activation message");
    assert!(last.content.contains("Added models:"));
    assert!(last.content.contains("state-space-alpha"));
    assert!(last.content.contains("state-space-beta"));
    assert!(last.content.contains("Use /model"));
    assert!(!last.content.contains("model picker is open"));

    assert!(super::model_context::handle_model_command(
        &mut app,
        "/model state-space-beta"
    ));
    assert_eq!(app.session.model.as_deref(), Some("state-space-beta"));
    assert_eq!(
        app.status_notice(),
        Some("Model → state-space-beta".to_string())
    );
}
#[test]
fn test_tui_cerebras_paste_key_lifecycle_has_no_degraded_success_messages() {
    let _env_lock = crate::storage::lock_test_env();
    let _guard = AzureLoginEnvGuard::save(&[
        "CEREBRAS_API_KEY",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_AUTH_HEADER",
        "JCODE_OPENROUTER_DYNAMIC_BEARER_PROVIDER",
        "JCODE_RUNTIME_PROVIDER",
        "JCODE_ACTIVE_PROVIDER",
        "JCODE_INITIAL_PROVIDER_EXPLICIT",
    ]);
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let fake_provider = AuthUxStateSpaceProvider {
        authed: StdArc::new(AtomicBool::new(false)),
        refreshes: StdArc::new(AtomicUsize::new(0)),
        model: StdArc::new(StdMutex::new("gpt-5.5".to_string())),
        set_model_requests: StdArc::new(StdMutex::new(Vec::new())),
        provider_id: "cerebras",
        provider_label: "Cerebras",
        models: &["qwen-3-235b-a22b-instruct-2507", "llama3.1-8b"],
        include_wrong_profile_first: true,
        include_generic_profile_duplicate: true,
    };
    let refreshes = fake_provider.refreshes.clone();
    let set_model_requests = fake_provider.set_model_requests.clone();
    let provider: Arc<dyn Provider> = Arc::new(fake_provider);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    let mut bus_rx = crate::bus::Bus::global().subscribe();
    while bus_rx.try_recv().is_ok() {}

    app.start_login_provider(
        crate::provider_catalog::resolve_login_provider("cerebras")
            .expect("Cerebras login provider"),
    );

    let prompt = app
        .display_messages
        .last()
        .expect("login prompt")
        .content
        .clone();
    assert!(prompt.contains("Cerebras API Key"), "{prompt}");
    assert!(
        prompt.contains("Stored variable: CEREBRAS_API_KEY"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Endpoint: https://api.cerebras.ai/v1"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Suggested default model: gpt-oss-120b"),
        "{prompt}"
    );
    assert!(prompt.contains("Paste your API key below"), "{prompt}");

    let pending = app
        .pending_login
        .take()
        .expect("pending Cerebras key login");
    let _runtime_guard = rt.enter();
    app.handle_login_input(pending, "test-cerebras-key".to_string());

    let mut saw_saved = false;
    let mut saw_catalog_started = false;
    let mut saw_activation = false;
    let mut saw_catalog_ready = false;
    let mut login_success_events = 0;
    let mut login_failure_events = 0;
    let mut catalog_warning_events = 0;
    let mut activation_events = 0;
    rt.block_on(async {
        while !(saw_saved && saw_catalog_started && saw_activation && saw_catalog_ready) {
            match tokio::time::timeout(Duration::from_secs(2), bus_rx.recv()).await {
                Ok(Ok(crate::bus::BusEvent::LoginCompleted(login))) => {
                    if login.success {
                        login_success_events += 1;
                    } else {
                        login_failure_events += 1;
                    }
                    assert!(login.success, "unexpected failed login event: {login:?}");
                    assert_eq!(login.provider, "Cerebras");
                    assert!(login.message.contains("Cerebras API key saved."));
                    let expected_path = crate::storage::app_config_dir()
                        .unwrap()
                        .join("cerebras.env");
                    assert!(
                        login
                            .message
                            .contains(&format!("Stored at {}.", expected_path.display())),
                        "{}",
                        login.message
                    );
                    assert!(login.message.contains("Fetching models now."));
                    assert!(!login.message.contains("did not switch models"));
                    app.handle_login_completed(login);
                    saw_saved = true;
                }
                Ok(Ok(crate::bus::BusEvent::UiActivity(activity))) => {
                    if activity.message.contains("Auth Model Catalog Warning") {
                        catalog_warning_events += 1;
                    }
                    assert!(
                        !activity.message.contains("Auth Model Catalog Warning"),
                        "unexpected warning activity: {}",
                        activity.message
                    );
                    assert!(
                        !activity.message.contains("did not switch models"),
                        "unexpected degraded activity: {}",
                        activity.message
                    );
                    if activity.message.contains("Model Discovery Started") {
                        saw_catalog_started = true;
                    }
                    super::local::handle_bus_event(
                        &mut app,
                        Ok(crate::bus::BusEvent::UiActivity(activity)),
                    );
                }
                Ok(Ok(event @ crate::bus::BusEvent::ProviderModelActivated { .. })) => {
                    activation_events += 1;
                    if let crate::bus::BusEvent::ProviderModelActivated {
                        model,
                        provider_key,
                        message,
                        ..
                    } = &event
                    {
                        assert_eq!(model, "qwen-3-235b-a22b-instruct-2507");
                        assert_eq!(provider_key.as_deref(), Some("cerebras"));
                        assert!(message.contains("Cerebras is ready."), "{message}");
                        assert!(!message.contains("wrong-profile-first"), "{message}");
                    }
                    super::local::handle_bus_event(&mut app, Ok(event));
                    saw_activation = true;
                }
                Ok(Ok(event @ crate::bus::BusEvent::AuthCatalogRefreshReady)) => {
                    super::local::handle_bus_event(&mut app, Ok(event));
                    saw_catalog_ready = true;
                }
                Ok(Ok(_)) => {}
                other => panic!("expected local Cerebras auth lifecycle event, got {other:?}"),
            }
        }
    });

    while let Ok(event) = bus_rx.try_recv() {
        match event {
            crate::bus::BusEvent::LoginCompleted(login) => {
                if login.success {
                    login_success_events += 1;
                } else {
                    panic!("late failed login event after successful auth: {login:?}");
                }
            }
            crate::bus::BusEvent::UiActivity(activity) => {
                if activity.message.contains("Auth Model Catalog Warning") {
                    panic!(
                        "late warning activity after successful auth: {}",
                        activity.message
                    );
                }
                assert!(
                    !activity.message.contains("did not switch models"),
                    "late degraded activity after successful auth: {}",
                    activity.message
                );
            }
            crate::bus::BusEvent::ProviderModelActivated {
                model,
                provider_key,
                message,
                ..
            } => {
                activation_events += 1;
                assert_eq!(model, "qwen-3-235b-a22b-instruct-2507");
                assert_eq!(provider_key.as_deref(), Some("cerebras"));
                assert!(message.contains("Cerebras is ready."), "{message}");
            }
            event @ crate::bus::BusEvent::AuthCatalogRefreshReady => {
                super::local::handle_bus_event(&mut app, Ok(event));
            }
            _ => {}
        }
    }

    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        login_success_events, 1,
        "expected exactly one successful login event"
    );
    assert_eq!(
        login_failure_events, 0,
        "happy auth must not publish failed login events"
    );
    assert_eq!(
        catalog_warning_events, 0,
        "happy auth must not publish catalog warnings"
    );
    assert_eq!(
        activation_events, 1,
        "expected exactly one provider activation event"
    );
    assert_eq!(
        app.session.model.as_deref(),
        Some("qwen-3-235b-a22b-instruct-2507")
    );
    assert_eq!(app.session.provider_key.as_deref(), Some("cerebras"));
    assert_eq!(
        set_model_requests.lock().unwrap().as_slice(),
        ["cerebras:qwen-3-235b-a22b-instruct-2507"],
        "post-login activation must preserve the authenticated Cerebras route instead of switching a bare model"
    );
    let transcript = app
        .display_messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    for forbidden in [
        "Auth Model Catalog Warning",
        "did not switch models",
        "contained no selectable",
        "Saved the API key and fetched the model catalog, but",
        "Login: Cerebras failed",
        "wrong-profile-first",
    ] {
        assert!(
            !transcript.contains(forbidden),
            "transcript contained forbidden degraded-success marker `{forbidden}`:\n{transcript}"
        );
    }

    set_model_requests.lock().unwrap().clear();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should open after Cerebras auth");
    let qwen_entry = picker
        .entries
        .iter()
        .find(|entry| entry.name == "qwen-3-235b-a22b-instruct-2507")
        .expect("selected Cerebras model should be visible in /model");
    assert_eq!(
        picker
            .entries
            .iter()
            .filter(|entry| entry.name == "qwen-3-235b-a22b-instruct-2507")
            .count(),
        1,
        "Cerebras model picker should not show duplicate rows for the selected model"
    );
    assert!(qwen_entry.options.iter().any(|route| {
        route.provider == "Cerebras"
            && route.api_method == "openai-compatible:cerebras"
            && route.available
    }));
    assert!(
        !qwen_entry
            .options
            .iter()
            .any(|route| route.api_method == "openai-compatible"),
        "generic direct route should be de-duplicated in favor of the Cerebras profile route"
    );
    let llama_idx = picker
        .entries
        .iter()
        .position(|entry| entry.name == "llama3.1-8b")
        .expect("alternate Cerebras model should be visible in /model");
    let llama_entry = &picker.entries[llama_idx];
    assert_eq!(
        picker
            .entries
            .iter()
            .filter(|entry| entry.name == "llama3.1-8b")
            .count(),
        1,
        "Cerebras model picker should not show duplicate rows for alternate models"
    );
    assert!(llama_entry.options.iter().any(|route| {
        route.provider == "Cerebras"
            && route.api_method == "openai-compatible:cerebras"
            && route.available
    }));
    let llama_cerebras_option = llama_entry
        .options
        .iter()
        .position(|route| {
            route.provider == "Cerebras"
                && route.api_method == "openai-compatible:cerebras"
                && route.available
        })
        .expect("alternate model should expose its authenticated Cerebras route");
    assert!(
        !llama_entry
            .options
            .iter()
            .any(|route| route.api_method == "openai-compatible"),
        "generic direct route should be de-duplicated in favor of the Cerebras profile route"
    );
    let filtered_pos = picker
        .filtered
        .iter()
        .position(|&idx| idx == llama_idx)
        .expect("alternate Cerebras model should be selectable in filtered picker list");

    let picker = app.inline_interactive_state.as_mut().unwrap();
    picker.selected = filtered_pos;
    picker.entries[llama_idx].selected_option = llama_cerebras_option;
    picker.column = picker.max_navigable_column();
    app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("Cerebras picker selection should switch models");

    assert_eq!(app.session.model.as_deref(), Some("llama3.1-8b"));
    assert_eq!(app.session.provider_key.as_deref(), Some("cerebras"));
    assert_eq!(app.provider.model(), "llama3.1-8b");
    assert_eq!(
        set_model_requests.lock().unwrap().as_slice(),
        ["cerebras:llama3.1-8b"],
        "model picker must route post-auth switches through the authenticated Cerebras profile"
    );
}
#[test]
fn test_tui_openai_compatible_empty_catalog_does_not_switch_to_profile_default() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let refreshes = StdArc::new(AtomicUsize::new(0));
    let set_model_attempts = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(EmptyPostLoginCatalogProvider {
        refreshes: StdArc::clone(&refreshes),
        set_model_attempts: StdArc::clone(&set_model_attempts),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    let mut bus_rx = crate::bus::Bus::global().subscribe();
    while bus_rx.try_recv().is_ok() {}

    let _guard = rt.enter();
    app.start_openai_compatible_post_login_activation(
        "cerebras".to_string(),
        "Cerebras".to_string(),
    );

    let activity = rt.block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), bus_rx.recv()).await {
                Ok(Ok(crate::bus::BusEvent::ProviderModelActivated { .. })) => {
                    panic!("empty catalog must not activate a provider model")
                }
                Ok(Ok(crate::bus::BusEvent::LoginCompleted(login))) => {
                    panic!("empty local catalog must not publish final login failure: {login:?}")
                }
                Ok(Ok(crate::bus::BusEvent::UiActivity(activity)))
                    if activity.message.contains("Model Discovery Still Updating") =>
                {
                    break activity;
                }
                Ok(Ok(_)) => continue,
                other => panic!("expected pending catalog activity, got {other:?}"),
            }
        }
    });

    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        set_model_attempts.load(Ordering::SeqCst),
        0,
        "post-login activation must not try the metadata default when the catalog has no selectable route"
    );
    assert!(activity.message.contains("Saved credentials are active"));
    assert!(activity.message.contains("Jcode is still processing"));
    assert!(!activity.message.contains("did not switch models"));
    assert!(!activity.message.contains("documented default"));
    assert!(!activity.message.contains("qwen-3-coder-480b"));
}
#[test]
fn test_tui_openai_compatible_local_refresh_failure_is_pending_not_final_failure() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let refreshes = StdArc::new(AtomicUsize::new(0));
    let set_model_attempts = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(FailingPostLoginCatalogProvider {
        refreshes: StdArc::clone(&refreshes),
        set_model_attempts: StdArc::clone(&set_model_attempts),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    let mut bus_rx = crate::bus::Bus::global().subscribe();
    while bus_rx.try_recv().is_ok() {}

    let _guard = rt.enter();
    app.start_openai_compatible_post_login_activation(
        "cerebras".to_string(),
        "Cerebras".to_string(),
    );

    let activity = rt.block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), bus_rx.recv()).await {
                Ok(Ok(crate::bus::BusEvent::ProviderModelActivated { .. })) => {
                    panic!("failing local refresh must not activate a provider model")
                }
                Ok(Ok(crate::bus::BusEvent::LoginCompleted(login))) => {
                    panic!(
                        "local refresh failure must not publish a final login failure while server auth-change recovery can still finish: {login:?}"
                    )
                }
                Ok(Ok(crate::bus::BusEvent::UiActivity(activity)))
                    if activity.message.contains("Model Discovery Still Updating") =>
                {
                    break activity;
                }
                Ok(Ok(_)) => continue,
                other => panic!("expected pending catalog activity, got {other:?}"),
            }
        }
    });

    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        set_model_attempts.load(Ordering::SeqCst),
        0,
        "local refresh failure must not try to switch models from an unavailable catalog"
    );
    assert!(activity.message.contains("Saved credentials are active"));
    assert!(
        activity
            .message
            .contains("server auth-change catalog refresh")
    );
    assert!(activity.message.contains("fixture refresh failed"));
    assert!(!activity.message.contains("Login: failed"));
    assert!(!activity.message.contains("Unable to sign in"));
    assert!(!activity.message.contains("did not switch models"));
}
#[test]
fn test_model_picker_opens_simplified_state_before_async_routes_complete() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(CountingModelRoutesProvider {
        calls: StdArc::clone(&calls),
        route_count: 2,
        delay: Duration::from_millis(75),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    app.open_model_picker();

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("loading picker should open immediately");
    assert_eq!(picker.entries.len(), 1);
    assert_eq!(picker.entries[0].name, "counting-a");
    assert_eq!(picker.entries[0].options[0].detail, "simplified catalog");
    assert!(app.pending_model_picker_load.is_some());
    assert_eq!(
        app.status_notice(),
        Some("Updating model routes…".to_string())
    );

    wait_for_model_picker_load(&mut app);
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("hydrated picker should still be open");
    assert!(picker.entries.len() >= 2);
    assert_eq!(app.status_notice(), Some("Model list updated".to_string()));
}
#[test]
fn test_model_picker_state_space_preserves_provider_labels_after_route_hydration() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let provider: Arc<dyn Provider> = Arc::new(MixedModelRoutesProvider {
        model: StdArc::new(StdMutex::new("gpt-5.5".to_string())),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.recent_authenticated_provider = Some(("chutes".to_string(), Instant::now()));

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("hydrated mixed-provider model picker should be open");
    let mut routes_by_model = std::collections::BTreeMap::new();
    for entry in &picker.entries {
        let route = entry
            .active_option()
            .expect("model picker entry should have an active route");
        routes_by_model.insert(
            entry.name.clone(),
            (route.provider.clone(), route.api_method.clone()),
        );
    }

    // Models with reasoning-effort support expand into effort rows (issue
    // #458); the hydrated route must be preserved on each variant.
    assert_eq!(
        routes_by_model.get("gpt-5.5 (high)"),
        Some(&("OpenAI".to_string(), "openai-oauth".to_string()))
    );
    assert_eq!(
        routes_by_model.get("claude-opus-4-6 (high)"),
        Some(&("Anthropic".to_string(), "claude-oauth".to_string()))
    );
    assert_eq!(
        routes_by_model.get("Qwen/Qwen3-Coder-480B-A35B-Instruct"),
        Some(&("Chutes".to_string(), "openai-compatible:chutes".to_string()))
    );
    assert_eq!(
        routes_by_model.get("deepseek/deepseek-v4-pro (high)"),
        Some(&("auto".to_string(), "openrouter".to_string()))
    );

    let chutes_rows = picker
        .entries
        .iter()
        .filter(|entry| {
            entry
                .active_option()
                .map(|route| route.provider == "Chutes")
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        chutes_rows, 1,
        "opening the model list must not collapse every route to the recently authenticated direct provider: {:?}",
        routes_by_model
    );
}
#[test]
fn test_model_picker_does_not_cache_single_model_fallback() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(CountingModelRoutesProvider {
        calls: StdArc::clone(&calls),
        route_count: 1,
        delay: Duration::ZERO,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        app.model_picker_cache.is_none(),
        "single-model fallback results should not be retained"
    );

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "single-model fallback should be rebuilt so a later full catalog can surface"
    );
}
#[test]
fn test_local_model_picker_selection_failure_keeps_picker_open_and_shows_next_steps() {
    let mut app = create_failing_model_switch_test_app();

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    assert!(app.inline_interactive_state.is_some());

    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .expect("enter should be handled");

    assert!(
        app.inline_interactive_state.is_some(),
        "picker should remain open so the user can choose another model"
    );
    assert_eq!(app.status_notice(), Some("Model switch failed".to_string()));

    let last = app.display_messages.last().expect("display message");
    assert_eq!(last.role, "error");
    assert!(last.content.contains("credentials expired"));
    assert!(last.content.contains("/model"));
    assert!(last.content.contains("/login"));
    assert!(last.content.contains("/account"));
}
#[test]
fn test_login_completed_spawns_auth_refresh_when_runtime_is_available() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let started = StdArc::new(AtomicBool::new(false));
    let completed = StdArc::new(AtomicBool::new(false));
    let provider: Arc<dyn Provider> = Arc::new(AsyncAuthRefreshingMockProvider {
        started: StdArc::clone(&started),
        completed: StdArc::clone(&completed),
        delay: Duration::from_millis(150),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    let _guard = rt.enter();
    let start = Instant::now();
    app.handle_login_completed(crate::bus::LoginCompleted {
        provider: "openrouter".to_string(),
        success: true,
        message: "OpenRouter ready".to_string(),
    });
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(100),
        "login completion should not block on auth refresh, took {:?}",
        elapsed
    );

    let wait_start = Instant::now();
    while !started.load(Ordering::SeqCst) || !completed.load(Ordering::SeqCst) {
        assert!(
            wait_start.elapsed() < Duration::from_secs(2),
            "background auth refresh did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn test_model_picker_waits_for_async_post_login_catalog_activation() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let logged_in = StdArc::new(StdMutex::new(false));
    let provider: Arc<dyn Provider> = Arc::new(AuthRefreshingMockProvider {
        logged_in: StdArc::clone(&logged_in),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    let mut bus_rx = crate::bus::Bus::global().subscribe();

    {
        let _guard = rt.enter();
        app.handle_login_completed(crate::bus::LoginCompleted {
            provider: "auto-import".to_string(),
            success: true,
            message: "Imported existing logins".to_string(),
        });
        app.open_model_picker();
    }

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("loading model picker should be open");
    assert_eq!(picker.entries.len(), 1);
    assert!(
        picker.entries[0].options[0]
            .detail
            .contains("updating model list")
    );
    // The single loading row labels the *current* model (which may legitimately
    // still be the pre-import one until async activation lands), so check the
    // route metadata: the stale pre-import catalog route must not be shown as a
    // selectable ready entry.
    assert!(
        !picker
            .entries
            .iter()
            .flat_map(|entry| entry.options.iter())
            .any(|option| option.api_method == "openai-oauth"),
        "the stale pre-import catalog must not be presented as ready"
    );

    let ready = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = bus_rx.recv().await.expect("auth catalog event");
                if matches!(event, crate::bus::BusEvent::AuthCatalogRefreshReady) {
                    break event;
                }
            }
        })
        .await
        .expect("post-login activation should finish")
    });
    assert!(crate::tui::app::local::handle_bus_event(
        &mut app,
        Ok(ready)
    ));
    assert!(*logged_in.lock().unwrap());
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should refresh in place");
    assert!(
        picker
            .entries
            .iter()
            .any(|entry| entry.name == "claude-opus-4.6")
    );
    assert!(
        picker
            .entries
            .iter()
            .any(|entry| entry.name == "grok-code-fast-1")
    );
}
#[test]
fn test_login_completed_surfaces_new_provider_models_in_local_model_picker() {
    let mut app = create_auth_refresh_test_app();

    app.handle_login_completed(crate::bus::LoginCompleted {
        provider: "copilot".to_string(),
        success: true,
        message: "Authenticated as **octocat** via GitHub Copilot.\n\nCopilot models are now available in `/model`."
            .to_string(),
    });

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker should be open");

    let copilot_entry = picker
        .entries
        .iter()
        .find(|entry| entry.name == "claude-opus-4.6")
        .expect("copilot model should be shown after login");

    assert!(
        picker
            .entries
            .iter()
            .any(|entry| entry.name == "grok-code-fast-1"),
        "all newly available Copilot models should appear in /model"
    );
    assert!(copilot_entry.options.iter().any(|route| {
        route.provider == "Copilot" && route.api_method == "copilot" && route.available
    }));

    assert!(
        picker.entries[0]
            .options
            .iter()
            .any(|route| route.provider == "Copilot" && route.detail.contains("recently added")),
        "recently authenticated provider should be prioritized and marked in /model"
    );
}
#[test]
fn test_azure_login_completion_switches_local_model_without_completion() {
    let _env_lock = crate::storage::lock_test_env();
    let _guard = AzureLoginEnvGuard::save(&[
        "AZURE_OPENAI_ENDPOINT",
        "AZURE_OPENAI_MODEL",
        "AZURE_OPENAI_API_KEY",
        "AZURE_OPENAI_USE_ENTRA",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_AUTH_HEADER",
        "JCODE_OPENROUTER_DYNAMIC_BEARER_PROVIDER",
        "JCODE_OPENROUTER_MODEL",
        "JCODE_RUNTIME_PROVIDER",
        "JCODE_ACTIVE_PROVIDER",
        "JCODE_INITIAL_PROVIDER_EXPLICIT",
    ]);
    crate::env::set_var("AZURE_OPENAI_ENDPOINT", "https://example.openai.azure.com");
    crate::env::set_var("AZURE_OPENAI_MODEL", "azure-deployment");
    crate::env::set_var("AZURE_OPENAI_API_KEY", "test-key");
    crate::env::set_var("AZURE_OPENAI_USE_ENTRA", "0");

    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let model = StdArc::new(StdMutex::new("old-model".to_string()));
    let auth_changed = StdArc::new(AtomicUsize::new(0));
    let complete_calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(AzureLoginMockProvider {
        model: StdArc::clone(&model),
        auth_changed: StdArc::clone(&auth_changed),
        complete_calls: StdArc::clone(&complete_calls),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    // This test asserts the login-completed status notice; a brand-new-install
    // classification would let first-run onboarding overwrite it with the
    // StartChoice prompt. Pre-commit the onboarding guard so the flow never
    // starts.
    app.onboarding_startup_checked = true;
    app.onboarding_flow = Some(crate::tui::app::onboarding_flow::OnboardingFlow {
        phase: crate::tui::app::onboarding_flow::OnboardingPhase::Done,
    });
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    app.provider_session_id = Some("stale-upstream".to_string());
    app.session.provider_session_id = Some("stale-upstream".to_string());
    app.session.model = Some("old-model".to_string());

    app.handle_login_completed(crate::bus::LoginCompleted {
        provider: "Azure OpenAI".to_string(),
        success: true,
        message: "Azure OpenAI ready".to_string(),
    });

    assert_eq!(&*model.lock().unwrap(), "azure-deployment");
    assert_eq!(app.session.model.as_deref(), Some("azure-deployment"));
    assert_eq!(app.provider_session_id, None);
    assert_eq!(app.session.provider_session_id, None);
    assert_eq!(auth_changed.load(Ordering::SeqCst), 1);
    assert_eq!(complete_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::env::var("JCODE_RUNTIME_PROVIDER").as_deref(),
        Ok("azure-openai")
    );
    assert_eq!(
        app.status_notice(),
        Some("Login: Azure OpenAI ready (azure-deployment)".to_string())
    );
}
