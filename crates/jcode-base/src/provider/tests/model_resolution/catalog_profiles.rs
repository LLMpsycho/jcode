#[test]
fn test_provider_for_model_claude() {
    assert_eq!(provider_for_model("claude-opus-4-6"), Some("claude"));
    assert_eq!(provider_for_model("claude-opus-4-6[1m]"), Some("claude"));
    assert_eq!(provider_for_model("claude-sonnet-4-6"), Some("claude"));
}
#[test]
fn test_provider_for_model_openai() {
    assert_eq!(provider_for_model("gpt-5.2-codex"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.5"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4[1m]"), Some("openai"));
    assert_eq!(provider_for_model("gpt-5.4-pro"), Some("openai"));
}
#[test]
fn test_provider_for_model_gemini() {
    assert_eq!(provider_for_model("gemini-2.5-pro"), Some("gemini"));
    assert_eq!(provider_for_model("gemini-2.5-flash"), Some("gemini"));
    assert_eq!(provider_for_model("gemini-3-pro-preview"), Some("gemini"));
}
#[test]
fn test_provider_for_model_bedrock() {
    assert_eq!(provider_for_model("amazon.nova-pro-v1:0"), Some("bedrock"));
    assert_eq!(
        provider_for_model("us.amazon.nova-micro-v1:0"),
        Some("bedrock")
    );
    assert_eq!(
        provider_for_model(
            "arn:aws:bedrock:us-east-2:302154194530:inference-profile/us.deepseek.r1-v1:0"
        ),
        Some("bedrock")
    );
}
#[test]
fn test_provider_for_model_openrouter() {
    // OpenRouter uses provider/model format
    assert_eq!(
        provider_for_model("anthropic/claude-sonnet-4"),
        Some("openrouter")
    );
    assert_eq!(provider_for_model("openai/gpt-4o"), Some("openrouter"));
    assert_eq!(
        provider_for_model("google/gemini-2.0-flash"),
        Some("openrouter")
    );
    assert_eq!(
        provider_for_model("meta-llama/llama-3.1-405b"),
        Some("openrouter")
    );
}
#[test]
fn test_openrouter_catalog_model_id_normalizes_bare_openai_and_claude_models() {
    assert_eq!(
        openrouter_catalog_model_id("gpt-5.4").as_deref(),
        Some("openai/gpt-5.4")
    );
    assert_eq!(
        openrouter_catalog_model_id("claude-sonnet-4-6").as_deref(),
        Some("anthropic/claude-sonnet-4-6")
    );
    assert_eq!(
        openrouter_catalog_model_id("anthropic/claude-sonnet-4").as_deref(),
        Some("anthropic/claude-sonnet-4")
    );
    assert_eq!(
        openrouter_catalog_model_id(
            "arn:aws:bedrock:us-east-2:302154194530:inference-profile/us.deepseek.r1-v1:0"
        ),
        None
    );
    assert_eq!(openrouter_catalog_model_id("composer-2-fast"), None);
}
#[test]
fn test_available_models_display_uses_route_models_and_filters_placeholder_rows() {
    // Hermetic env: this reads the process-global model catalog, so without a
    // clean scope it observes whatever catalog a sibling test installed and
    // fails depending on test ordering.
    with_clean_provider_test_env(|| {
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(None),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenAI),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        let models = provider.available_models_display();
        assert!(
            models
                .iter()
                .any(|model| known_openai_model_ids().contains(model)),
            "route-backed display models should include OpenAI picker rows: {:?}",
            models
        );
        assert!(
            models
                .iter()
                .any(|model| known_anthropic_model_ids().contains(model)),
            "route-backed display models should include Anthropic picker rows: {:?}",
            models
        );
        assert!(!models.iter().any(|model| model == "openrouter models"));
        assert!(!models.iter().any(|model| model == "copilot models"));
    });
}
#[test]
fn test_cerebras_model_routes_are_profile_scoped_and_unique() {
    with_clean_provider_test_env(|| {
        with_env_var("CEREBRAS_API_KEY", "test-cerebras-key", || {
            crate::provider_catalog::force_apply_openai_compatible_profile_env(
                crate::provider_catalog::openai_compatible_profile_by_id("cerebras"),
            );
            let openrouter =
                test_openrouter_runtime().expect("Cerebras direct provider should initialize");
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(None),
                openai: RwLock::new(None),
                copilot_api: RwLock::new(None),
                antigravity: RwLock::new(None),
                gemini: RwLock::new(None),
                cursor: RwLock::new(None),
                bedrock: RwLock::new(None),
                openrouter: RwLock::new(Some(openrouter)),
                openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
                active_openai_compatible_profile: RwLock::new(None),
                active: RwLock::new(ActiveProvider::OpenRouter),
                use_claude_cli: false,
                startup_notices: RwLock::new(Vec::new()),
                initial_provider: Some(ActiveProvider::OpenRouter),
                routes_memo: std::sync::Mutex::new(None),
                route_pinned: std::sync::atomic::AtomicBool::new(false),
                private_session: std::sync::atomic::AtomicBool::new(false),
                post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };

            let routes = provider.model_routes();
            // Assert against the profile's current static model list so this
            // test tracks catalog updates instead of hardcoding a model that
            // Cerebras may stop serving (the original fixture pinned
            // `qwen-3-235b-a22b-instruct-2507`, which rotted when the static
            // coverage was refreshed).
            let static_models = crate::provider_catalog::openai_compatible_profile_static_models(
                crate::provider_catalog::CEREBRAS_PROFILE,
            );
            let probe_model = static_models
                .first()
                .expect("Cerebras profile should have static models")
                .clone();
            let probe_routes = routes
                .iter()
                .filter(|route| route.provider == "Cerebras" && route.model == probe_model)
                .collect::<Vec<_>>();
            assert_eq!(
                probe_routes.len(),
                1,
                "Cerebras direct route should not appear twice in provider routes: {routes:?}"
            );
            assert_eq!(probe_routes[0].api_method, "openai-compatible:cerebras");
            assert!(probe_routes[0].available);
            assert!(
                !routes.iter().any(|route| {
                    route.provider == "Cerebras" && route.api_method == "openai-compatible"
                }),
                "generic Cerebras OpenAI-compatible route should be collapsed into the profile-scoped route: {routes:?}"
            );
        })
    });
}
#[test]
fn test_direct_chutes_ignores_legacy_openrouter_catalog_cache() {
    with_clean_provider_test_env(|| {
        let temp_home = tempfile::tempdir().expect("temp HOME");
        let home = temp_home.path().to_string_lossy().to_string();
        with_env_var("HOME", &home, || {
            let cache_dir = temp_home.path().join(".jcode").join("cache");
            std::fs::create_dir_all(&cache_dir).expect("create cache dir");
            let cached_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_secs();
            std::fs::write(
                cache_dir.join("chutes_models.json"),
                serde_json::json!({
                    "cached_at": cached_at,
                    "models": [
                        { "id": "openai/gpt-chat-latest" },
                        { "id": "anthropic/claude-sonnet-latest" },
                        { "id": "openrouter/owl-alpha" }
                    ]
                })
                .to_string(),
            )
            .expect("write legacy chutes cache");

            with_env_var("CHUTES_API_KEY", "test-chutes-key", || {
                let openrouter = test_openrouter_runtime()
                    .expect("autodetected Chutes provider should initialize");
                let direct_route = openrouter
                    .direct_openai_compatible_route_parts()
                    .expect("Chutes should initialize as a direct profile");
                assert_eq!(direct_route.0, "Chutes");
                assert_eq!(direct_route.1, "openai-compatible:chutes");

                let display_models = openrouter.available_models_display();
                assert!(
                    !display_models
                        .iter()
                        .any(|model| model == "openai/gpt-chat-latest"),
                    "legacy source-less Chutes cache must not be trusted as a direct Chutes catalog: {display_models:?}"
                );

                let provider = MultiProvider {
                    claude: RwLock::new(None),
                    anthropic: RwLock::new(None),
                    openai: RwLock::new(None),
                    copilot_api: RwLock::new(None),
                    antigravity: RwLock::new(None),
                    gemini: RwLock::new(None),
                    cursor: RwLock::new(None),
                    bedrock: RwLock::new(None),
                    openrouter: RwLock::new(Some(openrouter)),
                    openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
                    active_openai_compatible_profile: RwLock::new(None),
                    active: RwLock::new(ActiveProvider::OpenRouter),
                    use_claude_cli: false,
                    startup_notices: RwLock::new(Vec::new()),
                    initial_provider: Some(ActiveProvider::OpenRouter),
                    routes_memo: std::sync::Mutex::new(None),
                    route_pinned: std::sync::atomic::AtomicBool::new(false),
                    private_session: std::sync::atomic::AtomicBool::new(false),
                    post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                };

                let routes = provider.model_routes();
                assert!(routes.iter().any(|route| {
                    route.provider == "Chutes"
                        && route.api_method == "openai-compatible:chutes"
                        && route.available
                }));
                assert!(
                    !routes.iter().any(|route| {
                        route.provider == "Chutes" && route.model == "openai/gpt-chat-latest"
                    }),
                    "stale OpenRouter catalog entries must not be relabeled as Chutes routes: {routes:?}"
                );
                assert!(
                    !routes.iter().any(|route| {
                        route.api_method == "openrouter"
                            && matches!(route.provider.as_str(), "OpenAI" | "Anthropic")
                    }),
                    "direct Chutes profiles must not add OpenRouter fallback routes: {routes:?}"
                );
            })
        })
    });
}
#[test]
fn test_auth_changed_preserves_existing_direct_profile_session() {
    with_clean_provider_test_env(|| {
        let cerebras = crate::provider_catalog::openai_compatible_profile_by_id("cerebras")
            .expect("Cerebras profile exists");
        let groq = crate::provider_catalog::openai_compatible_profile_by_id("groq")
            .expect("Groq profile exists");

        crate::env::set_var("CEREBRAS_API_KEY", "test-cerebras-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(cerebras));
        let openrouter = test_openrouter_runtime().expect("Cerebras provider should initialize");
        openrouter
            .set_model("qwen-3-235b-a22b-instruct-2507")
            .expect("Cerebras model should be selectable");

        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(Some(openrouter)),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenRouter),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: Some(ActiveProvider::OpenRouter),
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        crate::env::set_var("GROQ_API_KEY", "test-groq-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(groq));
        provider.on_auth_changed_preserve_current_provider();

        assert_eq!(provider.model(), "qwen-3-235b-a22b-instruct-2507");
        let active_direct_route = provider
            .openrouter_provider()
            .expect("existing direct provider remains installed")
            .direct_openai_compatible_route_parts()
            .expect("existing direct provider remains direct");
        assert_eq!(active_direct_route.0, "Cerebras");
        assert_eq!(active_direct_route.1, "openai-compatible:cerebras");

        let routes = provider.model_routes();
        assert!(routes.iter().any(|route| {
            route.model == "qwen-3-235b-a22b-instruct-2507"
                && route.provider == "Cerebras"
                && route.api_method == "openai-compatible:cerebras"
                && route.available
        }));
        assert!(
            routes.iter().all(|route| {
                !(route.model == "qwen-3-235b-a22b-instruct-2507" && route.provider == "Groq")
            }),
            "Groq auth should not relabel an existing Cerebras session route: {routes:?}"
        );
    });
}
#[test]
fn test_auth_changed_replaces_template_direct_profile_for_new_logins() {
    with_clean_provider_test_env(|| {
        let cerebras = crate::provider_catalog::openai_compatible_profile_by_id("cerebras")
            .expect("Cerebras profile exists");
        let groq = crate::provider_catalog::openai_compatible_profile_by_id("groq")
            .expect("Groq profile exists");

        crate::env::set_var("CEREBRAS_API_KEY", "test-cerebras-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(cerebras));
        let openrouter = test_openrouter_runtime().expect("Cerebras provider should initialize");

        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(Some(openrouter)),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenRouter),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: Some(ActiveProvider::OpenRouter),
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        crate::env::set_var("GROQ_API_KEY", "test-groq-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(groq));
        provider.on_auth_changed();

        let active_direct_route = provider
            .openrouter_provider()
            .expect("template direct provider remains installed")
            .direct_openai_compatible_route_parts()
            .expect("template direct provider remains direct");
        assert_eq!(active_direct_route.0, "Groq");
        assert_eq!(active_direct_route.1, "openai-compatible:groq");
    });
}
#[test]
fn test_state_space_openrouter_default_survives_switch_to_nvidia_nim() {
    with_clean_provider_test_env(|| {
        let nvidia = crate::provider_catalog::openai_compatible_profile_by_id("nvidia-nim")
            .expect("NVIDIA NIM profile exists");

        save_test_openrouter_model_cache(
            "openrouter",
            "https://openrouter.ai/api/v1",
            &["openrouter/owl-alpha"],
        );

        crate::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(None);
        let openrouter = test_openrouter_runtime().expect("OpenRouter provider should initialize");
        openrouter
            .set_model("openrouter/owl-alpha")
            .expect("OpenRouter default model should be selectable");

        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(Some(openrouter)),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenRouter),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        crate::env::set_var(nvidia.api_key_env, "test-nvidia-key");
        provider
            .set_model("nvidia-nim:nvidia/llama-3.1-nemotron-ultra-253b-v1")
            .expect("NVIDIA NIM model should be selectable after OpenRouter default");
        assert!(
            std::env::var_os("JCODE_OPENROUTER_API_BASE").is_none(),
            "profile route selection should not mutate global OpenRouter API base env"
        );

        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(provider.model(), "nvidia/llama-3.1-nemotron-ultra-253b-v1");
        let active_direct_route = provider
            .active_openrouter_execution_provider()
            .expect("NVIDIA direct provider is active")
            .direct_openai_compatible_route_parts()
            .expect("NVIDIA NIM is hosted through OpenAI-compatible runtime");
        assert_eq!(active_direct_route.0, "NVIDIA NIM");
        assert_eq!(active_direct_route.1, "openai-compatible:nvidia-nim");
        assert!(
            provider
                .openrouter_provider()
                .expect("real OpenRouter provider remains installed")
                .direct_openai_compatible_route_parts()
                .is_none(),
            "compatible profile must not replace the real OpenRouter slot"
        );

        let routes = provider.model_routes();
        assert!(
            routes.iter().any(|route| {
                route.model == "nvidia/llama-3.1-nemotron-ultra-253b-v1"
                    && route.provider == "NVIDIA NIM"
                    && route.api_method == "openai-compatible:nvidia-nim"
                    && route.available
            }),
            "NVIDIA route should remain selectable: {routes:?}"
        );
        assert!(
            routes.iter().any(|route| {
                route.model == "openrouter/owl-alpha"
                    && route.api_method == "openrouter"
                    && route.available
            }),
            "OpenRouter route should remain selectable after switching to NVIDIA NIM: {routes:?}"
        );
        assert!(
            routes.iter().all(|route| {
                !(route.model == "openrouter/owl-alpha" && route.provider == "NVIDIA NIM")
            }),
            "OpenRouter model must not be relabeled as NVIDIA NIM: {routes:?}"
        );

        let openrouter_route = routes
            .iter()
            .find(|route| route.model == "openrouter/owl-alpha" && route.api_method == "openrouter")
            .expect("OpenRouter route should be present after switching to NVIDIA NIM");
        let selection = crate::provider::RouteSelection::from_model_route(openrouter_route);
        provider
            .set_route_selection(&selection)
            .expect("OpenRouter route should switch runtime back to OpenRouter");
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(provider.model(), "openrouter/owl-alpha");
        let active_direct_route = provider
            .openrouter_provider()
            .expect("OpenRouter provider remains installed")
            .direct_openai_compatible_route_parts();
        assert!(
            active_direct_route.is_none(),
            "OpenRouter model should not remain bound to NVIDIA NIM runtime: {active_direct_route:?}"
        );
    });
}
#[test]
fn test_session_route_restore_request_matrix_preserves_runtime_identity() {
    let cases = [
        (
            "claude-sonnet-4-6",
            Some("claude"),
            Some("claude-oauth"),
            "claude-oauth:claude-sonnet-4-6",
        ),
        (
            "claude-sonnet-4-6",
            Some("claude"),
            Some("anthropic-api-key"),
            "claude-api:claude-sonnet-4-6",
        ),
        (
            "gpt-5.4",
            Some("openai"),
            Some("openai-oauth"),
            "openai-oauth:gpt-5.4",
        ),
        (
            "gpt-5.4",
            Some("openai"),
            Some("openai-api-key"),
            "openai-api:gpt-5.4",
        ),
        (
            "openrouter/owl-alpha",
            Some("openrouter"),
            Some("openrouter"),
            "openrouter:openrouter/owl-alpha",
        ),
        (
            "nvidia/example",
            Some("openai-compatible:nvidia-nim"),
            Some("openai-compatible:nvidia-nim"),
            "nvidia-nim:nvidia/example",
        ),
        (
            "claude-sonnet-4",
            Some("copilot"),
            Some("copilot"),
            "copilot:claude-sonnet-4",
        ),
        (
            "composer-2.5",
            Some("cursor"),
            Some("cursor"),
            "cursor:composer-2.5",
        ),
        (
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            Some("bedrock"),
            Some("bedrock"),
            "bedrock:anthropic.claude-3-5-sonnet-20241022-v2:0",
        ),
        (
            "default",
            Some("antigravity"),
            Some("antigravity-https"),
            "antigravity:default",
        ),
    ];

    for (model, provider_key, api_method, expected) in cases {
        assert_eq!(
            MultiProvider::model_switch_request_for_session_route(model, provider_key, api_method),
            expected,
            "restore request should preserve route identity for {provider_key:?}/{api_method:?}"
        );
    }
}
#[test]
fn test_openrouter_and_compatible_profile_transition_invariants() {
    with_clean_provider_test_env(|| {
        let nvidia = crate::provider_catalog::openai_compatible_profile_by_id("nvidia-nim")
            .expect("NVIDIA NIM profile exists");

        save_test_openrouter_model_cache(
            "openrouter",
            "https://openrouter.ai/api/v1",
            &["openrouter/owl-alpha"],
        );

        crate::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
        crate::env::set_var(nvidia.api_key_env, "test-nvidia-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(None);
        let openrouter = test_openrouter_runtime().expect("OpenRouter provider should initialize");
        openrouter
            .set_model("openrouter/owl-alpha")
            .expect("OpenRouter default model should be selectable");

        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(None),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(Some(openrouter.clone())),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::OpenRouter),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        provider
            .set_model("nvidia-nim:nvidia/llama-3.1-nemotron-ultra-253b-v1")
            .expect("NVIDIA NIM model should be selectable");
        assert_eq!(provider.model(), "nvidia/llama-3.1-nemotron-ultra-253b-v1");
        assert!(Arc::ptr_eq(
            &provider
                .openrouter_provider()
                .expect("real OpenRouter remains"),
            &openrouter
        ));
        assert_eq!(
            provider
                .active_openrouter_execution_provider()
                .expect("active compatible runtime")
                .direct_openai_compatible_route_parts()
                .map(|(_provider, api_method, _detail)| api_method),
            Some("openai-compatible:nvidia-nim".to_string())
        );

        provider
            .set_model("openrouter:openrouter/owl-alpha")
            .expect("OpenRouter switch should select real OpenRouter slot");
        assert_eq!(provider.model(), "openrouter/owl-alpha");
        assert!(
            provider
                .active_openrouter_execution_provider()
                .expect("active OpenRouter runtime")
                .direct_openai_compatible_route_parts()
                .is_none(),
            "real OpenRouter route must not inherit compatible profile state"
        );

        provider
            .set_model("nvidia-nim:nvidia/llama-3.1-nemotron-ultra-253b-v1")
            .expect("cached compatible runtime should be selectable again");
        assert_eq!(provider.model(), "nvidia/llama-3.1-nemotron-ultra-253b-v1");
        assert!(
            provider
                .openrouter_provider()
                .expect("real OpenRouter remains")
                .direct_openai_compatible_route_parts()
                .is_none(),
            "compatible profile route must never overwrite the real OpenRouter runtime"
        );
    });
}
