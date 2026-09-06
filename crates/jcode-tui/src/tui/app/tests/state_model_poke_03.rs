#[test]
fn test_model_picker_preview_arrow_keys_navigate() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    // Type /model to open preview
    for c in "/model".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker preview should be open");
    assert!(picker.preview);
    let initial_selected = picker.selected;

    // Down arrow should navigate in preview mode
    app.handle_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("picker should still be open");
    assert!(picker.preview, "should remain in preview mode");
    assert_eq!(picker.selected, initial_selected + 1);

    // Up arrow should navigate back
    app.handle_key(KeyCode::Up, KeyModifiers::empty()).unwrap();

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("picker should still be open");
    assert!(picker.preview, "should remain in preview mode");
    assert_eq!(picker.selected, initial_selected);

    // Opening the preview should place the cursor in the model filter argument.
    assert_eq!(app.input(), "/model ");
}

#[test]
fn test_open_model_picker_without_routes_shows_actionable_guidance() {
    let mut app = create_test_app();

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);

    assert!(app.inline_interactive_state.is_none());
    assert_eq!(app.status_notice(), Some("No models available".to_string()));

    let last = app.display_messages.last().expect("display message");
    assert_eq!(last.role, "system");
    assert!(last.content.contains("/login"));
    assert!(last.content.contains("/account"));
    assert!(last.content.contains("/model"));
}

#[test]
fn test_remote_model_picker_during_startup_waits_for_session_catalog() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.set_remote_startup_phase(crate::tui::app::RemoteStartupPhase::LoadingSession);
    app.remote_provider_model = Some("gpt-5.6-sol".to_string());
    app.remote_available_entries.clear();
    app.remote_model_options.clear();

    app.open_model_picker();

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("loading model picker should be open");
    assert_eq!(picker.entries.len(), 1);
    assert_eq!(picker.entries[0].name, "gpt-5.6-sol");
    assert_eq!(picker.entries[0].options[0].detail, "updating model list…");
}

#[test]
fn test_remote_model_command_opens_picker_without_catalog_request() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);
    app.input = "/model".to_string();
    app.cursor_pos = app.input.len();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();
    let request_id_before = remote.next_request_id_for_test();

    rt.block_on(app.handle_remote_key(KeyCode::Enter, KeyModifiers::empty(), &mut remote))
        .unwrap();

    assert!(app.inline_interactive_state.is_some());
    assert_eq!(
        remote.next_request_id_for_test(),
        request_id_before,
        "opening /model must not refresh or request a remote catalog"
    );
}

#[derive(Clone)]
struct CountingModelRoutesProvider {
    calls: StdArc<AtomicUsize>,
    route_count: usize,
    delay: Duration,
}

#[derive(Clone)]
struct MixedModelRoutesProvider {
    model: StdArc<StdMutex<String>>,
}

#[derive(Clone)]
struct AuthUxStateSpaceProvider {
    authed: StdArc<AtomicBool>,
    refreshes: StdArc<AtomicUsize>,
    model: StdArc<StdMutex<String>>,
    set_model_requests: StdArc<StdMutex<Vec<String>>>,
    provider_id: &'static str,
    provider_label: &'static str,
    models: &'static [&'static str],
    include_wrong_profile_first: bool,
    include_generic_profile_duplicate: bool,
}

#[derive(Clone)]
struct EmptyPostLoginCatalogProvider {
    refreshes: StdArc<AtomicUsize>,
    set_model_attempts: StdArc<AtomicUsize>,
}

#[derive(Clone)]
struct FailingPostLoginCatalogProvider {
    refreshes: StdArc<AtomicUsize>,
    set_model_attempts: StdArc<AtomicUsize>,
}

impl AuthUxStateSpaceProvider {
    fn routes(&self) -> Vec<crate::provider::ModelRoute> {
        let authed = self.authed.load(Ordering::SeqCst);
        let mut routes = Vec::new();
        if self.include_wrong_profile_first {
            routes.push(crate::provider::ModelRoute {
                model: "wrong-profile-first".to_string(),
                provider: self.provider_label.to_string(),
                api_method: "openai-compatible:other-provider".to_string(),
                available: authed,
                detail: if authed {
                    "fresh wrong-profile catalog route".to_string()
                } else {
                    "no API key".to_string()
                },
                cheapness: None,
            });
        }
        for model in self.models {
            routes.push(crate::provider::ModelRoute {
                model: (*model).to_string(),
                provider: self.provider_label.to_string(),
                api_method: format!("openai-compatible:{}", self.provider_id),
                available: authed,
                detail: if authed {
                    "fresh catalog route".to_string()
                } else {
                    "no API key".to_string()
                },
                cheapness: None,
            });
            if self.include_generic_profile_duplicate {
                routes.push(crate::provider::ModelRoute {
                    model: (*model).to_string(),
                    provider: self.provider_label.to_string(),
                    api_method: "openai-compatible".to_string(),
                    available: authed,
                    detail: if authed {
                        "duplicate generic direct route".to_string()
                    } else {
                        "no API key".to_string()
                    },
                    cheapness: None,
                });
            }
        }
        routes
    }
}

