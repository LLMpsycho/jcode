#[test]
fn test_openai_auth_mode_prefixed_model_switch_changes_credentials() {
    with_clean_provider_test_env(|| {
        let prev_runtime = std::env::var_os("JCODE_RUNTIME_PROVIDER");
        crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
        crate::auth::codex::upsert_account_from_tokens(
            "openai-1",
            "oauth-access-token",
            "oauth-refresh-token",
            None,
            None,
        )
        .expect("save OAuth account");

        let openai = test_openai_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(Some(Arc::clone(&openai) as Arc<dyn Provider>)),
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
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();

        // Route pinning is MultiProvider's job; per-pin token resolution is
        // covered by jcode-provider-openai-runtime's tests.
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::Auto,
            "default OpenAI credentials stay on the OAuth-first Auto pin"
        );

        provider
            .set_model("openai-api:gpt-5.5")
            .expect("API-key route should select the OpenAI API credentials");
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::ApiKey
        );

        provider
            .set_model("openai-oauth:gpt-5.5")
            .expect("OAuth route should switch back to Codex OAuth credentials");
        assert_eq!(
            openai.credential_mode(),
            jcode_provider_core::CredentialMode::OAuth
        );

        if let Some(prev_runtime) = prev_runtime {
            crate::env::set_var("JCODE_RUNTIME_PROVIDER", prev_runtime);
        } else {
            crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        }
    });
}
#[test]
fn test_initial_openai_provider_can_switch_to_anthropic_auth_routes() {
    with_clean_provider_test_env(|| {
        crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
        crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
            label: "claude-1".to_string(),
            access: "oauth-access-token".to_string(),
            refresh: "oauth-refresh-token".to_string(),
            expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
            email: None,
            subscription_type: Some("max".to_string()),
            scopes: vec!["user:inference".to_string()],
        })
        .expect("save Claude OAuth account");

        let anthropic = test_anthropic_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
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
            initial_provider: Some(ActiveProvider::OpenAI),
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();

        // Route pinning is MultiProvider's job; the concrete token resolution
        // for each pin is covered by jcode-provider-anthropic-runtime's
        // credential-mode tests.
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::Auto,
            "default (Auto) leaves the credential pin unset"
        );

        provider
            .set_model("claude-oauth:claude-opus-4-6")
            .expect("OAuth route should select Claude OAuth credentials");
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::OAuth
        );

        provider
            .set_model("claude-api:claude-opus-4-6")
            .expect("API route should select Anthropic API-key credentials");
        assert_eq!(
            anthropic.credential_mode(),
            jcode_provider_core::CredentialMode::ApiKey
        );
    });
}
#[test]
fn test_config_default_provider_anthropic_api_pins_api_credential() {
    use jcode_provider_core::{Provider, ResolvedCredential};
    // A config `default_provider = "anthropic-api"` is a routing decision that
    // also pins the OAuth-vs-API credential. Applying the default at startup
    // must leave the provider on the API-key route so the header auth tag and
    // model picker report "API Key", not the Auto/OAuth fallback.
    for (default_provider, expected, expect_oauth) in [
        ("anthropic-api", ResolvedCredential::ApiKey, false),
        ("claude-api", ResolvedCredential::ApiKey, false),
        ("claude", ResolvedCredential::Oauth, true),
        ("anthropic", ResolvedCredential::Oauth, true),
    ] {
        with_clean_provider_test_env(|| {
            crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
            crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
                label: "claude-1".to_string(),
                access: "oauth-access-token".to_string(),
                refresh: "oauth-refresh-token".to_string(),
                expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
                email: None,
                subscription_type: Some("max".to_string()),
                scopes: vec!["user:inference".to_string()],
            })
            .expect("save Claude OAuth account");

            let anthropic = test_anthropic_runtime();
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
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
            let rt = enter_test_runtime();
            let _runtime_guard = rt.enter();

            provider
                .set_config_default_model("claude-opus-4-6", Some(default_provider))
                .unwrap_or_else(|e| {
                    panic!("default_provider '{default_provider}' should apply: {e}")
                });

            assert_eq!(
                provider.active_provider(),
                ActiveProvider::Claude,
                "default_provider '{default_provider}' routes to Claude",
            );
            assert_eq!(
                provider.active_explicit_credential(),
                (!expect_oauth).then_some(ResolvedCredential::ApiKey),
                "default_provider '{default_provider}' explicit-pin visibility",
            );
            assert_eq!(
                anthropic.credential_mode(),
                if expect_oauth {
                    // "claude"/"anthropic" leave Auto (OAuth-first) rather than
                    // pinning OAuth explicitly.
                    jcode_provider_core::CredentialMode::Auto
                } else {
                    jcode_provider_core::CredentialMode::ApiKey
                },
                "default_provider '{default_provider}' should resolve {expected:?}",
            );
        });
    }
}
#[test]
fn test_config_default_model_with_credential_prefix_applies_model_and_pin() {
    use jcode_provider_core::{Provider, ResolvedCredential};
    // The model picker saves default_model as a full spec like
    // `claude-api:claude-opus-4-6`. Startup must parse the prefix (routing +
    // credential pin) instead of handing the raw spec to the Anthropic
    // provider, which would reject it and silently keep the fallback default.
    for (spec, expect_oauth) in [
        ("claude-api:claude-opus-4-6", false),
        ("claude-oauth:claude-opus-4-6", true),
        ("claude:claude-opus-4-6", true),
    ] {
        with_clean_provider_test_env(|| {
            crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
            crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
                label: "claude-1".to_string(),
                access: "oauth-access-token".to_string(),
                refresh: "oauth-refresh-token".to_string(),
                expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
                email: None,
                subscription_type: Some("max".to_string()),
                scopes: vec!["user:inference".to_string()],
            })
            .expect("save Claude OAuth account");

            let anthropic = test_anthropic_runtime();
            let provider = MultiProvider {
                claude: RwLock::new(None),
                anthropic: RwLock::new(Some(Arc::clone(&anthropic) as Arc<dyn Provider>)),
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
            let rt = enter_test_runtime();
            let _runtime_guard = rt.enter();

            provider
                .set_config_default_model(spec, Some("anthropic-api"))
                .unwrap_or_else(|e| panic!("default_model '{spec}' should apply: {e}"));

            assert_eq!(
                provider.active_provider(),
                ActiveProvider::Claude,
                "default_model '{spec}' routes to Claude",
            );
            assert_eq!(
                provider.model(),
                "claude-opus-4-6",
                "default_model '{spec}' should set the bare model id",
            );
            // `claude-api:` must pin the API key; `claude:`/`claude-oauth:`
            // resolve OAuth-first (Auto or explicit OAuth respectively), so the
            // pin must not be ApiKey. Concrete token resolution per pin is
            // covered by jcode-provider-anthropic-runtime's tests.
            if expect_oauth {
                assert_ne!(
                    anthropic.credential_mode(),
                    jcode_provider_core::CredentialMode::ApiKey,
                    "default_model '{spec}' must not pin the API key (expected {:?})",
                    ResolvedCredential::Oauth,
                );
            } else {
                assert_eq!(
                    anthropic.credential_mode(),
                    jcode_provider_core::CredentialMode::ApiKey,
                    "default_model '{spec}' should resolve {:?}",
                    ResolvedCredential::ApiKey,
                );
            }
        });
    }
}
#[test]
fn test_multi_provider_fork_switch_request_preserves_route_identity_state_space() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("OPENAI_API_KEY", "sk-test-openai-api-key");
        crate::auth::codex::upsert_account_from_tokens(
            "openai-1",
            "oauth-access-token",
            "oauth-refresh-token",
            None,
            None,
        )
        .expect("save OpenAI OAuth account");
        let openai = test_openai_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(Some(openai)),
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

        let generation_before = crate::provider::pricing::auth_pricing_generation();
        provider
            .set_model("openai-api:gpt-5.5")
            .expect("API-key route should be selectable");
        let api_route_memo_key = provider.routes_memo_key();
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before,
            "restoring an in-memory credential route must not invalidate global catalogs"
        );
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "openai-api:gpt-5.5"
        );
        let _fork = provider.fork();
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before,
            "forking a provider must not invalidate global catalogs"
        );
        provider
            .set_model("openai-oauth:gpt-5.5")
            .expect("OAuth route should be selectable");
        assert_ne!(
            provider.routes_memo_key(),
            api_route_memo_key,
            "OAuth and API-key catalogs need distinct shared memo keys"
        );
        assert_eq!(
            crate::provider::pricing::auth_pricing_generation(),
            generation_before
        );
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "openai-oauth:gpt-5.5"
        );
    });

    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-api-key");
        crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
            label: "claude-1".to_string(),
            access: "oauth-access-token".to_string(),
            refresh: "oauth-refresh-token".to_string(),
            expires: chrono::Utc::now().timestamp_millis() + 3_600_000,
            email: None,
            subscription_type: Some("max".to_string()),
            scopes: vec!["user:inference".to_string()],
        })
        .expect("save Claude OAuth account");
        let anthropic = test_anthropic_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(Some(anthropic)),
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
            .set_model("claude-oauth:claude-opus-4-6")
            .expect("OAuth route should be selectable");
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "claude-oauth:claude-opus-4-6"
        );
        provider
            .set_model("claude-api:claude-opus-4-6")
            .expect("API-key route should be selectable");
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "claude-api:claude-opus-4-6"
        );
    });

    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("CEREBRAS_API_KEY", "test-cerebras-key");
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
            .set_model("cerebras:qwen-3-235b-a22b-instruct-2507")
            .expect("profile-prefixed Cerebras route should be selectable");
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "cerebras:qwen-3-235b-a22b-instruct-2507"
        );
    });

    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
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
                .set_model("openrouter:openai/gpt-5.4@OpenAI")
                .expect("OpenRouter provider-pinned route should be selectable");
            assert_eq!(
                provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
                "openrouter:openai/gpt-5.4@OpenAI"
            );
        })
    });
}
