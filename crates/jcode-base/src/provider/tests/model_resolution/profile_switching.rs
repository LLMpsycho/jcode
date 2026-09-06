#[test]
fn test_set_model_accepts_bare_openai_openrouter_pin_when_openrouter_available() {
    with_clean_provider_test_env(|| {
        with_env_var("OPENROUTER_API_KEY", "test-openrouter-key", || {
            let openrouter =
                test_openrouter_runtime().expect("openrouter provider should initialize");
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
                active: RwLock::new(ActiveProvider::OpenAI),
                use_claude_cli: false,
                startup_notices: RwLock::new(Vec::new()),
                initial_provider: None,
                routes_memo: std::sync::Mutex::new(None),
                route_pinned: std::sync::atomic::AtomicBool::new(false),
                private_session: std::sync::atomic::AtomicBool::new(false),
                post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };

            provider
                .set_model("gpt-5.4@OpenAI")
                .expect("bare pinned OpenRouter spec should normalize");

            assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
            assert_eq!(provider.model(), "openai/gpt-5.4");
        })
    });
}
#[test]
fn test_active_compatible_route_treats_claude_like_bare_model_as_provider_local() {
    with_clean_provider_test_env(|| {
        with_env_var("OPENROUTER_API_KEY", "test-openrouter-key", || {
            with_env_var("JCODE_OPENROUTER_PROVIDER_FEATURES", "0", || {
                with_env_var(
                    "JCODE_OPENROUTER_API_BASE",
                    "https://compat.example.test/v1",
                    || {
                        let openrouter = test_openrouter_runtime()
                            .expect("custom compatible provider should initialize");
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
                            openai_compatible_profiles: RwLock::new(
                                std::collections::HashMap::new(),
                            ),
                            active_openai_compatible_profile: RwLock::new(None),
                            active: RwLock::new(ActiveProvider::OpenRouter),
                            use_claude_cli: false,
                            startup_notices: RwLock::new(Vec::new()),
                            initial_provider: Some(ActiveProvider::OpenRouter),
                            routes_memo: std::sync::Mutex::new(None),
                            route_pinned: std::sync::atomic::AtomicBool::new(false),
                            private_session: std::sync::atomic::AtomicBool::new(false),
                            post_auth_refreshes_pending: Arc::new(
                                std::sync::atomic::AtomicUsize::new(0),
                            ),
                        };

                        provider.set_model("claude-opus4.6-thinking").expect(
                            "active OpenAI-compatible route should accept opaque model IDs",
                        );

                        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
                        assert_eq!(provider.model(), "claude-opus4.6-thinking");
                    },
                )
            })
        })
    });
}
#[test]
fn test_bare_model_served_by_active_catalog_profile_stays_on_that_profile() {
    with_clean_provider_test_env(|| {
        let profile = crate::provider_catalog::OPENCODE_GO_PROFILE;
        crate::env::set_var(profile.api_key_env, "test-opencode-go-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(profile));
        let runtime = test_openrouter_runtime().expect("OpenCode Go runtime should initialize");
        runtime
            .set_model("kimi-k2.5")
            .expect("initial OpenCode Go model should be selectable");
        let provider = test_multi_provider_with_openrouter(runtime);

        assert!(provider.model_routes().iter().any(|route| {
            route.model == "deepseek-v4-flash"
                && route.api_method == "openai-compatible:opencode-go"
                && route.available
        }));
        provider
            .set_model("deepseek-v4-flash")
            .expect("bare catalog model should stay on OpenCode Go");

        assert_eq!(provider.model(), "deepseek-v4-flash");
        assert_eq!(
            provider
                .active_openrouter_execution_provider()
                .and_then(|runtime| runtime.direct_openai_compatible_route_parts())
                .map(|(_, api_method, _)| api_method),
            Some("openai-compatible:opencode-go".to_string())
        );
    });
}
#[test]
fn test_bare_model_absent_from_active_catalog_profile_still_rebinds_to_openrouter() {
    with_clean_provider_test_env(|| {
        let profile = crate::provider_catalog::OPENCODE_GO_PROFILE;
        crate::env::set_var(profile.api_key_env, "test-opencode-go-key");
        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(profile));
        let runtime = test_openrouter_runtime().expect("OpenCode Go runtime should initialize");
        runtime
            .set_model("kimi-k2.5")
            .expect("initial OpenCode Go model should be selectable");
        let provider = test_multi_provider_with_openrouter(runtime);

        let error = provider
            .set_model("not-in-opencode-go-catalog")
            .expect_err("unknown bare model should still attempt native OpenRouter");

        assert!(
            error.to_string().contains("OPENROUTER_API_KEY"),
            "unexpected rebind error: {error:#}"
        );
        assert_eq!(provider.model(), "kimi-k2.5");
        assert_eq!(
            provider
                .active_openrouter_execution_provider()
                .and_then(|runtime| runtime.direct_openai_compatible_route_parts())
                .map(|(_, api_method, _)| api_method),
            Some("openai-compatible:opencode-go".to_string())
        );
    });
}
#[test]
fn test_active_compatible_route_preserves_custom_at_sign_model_ids() {
    with_clean_provider_test_env(|| {
        with_env_var("OPENROUTER_API_KEY", "test-openrouter-key", || {
            with_env_var("JCODE_OPENROUTER_PROVIDER_FEATURES", "0", || {
                with_env_var(
                    "JCODE_OPENROUTER_API_BASE",
                    "https://compat.example.test/v1",
                    || {
                        let openrouter = test_openrouter_runtime()
                            .expect("custom compatible provider should initialize");
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
                            openai_compatible_profiles: RwLock::new(
                                std::collections::HashMap::new(),
                            ),
                            active_openai_compatible_profile: RwLock::new(None),
                            active: RwLock::new(ActiveProvider::OpenRouter),
                            use_claude_cli: false,
                            startup_notices: RwLock::new(Vec::new()),
                            initial_provider: Some(ActiveProvider::OpenRouter),
                            routes_memo: std::sync::Mutex::new(None),
                            route_pinned: std::sync::atomic::AtomicBool::new(false),
                            private_session: std::sync::atomic::AtomicBool::new(false),
                            post_auth_refreshes_pending: Arc::new(
                                std::sync::atomic::AtomicUsize::new(0),
                            ),
                        };

                        provider
                            .set_model("gpt-5.4@OpenAI")
                            .expect("custom compatible provider should preserve @ in model IDs");

                        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
                        assert_eq!(provider.model(), "gpt-5.4@OpenAI");
                    },
                )
            })
        })
    });
}
#[test]
fn test_config_default_provider_openai_compatible_keeps_gpt_model_provider_local() {
    with_clean_provider_test_env(|| {
        with_env_var(
            "JCODE_OPENAI_COMPAT_API_BASE",
            "https://compat.example.test/v1",
            || {
                with_env_var("JCODE_OPENAI_COMPAT_API_KEY_NAME", "OPENAI_API_KEY", || {
                    with_env_var("OPENAI_API_KEY", "test-compatible-key", || {
                        crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(
                            crate::provider_catalog::OPENAI_COMPAT_PROFILE,
                        ));
                        let openrouter = test_openrouter_runtime()
                            .expect("OpenAI-compatible provider should initialize");
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
                            openai_compatible_profiles: RwLock::new(
                                std::collections::HashMap::new(),
                            ),
                            active_openai_compatible_profile: RwLock::new(None),
                            active: RwLock::new(ActiveProvider::OpenRouter),
                            use_claude_cli: false,
                            startup_notices: RwLock::new(Vec::new()),
                            initial_provider: None,
                            routes_memo: std::sync::Mutex::new(None),
                            route_pinned: std::sync::atomic::AtomicBool::new(false),
                            private_session: std::sync::atomic::AtomicBool::new(false),
                            post_auth_refreshes_pending: Arc::new(
                                std::sync::atomic::AtomicUsize::new(0),
                            ),
                        };

                        provider
                            .set_config_default_model("gpt-5.5", Some("openai-compatible"))
                            .expect(
                                "configured OpenAI-compatible default model should apply locally",
                            );

                        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
                        assert_eq!(provider.model(), "gpt-5.5");
                        assert_eq!(
                            crate::provider_catalog::runtime_provider_display_name(provider.name()),
                            "OpenAI-compatible"
                        );
                    })
                })
            },
        )
    });
}
#[test]
fn test_custom_compatible_model_routes_do_not_request_openrouter_rewrite() {
    with_clean_provider_test_env(|| {
        with_env_var("OPENROUTER_API_KEY", "test-openrouter-key", || {
            with_env_var("JCODE_OPENROUTER_PROVIDER_FEATURES", "0", || {
                with_env_var(
                    "JCODE_OPENROUTER_API_BASE",
                    "https://compat.example.test/v1",
                    || {
                        let openrouter = test_openrouter_runtime()
                            .expect("custom compatible provider should initialize");
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
                            openai_compatible_profiles: RwLock::new(
                                std::collections::HashMap::new(),
                            ),
                            active_openai_compatible_profile: RwLock::new(None),
                            active: RwLock::new(ActiveProvider::OpenRouter),
                            use_claude_cli: false,
                            startup_notices: RwLock::new(Vec::new()),
                            initial_provider: Some(ActiveProvider::OpenRouter),
                            routes_memo: std::sync::Mutex::new(None),
                            route_pinned: std::sync::atomic::AtomicBool::new(false),
                            private_session: std::sync::atomic::AtomicBool::new(false),
                            post_auth_refreshes_pending: Arc::new(
                                std::sync::atomic::AtomicUsize::new(0),
                            ),
                        };

                        provider.set_model("claude-opus4.6-thinking").expect(
                            "active OpenAI-compatible route should accept opaque model IDs",
                        );

                        let routes = provider.model_routes();
                        assert!(routes.iter().any(|route| {
                            route.model == "claude-opus4.6-thinking"
                                && route.provider == "OpenAI-compatible"
                                && route.api_method == "openai-compatible"
                        }));
                        assert!(!routes.iter().any(|route| {
                            route.model == "claude-opus4.6-thinking"
                                && route.provider == "auto"
                                && route.api_method == "openrouter"
                        }));
                    },
                )
            })
        })
    });
}
#[test]
fn test_configured_direct_compatible_profiles_are_listed_without_openrouter_key() {
    with_clean_provider_test_env(|| {
        with_env_var("DEEPSEEK_API_KEY", "test-deepseek-key", || {
            with_env_var("KIMI_API_KEY", "test-kimi-key", || {
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

                let routes = provider.model_routes();
                assert!(routes.iter().any(|route| {
                    route.model == "deepseek-v4-flash"
                        && route.provider == "DeepSeek"
                        && route.api_method == "openai-compatible:deepseek"
                        && route.available
                }));
                assert!(routes.iter().any(|route| {
                    route.model == "deepseek-v4-pro"
                        && route.provider == "DeepSeek"
                        && route.api_method == "openai-compatible:deepseek"
                        && route.available
                }));
                assert!(routes.iter().any(|route| {
                    route.model == "kimi-for-coding"
                        && route.provider == "Kimi Code"
                        && route.api_method == "openai-compatible:kimi"
                        && route.available
                }));
                assert!(
                    !routes
                        .iter()
                        .any(|route| route.model == "openrouter models")
                );
            })
        })
    });
}
#[test]
fn test_named_config_provider_models_appear_in_picker_and_are_selectable() {
    // Issue #444: models declared under `[[providers.<name>.models]]` in
    // config.toml must appear in the model picker with a route back to that
    // profile, and selecting the emitted `<name>:<model>` spec must bind the
    // named profile runtime.
    with_clean_provider_test_env(|| {
        let jcode_home = std::env::var_os("JCODE_HOME").expect("test JCODE_HOME should be set");
        std::fs::write(
            std::path::PathBuf::from(jcode_home).join("config.toml"),
            r#"
[provider]
default_provider = "my-gateway"
default_model = "vendor/my-model"

[providers.my-gateway]
type = "openai-compatible"
base_url = "https://example.com/proxy/openai"
auth = "none"
default_model = "vendor/my-model"

[[providers.my-gateway.models]]
id = "vendor/my-model"
context_window = 230000
input = ["text"]

[[providers.my-gateway.models]]
id = "vendor/image-only-model"
input = ["image"]
"#,
        )
        .expect("write test config.toml");
        crate::config::invalidate_config_cache();

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
            active: RwLock::new(ActiveProvider::Claude),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        // The picker must offer the text-capable configured model with a
        // route back to the named profile, and exclude image-only models.
        let routes = provider.model_routes();
        let route = routes
            .iter()
            .find(|route| route.model == "vendor/my-model")
            .unwrap_or_else(|| {
                panic!("configured named-profile model missing from picker: {routes:?}")
            });
        assert_eq!(route.provider, "my-gateway");
        assert_eq!(route.api_method, "openai-compatible:my-gateway");
        assert!(route.available);
        assert!(
            !routes
                .iter()
                .any(|route| route.model == "vendor/image-only-model"),
            "image-only configured models must not be listed"
        );

        // Selecting the picker's spec must bind the named profile runtime.
        provider
            .set_model("my-gateway:vendor/my-model")
            .expect("named profile model spec must be selectable");
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(provider.model(), "vendor/my-model");

        // And the configured default_provider/default_model pair must bind the
        // profile directly (same bug class as issue #448).
        let provider2 = MultiProvider {
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
            active: RwLock::new(ActiveProvider::Claude),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: None,
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        provider2
            .set_config_default_model("vendor/my-model", Some("my-gateway"))
            .expect("configured named-profile default must bind the profile runtime");
        assert_eq!(provider2.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(provider2.model(), "vendor/my-model");
    });
}
#[test]
fn test_config_default_provider_deepseek_applies_without_openrouter_key() {
    // Issue #448: `default_provider = "deepseek"` + `default_model =
    // "deepseek-v4-pro"` with only DEEPSEEK_API_KEY set must bind the DeepSeek
    // profile runtime. The generic OpenRouter path would try to rebind the
    // slot to a plain OpenRouter API-key runtime, fail (no OPENROUTER_API_KEY),
    // and silently fall back to the auto-detected default provider.
    with_clean_provider_test_env(|| {
        with_env_var("DEEPSEEK_API_KEY", "test-deepseek-key", || {
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(Some(test_anthropic_runtime())),
                openai: RwLock::new(None),
                copilot_api: RwLock::new(None),
                antigravity: RwLock::new(None),
                gemini: RwLock::new(None),
                cursor: RwLock::new(None),
                bedrock: RwLock::new(None),
                openrouter: RwLock::new(None),
                openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
                active_openai_compatible_profile: RwLock::new(None),
                active: RwLock::new(ActiveProvider::Claude),
                use_claude_cli: false,
                startup_notices: RwLock::new(Vec::new()),
                initial_provider: None,
                routes_memo: std::sync::Mutex::new(None),
                route_pinned: std::sync::atomic::AtomicBool::new(false),
                private_session: std::sync::atomic::AtomicBool::new(false),
                post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };

            provider
                .set_config_default_model("deepseek-v4-pro", Some("deepseek"))
                .expect("configured DeepSeek default must bind the profile runtime");
            assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
            assert_eq!(provider.model(), "deepseek-v4-pro");
            assert_eq!(provider.display_name(), "DeepSeek");
        })
    });
}
#[test]
fn test_profile_prefixed_model_switch_reinitializes_direct_compatible_runtime() {
    with_clean_provider_test_env(|| {
        with_env_var("DEEPSEEK_API_KEY", "test-deepseek-key", || {
            with_env_var("KIMI_API_KEY", "test-kimi-key", || {
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

                provider
                    .set_model("deepseek:deepseek-v4-pro")
                    .expect("DeepSeek profile-prefixed model should initialize direct provider");
                assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
                assert_eq!(provider.model(), "deepseek-v4-pro");
                // `display_name` resolves through the active execution runtime
                // (registry), which is the production display path since the
                // compat-profile/OpenRouter slot split.
                assert_eq!(provider.display_name(), "DeepSeek");

                provider
                    .set_model("kimi:kimi-for-coding")
                    .expect("Kimi profile-prefixed model should reinitialize direct provider");
                assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
                assert_eq!(provider.model(), "kimi-for-coding");
                assert_eq!(provider.display_name(), "Kimi Code");
            })
        })
    });
}