impl MixedModelRoutesProvider {
    fn routes() -> Vec<crate::provider::ModelRoute> {
        vec![
            crate::provider::ModelRoute {
                model: "gpt-5.5".to_string(),
                provider: "OpenAI".to_string(),
                api_method: "openai-oauth".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            },
            crate::provider::ModelRoute {
                model: "claude-opus-4-6".to_string(),
                provider: "Anthropic".to_string(),
                api_method: "claude-oauth".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            },
            crate::provider::ModelRoute {
                model: "Qwen/Qwen3-Coder-480B-A35B-Instruct".to_string(),
                provider: "Chutes".to_string(),
                api_method: "openai-compatible:chutes".to_string(),
                available: true,
                detail: "https://llm.chutes.ai/v1".to_string(),
                cheapness: None,
            },
            crate::provider::ModelRoute {
                model: "deepseek/deepseek-v4-pro".to_string(),
                provider: "auto".to_string(),
                api_method: "openrouter".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            },
        ]
    }
}

#[async_trait::async_trait]
impl Provider for AuthUxStateSpaceProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("AuthUxStateSpaceProvider")
    }

    fn name(&self) -> &str {
        "openrouter"
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn available_models_display(&self) -> Vec<String> {
        self.routes()
            .into_iter()
            .filter(|route| route.available)
            .map(|route| route.model)
            .collect()
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        self.routes()
    }

    fn set_model(&self, model: &str) -> Result<()> {
        self.set_model_requests
            .lock()
            .unwrap()
            .push(model.to_string());
        let model = model
            .strip_prefix(&format!("{}:", self.provider_id))
            .unwrap_or(model);
        let found = self
            .routes()
            .into_iter()
            .any(|route| route.available && route.model == model);
        if !found {
            anyhow::bail!("model {model} is not available in the refreshed catalog");
        }
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn on_auth_changed(&self) {
        self.authed.store(true, Ordering::SeqCst);
    }

    async fn refresh_model_catalog(&self) -> Result<crate::provider::ModelCatalogRefreshSummary> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        Ok(crate::provider::ModelCatalogRefreshSummary {
            model_count_before: 0,
            model_count_after: 2,
            models_added: 2,
            models_removed: 0,
            models_added_names: Vec::new(),
            models_removed_names: Vec::new(),
            route_count_before: 0,
            route_count_after: 2,
            routes_added: 2,
            routes_removed: 0,
            routes_changed: 0,
        })
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Provider for MixedModelRoutesProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("MixedModelRoutesProvider")
    }

    fn name(&self) -> &str {
        "mixed"
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn available_models_display(&self) -> Vec<String> {
        Self::routes()
            .into_iter()
            .map(|route| route.model)
            .collect()
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        Self::routes()
    }

    fn set_model(&self, model: &str) -> Result<()> {
        let model = model.strip_prefix("chutes:").unwrap_or(model);
        if !Self::routes().iter().any(|route| route.model == model) {
            anyhow::bail!("model {model} is not available in the mixed catalog");
        }
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Provider for EmptyPostLoginCatalogProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("EmptyPostLoginCatalogProvider")
    }

    fn name(&self) -> &str {
        "empty-catalog"
    }

    fn model(&self) -> String {
        "pre-auth-model".to_string()
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        vec![]
    }

    fn set_model(&self, model: &str) -> Result<()> {
        self.set_model_attempts.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("unexpected attempt to switch to {model}")
    }

    async fn refresh_model_catalog(&self) -> Result<crate::provider::ModelCatalogRefreshSummary> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        Ok(crate::provider::ModelCatalogRefreshSummary {
            model_count_before: 0,
            model_count_after: 0,
            models_added: 0,
            models_removed: 0,
            models_added_names: Vec::new(),
            models_removed_names: Vec::new(),
            route_count_before: 0,
            route_count_after: 0,
            routes_added: 0,
            routes_removed: 0,
            routes_changed: 0,
        })
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Provider for FailingPostLoginCatalogProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("FailingPostLoginCatalogProvider")
    }

    fn name(&self) -> &str {
        "failing-catalog"
    }

    fn model(&self) -> String {
        "pre-auth-model".to_string()
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        vec![]
    }

    fn set_model(&self, model: &str) -> Result<()> {
        self.set_model_attempts.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("unexpected attempt to switch to {model}")
    }

    async fn refresh_model_catalog(&self) -> Result<crate::provider::ModelCatalogRefreshSummary> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fixture refresh failed before server auth-change catalog refresh")
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Provider for CountingModelRoutesProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("CountingModelRoutesProvider")
    }

    fn name(&self) -> &str {
        "counting"
    }

    fn model(&self) -> String {
        "counting-a".to_string()
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        (0..self.route_count)
            .map(|idx| crate::provider::ModelRoute {
                model: if idx < 26 {
                    format!("counting-{}", (b'a' + idx as u8) as char)
                } else {
                    format!("counting-{idx}")
                },
                provider: "Counting".to_string(),
                api_method: "test".to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            })
            .collect()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[test]
fn test_subagent_model_large_catalog_uses_cached_searchable_picker() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(CountingModelRoutesProvider {
        calls: StdArc::clone(&calls),
        route_count: 400,
        delay: Duration::from_millis(25),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    for c in "/subagent-model".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    wait_for_model_picker_load(&mut app);

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("subagent model picker should be open");
    assert!(picker.preview);
    assert_eq!(picker.entries.len(), 401, "400 models plus inherit");
    assert!(matches!(
        picker.entries[0].action,
        crate::tui::PickerAction::SubagentModelChoice { inherit: true }
    ));
    let calls_after_load = calls.load(Ordering::SeqCst);

    for _ in 0..10 {
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        calls_after_load,
        "navigation must not rebuild or re-query the model catalog"
    );

    app.handle_key(KeyCode::Char('3'), KeyModifiers::empty())
        .unwrap();
    let picker = app.inline_interactive_state.as_ref().unwrap();
    assert!(
        !picker.filtered.is_empty(),
        "typed input should filter models"
    );
    assert!(picker.filtered.len() < picker.entries.len());

    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    assert!(app.session.subagent_model.is_some());
    assert_eq!(app.provider.model(), "counting-a");
}

#[test]
fn test_model_picker_reuses_cached_entries_until_invalidated() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(CountingModelRoutesProvider {
        calls: StdArc::clone(&calls),
        route_count: 2,
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
    assert!(app.model_picker_cache.is_some());

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "second open should reuse cached picker entries"
    );

    app.invalidate_model_picker_cache();
    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "invalidating should force rebuilding provider routes"
    );
}

#[test]
fn test_shift_tab_model_favorite_hotkey_preserves_input_line() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests();

    let calls = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(CountingModelRoutesProvider {
        calls: StdArc::clone(&calls),
        route_count: 2,
        delay: Duration::ZERO,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;

    app.set_input_for_test("do not drop this draft");
    let cursor = app.cursor_pos();

    app.handle_key(KeyCode::BackTab, KeyModifiers::SHIFT)
        .unwrap();
    wait_for_model_picker_load(&mut app);

    assert_eq!(app.input(), "do not drop this draft");
    assert_eq!(app.cursor_pos(), cursor);
}

#[test]
fn test_new_local_session_does_not_run_post_login_model_refresh() {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();

    let authed = StdArc::new(AtomicBool::new(false));
    let refreshes = StdArc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(AuthUxStateSpaceProvider {
        authed: StdArc::clone(&authed),
        refreshes: StdArc::clone(&refreshes),
        model: StdArc::new(StdMutex::new("existing-model".to_string())),
        set_model_requests: StdArc::new(StdMutex::new(Vec::new())),
        provider_id: "state-space",
        provider_label: "StateSpace",
        models: &["state-space-alpha", "state-space-beta"],
        include_wrong_profile_first: false,
        include_generic_profile_duplicate: false,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let _guard = rt.enter();
    let app = App::new_for_test_harness(provider, registry);

    // Give any accidentally spawned startup work a chance to execute.
    std::thread::sleep(Duration::from_millis(50));

    assert!(
        !authed.load(Ordering::SeqCst),
        "startup called on_auth_changed"
    );
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        0,
        "startup refreshed a provider catalog"
    );
    assert!(!app.auth_catalog_refresh_pending);
    assert!(
        !app.onboarding_auto_model_selection_active
            .load(Ordering::SeqCst)
    );
    assert!(
        app.onboarding_auto_model_selection_baseline
            .lock()
            .unwrap()
            .is_none()
    );
}

#[derive(Clone)]
struct AzureLoginMockProvider {
    model: StdArc<StdMutex<String>>,
    auth_changed: StdArc<AtomicUsize>,
    complete_calls: StdArc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for AzureLoginMockProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        self.complete_calls.fetch_add(1, Ordering::SeqCst);
        let stream = futures::stream::empty::<Result<crate::message::StreamEvent>>();
        Ok(Box::pin(stream) as crate::provider::EventStream)
    }

    fn name(&self) -> &str {
        "OpenRouter"
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn set_model(&self, model: &str) -> Result<()> {
        let model = model
            .trim()
            .strip_prefix("openrouter:")
            .unwrap_or_else(|| model.trim())
            .trim();
        if model.is_empty() {
            anyhow::bail!("model cannot be empty");
        }
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn available_models_display(&self) -> Vec<String> {
        vec![self.model()]
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        vec![crate::provider::ModelRoute {
            model: self.model(),
            provider: "Azure OpenAI".to_string(),
            api_method: "openai-compatible".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        }]
    }

    fn on_auth_changed(&self) {
        self.auth_changed.fetch_add(1, Ordering::SeqCst);
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

struct AzureLoginEnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl AzureLoginEnvGuard {
    fn save(keys: &[&'static str]) -> Self {
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in keys {
            crate::env::remove_var(key);
        }
        Self { saved }
    }
}

impl Drop for AzureLoginEnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            if let Some(value) = value {
                crate::env::set_var(key, value);
            } else {
                crate::env::remove_var(key);
            }
        }
    }
}

#[test]
fn test_login_picker_preview_stays_open_and_updates_filter() {
    let mut app = create_test_app();

    for c in "/login za".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("login picker preview should be open");
    assert!(picker.preview);
    assert_eq!(picker.kind, crate::tui::PickerKind::Login);
    assert_eq!(picker.filter, "za");
    assert!(
        picker
            .filtered
            .iter()
            .any(|&i| picker.entries[i].name == "Z.AI")
    );
    assert_eq!(app.input(), "/login za");
}

#[test]
fn test_login_picker_preview_enter_starts_login_flow() {
    let mut app = create_test_app();

    for c in "/login zai".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert!(app.inline_interactive_state.is_none());
    match app.pending_login {
        Some(crate::tui::app::auth::PendingLogin::ApiKeyProfile {
            provider,
            openai_compatible_profile: Some(profile),
            ..
        }) => {
            assert_eq!(provider, "Z.AI");
            assert_eq!(profile.id, crate::provider_catalog::ZAI_PROFILE.id);
        }
        ref other => panic!("unexpected pending login state: {other:?}"),
    }
}

#[test]
fn test_typing_login_auto_inserts_filter_space() {
    let mut app = create_test_app();

    for c in "/login".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }

    // The trailing space arms provider filtering immediately, so the next
    // keystrokes filter the login picker instead of extending the command.
    assert_eq!(app.input(), "/login ");
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("login picker preview should be open");
    assert!(picker.preview);
    assert_eq!(picker.kind, crate::tui::PickerKind::Login);
    assert_eq!(picker.filter, "");

    // A habitual manually-typed space is swallowed instead of doubling up.
    app.handle_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.input(), "/login ");

    for c in "za".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    assert_eq!(app.input(), "/login za");
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("login picker preview should stay open");
    assert_eq!(picker.filter, "za");
}

#[test]
fn test_login_preview_enter_without_selection_focuses_picker_instead_of_logging_in() {
    let mut app = create_test_app();

    for c in "/login".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    // No filter and no explicit selection: Enter must not launch the first
    // provider's login flow. It focuses the picker for a deliberate choice.
    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("login picker should stay open after bare Enter");
    assert!(!picker.preview, "picker should be focused (not preview)");
    assert_eq!(picker.kind, crate::tui::PickerKind::Login);
    assert!(app.pending_login.is_none());
    assert_eq!(app.input(), "");
}

#[test]
fn test_login_preview_enter_after_navigation_starts_selected_login() {
    let mut app = create_test_app();

    for c in "/login".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    // Explicit navigation makes the selection deliberate, so Enter activates.
    // Navigate to the Anthropic API key row (an offline api-key prompt flow).
    app.handle_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert!(
        app.inline_interactive_state.is_none(),
        "picker should close after selecting a provider"
    );
    assert!(
        app.pending_login.is_some(),
        "selected provider login flow should start"
    );
}

#[test]
fn test_subagent_model_command_sets_and_resets_session_preference() {
    let mut app = create_test_app();

    assert!(super::commands::handle_session_command(
        &mut app,
        "/subagent-model gpt-5.4"
    ));
    assert_eq!(app.session.subagent_model.as_deref(), Some("gpt-5.4"));

    assert!(super::commands::handle_session_command(
        &mut app,
        "/subagent-model inherit"
    ));
    assert_eq!(app.session.subagent_model, None);
}

#[test]
fn test_autoreview_command_toggles_session_preference() {
    let mut app = create_test_app();

    assert!(super::commands::handle_session_command(
        &mut app,
        "/autoreview on"
    ));
    assert_eq!(app.session.autoreview_enabled, Some(true));
    assert!(app.autoreview_enabled);

    assert!(super::commands::handle_session_command(
        &mut app,
        "/autoreview off"
    ));
    assert_eq!(app.session.autoreview_enabled, Some(false));
    assert!(!app.autoreview_enabled);
}

#[test]
fn test_autojudge_command_toggles_session_preference() {
    let mut app = create_test_app();

    assert!(super::commands::handle_session_command(
        &mut app,
        "/autojudge on"
    ));
    assert_eq!(app.session.autojudge_enabled, Some(true));
    assert!(app.autojudge_enabled);

    assert!(super::commands::handle_session_command(
        &mut app,
        "/autojudge off"
    ));
    assert_eq!(app.session.autojudge_enabled, Some(false));
    assert!(!app.autojudge_enabled);
}

#[test]
fn test_transcript_path_command_reports_current_session_file() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        let expected = crate::session::session_path(&app.session.id).expect("session path");

        assert!(super::commands::handle_session_command(
            &mut app,
            "/transcript path"
        ));

        assert!(app.display_messages().iter().any(|msg| {
            msg.content.contains("Transcript file:")
                && msg.content.contains(&expected.display().to_string())
        }));
    });
}

#[test]
fn test_help_topic_shows_overnight_command_details() {
    let mut app = create_test_app();
    app.input = "/help overnight".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/overnight <hours>[h|m] [mission]"));
    assert!(msg.content.contains("review HTML page"));
    assert!(msg.content.contains("/overnight status"));
}

#[test]
fn test_overnight_status_without_runs_is_handled() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(super::commands::handle_session_command(
            &mut app,
            "/overnight status"
        ));

        let msg = app
            .display_messages()
            .last()
            .expect("missing overnight status response");
        assert_eq!(msg.role, "system");
        assert!(msg.content.contains("No overnight runs found"));
    });
}

#[test]
fn test_overnight_help_command_is_handled() {
    let mut app = create_test_app();
    assert!(super::commands::handle_session_command(
        &mut app,
        "/overnight help"
    ));

    let msg = app
        .display_messages()
        .last()
        .expect("missing overnight help response");
    assert_eq!(msg.role, "system");
    assert!(msg.content.contains("/overnight <hours>[h|m] [mission]"));
    assert!(msg.content.contains("/overnight review"));
}

#[test]
fn test_overnight_start_runs_as_visible_local_turn() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(super::commands::handle_session_command(
            &mut app,
            "/overnight 1m hi"
        ));

        assert!(
            app.pending_turn,
            "local overnight should start a visible turn"
        );
        assert!(
            app.is_processing,
            "local overnight should enter processing state"
        );
        assert!(
            app.queued_messages.is_empty(),
            "local overnight should not use remote queue"
        );
        let last_message = app
            .session
            .messages
            .last()
            .expect("overnight prompt message");
        assert!(last_message.content.iter().any(|block| matches!(
            block,
            crate::message::ContentBlock::Text { text, .. }
                if text.contains("visible Overnight Coordinator")
        )));
    });
}

#[test]
fn test_overnight_start_queues_remote_turn_without_stuck_sending() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        assert!(super::commands::handle_session_command(
            &mut app,
            "/overnight 1m hi"
        ));

        assert!(
            !app.pending_turn,
            "remote overnight should not set local pending_turn"
        );
        assert!(
            !app.is_processing,
            "remote overnight should not get stuck in local Sending"
        );
        assert_eq!(app.queued_messages.len(), 1);
        assert!(app.queued_messages[0].contains("visible Overnight Coordinator"));
    });
}

include!("state_model_poke_03/catalog_activation.rs");
include!("state_model_poke_03/route_selection.rs");
include!("state_model_poke_03/auto_poke.rs");
