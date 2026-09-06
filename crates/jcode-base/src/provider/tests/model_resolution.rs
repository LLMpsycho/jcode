fn test_multi_provider_with_openrouter(openrouter: Arc<dyn Provider>) -> MultiProvider {
    MultiProvider {
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
    }
}

#[test]
fn test_deepseek_direct_profile_supports_reasoning_effort_via_multi_provider() {
    with_clean_provider_test_env(|| {
        with_env_var("DEEPSEEK_API_KEY", "test-deepseek-key", || {
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

            assert_eq!(
                provider.available_efforts(),
                vec![
                    "none",
                    "low",
                    "medium",
                    "high",
                    "max",
                    "swarm",
                    "swarm-deep"
                ]
            );
            provider
                .set_reasoning_effort("max")
                .expect("/effort max should work for direct DeepSeek profile");
            assert_eq!(provider.reasoning_effort().as_deref(), Some("max"));
        })
    });
}

#[test]
fn test_explicit_copilot_prefix_treats_claude_like_model_as_provider_local() {
    with_clean_provider_test_env(|| {
        let copilot = test_copilot_runtime();
        let provider = MultiProvider {
            claude: RwLock::new(None),
            anthropic: RwLock::new(None),
            openai: RwLock::new(None),
            copilot_api: RwLock::new(Some(copilot)),
            antigravity: RwLock::new(None),
            gemini: RwLock::new(None),
            cursor: RwLock::new(None),
            bedrock: RwLock::new(None),
            openrouter: RwLock::new(None),
            openai_compatible_profiles: RwLock::new(std::collections::HashMap::new()),
            active_openai_compatible_profile: RwLock::new(None),
            active: RwLock::new(ActiveProvider::Copilot),
            use_claude_cli: false,
            startup_notices: RwLock::new(Vec::new()),
            initial_provider: Some(ActiveProvider::Copilot),
            routes_memo: std::sync::Mutex::new(None),
            route_pinned: std::sync::atomic::AtomicBool::new(false),
            private_session: std::sync::atomic::AtomicBool::new(false),
            post_auth_refreshes_pending: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };

        provider
            .set_model("copilot:claude-opus-4.6")
            .expect("explicit Copilot route should accept Copilot's dotted Claude model ID");

        assert_eq!(provider.active_provider(), ActiveProvider::Copilot);
        assert_eq!(provider.model(), "claude-opus-4.6");
    });
}

#[test]
fn test_initial_provider_does_not_block_provider_specific_model_switch() {
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
                cursor: RwLock::new(Some(test_cursor_runtime())),
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

            provider
                .set_model("cursor:gpt-5")
                .expect("an initial OpenRouter selection must allow switching to Cursor");

            assert_eq!(provider.active_provider(), ActiveProvider::Cursor);
            assert_eq!(provider.model(), "gpt-5");
        })
    });
}

#[test]
fn test_provider_for_model_unknown() {
    assert_eq!(provider_for_model("unknown-model"), None);
}

#[test]
fn test_provider_for_model_cursor() {
    assert_eq!(provider_for_model("composer-2-fast"), Some("cursor"));
    assert_eq!(provider_for_model("composer-2"), Some("cursor"));
    assert_eq!(provider_for_model("sonnet-4.6"), Some("cursor"));
    assert_eq!(provider_for_model("gpt-5"), Some("openai"));
}

#[test]
fn test_context_limit_spark_vs_codex() {
    assert_eq!(
        context_limit_for_model("gpt-5.3-codex-spark"),
        Some(128_000)
    );
    assert_eq!(context_limit_for_model("gpt-5.5"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5.3-codex"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5.2-codex"), Some(272_000));
    assert_eq!(context_limit_for_model("gpt-5-codex"), Some(272_000));
}

#[test]
fn test_context_limit_gpt_5_4() {
    assert_eq!(context_limit_for_model("gpt-5.4"), Some(1_000_000));
    assert_eq!(context_limit_for_model("gpt-5.4-pro"), Some(1_000_000));
    assert_eq!(context_limit_for_model("gpt-5.4[1m]"), Some(1_000_000));
}

#[test]
fn test_context_limit_respects_provider_hint() {
    assert_eq!(
        context_limit_for_model_with_provider("gpt-5.4", Some("openai")),
        Some(1_000_000)
    );
    assert_eq!(
        context_limit_for_model_with_provider("gpt-5.4", Some("copilot")),
        Some(128_000)
    );
    assert_eq!(
        context_limit_for_model_with_provider("claude-sonnet-4-6[1m]", Some("claude")),
        Some(1_048_576)
    );
}

#[test]
fn test_resolve_model_capabilities_uses_provider_hint() {
    let openai = resolve_model_capabilities("gpt-5.4", Some("openai"));
    assert_eq!(openai.provider.as_deref(), Some("openai"));
    assert_eq!(openai.context_window, Some(1_000_000));

    let copilot = resolve_model_capabilities("gpt-5.4", Some("copilot"));
    assert_eq!(copilot.provider.as_deref(), Some("copilot"));
    assert_eq!(copilot.context_window, Some(128_000));

    let gemini = resolve_model_capabilities("gemini-2.5-pro", Some("gemini"));
    assert_eq!(gemini.provider.as_deref(), Some("gemini"));
    assert_eq!(gemini.context_window, Some(1_000_000));
}

#[test]
fn test_normalize_model_id_strips_1m_suffix() {
    assert_eq!(models::normalize_model_id("gpt-5.4[1m]"), "gpt-5.4");
    assert_eq!(models::normalize_model_id(" GPT-5.4[1M] "), "gpt-5.4");
}

#[test]
fn test_merge_openai_model_ids_appends_dynamic_oauth_models() {
    let models = models::merge_openai_model_ids(vec![
        "gpt-5.4".to_string(),
        "gpt-5.4-fast-preview".to_string(),
        "gpt-5.4-fast-preview".to_string(),
        " gpt-5.5-experimental ".to_string(),
    ]);

    assert!(models.iter().any(|model| model == "gpt-5.4"));
    assert!(models.iter().any(|model| model == "gpt-5.4-fast-preview"));
    assert!(models.iter().any(|model| model == "gpt-5.5-experimental"));
    assert_eq!(
        models
            .iter()
            .filter(|model| model.as_str() == "gpt-5.4-fast-preview")
            .count(),
        1
    );
}

#[test]
fn test_merge_anthropic_model_ids_appends_dynamic_models() {
    let models = models::merge_anthropic_model_ids(vec![
        "claude-opus-4-6".to_string(),
        "claude-sonnet-5-preview".to_string(),
        "claude-sonnet-5-preview".to_string(),
        " claude-haiku-5-beta ".to_string(),
    ]);

    assert!(models.iter().any(|model| model == "claude-opus-4-6"));
    assert!(models.iter().any(|model| model == "claude-opus-4-6[1m]"));
    assert!(
        models
            .iter()
            .any(|model| model == "claude-sonnet-5-preview")
    );
    assert!(models.iter().any(|model| model == "claude-haiku-5-beta"));
    assert_eq!(
        models
            .iter()
            .filter(|model| model.as_str() == "claude-sonnet-5-preview")
            .count(),
        1
    );
}

#[test]
fn test_parse_anthropic_model_catalog_reads_context_limits() {
    let data = serde_json::json!({
        "data": [
            {
                "id": "claude-opus-4-6",
                "max_input_tokens": 1_048_576
            },
            {
                "id": "claude-sonnet-5-preview",
                "max_input_tokens": 333_000
            }
        ]
    });

    let catalog = models::parse_anthropic_model_catalog(&data);
    assert!(
        catalog
            .available_models
            .contains(&"claude-opus-4-6".to_string())
    );
    assert!(
        catalog
            .available_models
            .contains(&"claude-sonnet-5-preview".to_string())
    );
    assert_eq!(
        catalog.context_limits.get("claude-opus-4-6"),
        Some(&1_048_576)
    );
    assert_eq!(
        catalog.context_limits.get("claude-sonnet-5-preview"),
        Some(&333_000)
    );
}

#[test]
fn test_context_limit_claude() {
    with_clean_provider_test_env(|| {
        assert_eq!(context_limit_for_model("claude-opus-4-6"), Some(200_000));
        assert_eq!(context_limit_for_model("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(
            context_limit_for_model("claude-opus-4-6[1m]"),
            Some(1_048_576)
        );
        assert_eq!(
            context_limit_for_model("claude-sonnet-4-6[1m]"),
            Some(1_048_576)
        );
        // Opus 4.8 / 4.7 expose a 1M window natively (no `[1m]` opt-in needed),
        // matching the live Anthropic catalog's `max_input_tokens: 1000000`.
        assert_eq!(context_limit_for_model("claude-opus-4-8"), Some(1_000_000));
        assert_eq!(
            context_limit_for_model("claude-opus-4-8[1m]"),
            Some(1_000_000)
        );
        assert_eq!(context_limit_for_model("claude-opus-4-7"), Some(1_000_000));
    });
}

#[test]
fn test_context_limit_dynamic_cache() {
    populate_context_limits(
        [("test-model-xyz".to_string(), 64_000)]
            .into_iter()
            .collect(),
    );
    assert_eq!(context_limit_for_model("test-model-xyz"), Some(64_000));
}

// --- Migrated from the OpenRouter runtime tests: these exercise MultiProvider
// --- routing/identity with a real OpenRouter runtime via the registry.

struct OrEnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl OrEnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        crate::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        crate::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for OrEnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            crate::env::set_var(self.key, previous);
        } else {
            crate::env::remove_var(self.key);
        }
    }
}

fn isolate_openrouter_autodetect_env_or() -> Vec<OrEnvVarGuard> {
    let mut guards = vec![
        OrEnvVarGuard::remove("JCODE_OPENROUTER_API_BASE"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_API_KEY_NAME"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_ENV_FILE"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_DYNAMIC_BEARER_PROVIDER"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_MODEL"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_ALLOW_NO_AUTH"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_TRANSPORT_STATE"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_PROVIDER_FEATURES"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_MODEL_CATALOG"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_AUTH_HEADER"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_AUTH_HEADER_NAME"),
        OrEnvVarGuard::remove("JCODE_OPENROUTER_STATIC_MODELS"),
        OrEnvVarGuard::remove("JCODE_ACTIVE_PROVIDER"),
        OrEnvVarGuard::remove("JCODE_RUNTIME_PROVIDER"),
        OrEnvVarGuard::remove("JCODE_NAMED_PROVIDER_PROFILE"),
        OrEnvVarGuard::remove("JCODE_PROVIDER_PROFILE_NAME"),
        OrEnvVarGuard::remove("JCODE_PROVIDER_PROFILE_ACTIVE"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_API_BASE"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_API_KEY_NAME"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_ENV_FILE"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_SETUP_URL"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_DEFAULT_MODEL"),
        OrEnvVarGuard::remove("JCODE_OPENAI_COMPAT_LOCAL_ENABLED"),
        OrEnvVarGuard::remove("OPENROUTER_API_KEY"),
    ];
    guards.extend(
        crate::provider_catalog::openai_compatible_profiles()
            .iter()
            .map(|profile| OrEnvVarGuard::remove(profile.api_key_env)),
    );
    guards
}

fn spawn_single_response_chat_server_or() -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake provider server");
    let addr = listener.local_addr().expect("fake provider addr");
    let (request_tx, request_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fake provider request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("set read timeout");
        let mut request = vec![0u8; 16384];
        let n = stream.read(&mut request).unwrap_or(0);
        let request = String::from_utf8_lossy(&request[..n]).into_owned();
        let _ = request_tx.send(request);

        let body = "data: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write fake provider response");
    });

    (format!("http://{addr}/v1"), request_rx)
}

#[test]
fn default_named_openai_compatible_provider_uses_direct_compatible_request_path() {
    let _lock = crate::storage::lock_test_env();
    // These tests construct MultiProvider directly (not via
    // with_clean_provider_test_env), so make sure the external runtime
    // factories are registered before startup paths need them.
    register_test_external_runtimes();
    let temp = tempfile::TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = OrEnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = OrEnvVarGuard::set("HOME", temp.path());
    let _appdata = OrEnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env_or();
    let _key = OrEnvVarGuard::set("TEST_DEFAULT_COMPAT_KEY", "test-key");
    let (api_base, request_rx) = spawn_single_response_chat_server_or();

    std::fs::create_dir_all(&jcode_home).expect("create test config dir");
    std::fs::write(
        jcode_home.join("config.toml"),
        format!(
            r#"
[provider]
default_provider = "my-gateway"

[providers.my-gateway]
type = "openai-compatible"
base_url = "{api_base}"
api_key_env = "TEST_DEFAULT_COMPAT_KEY"
default_model = "opaque/model@id"
model_catalog = false
"#
        ),
    )
    .expect("write test config");
    crate::config::invalidate_config_cache();

    let provider = MultiProvider::new_with_auth_status(crate::auth::AuthStatus::default());
    assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
    let openrouter = provider
        .openrouter_provider()
        .expect("openrouter execution slot");
    assert!(
        !openrouter.supports_provider_routing_features(),
        "named openai-compatible defaults must not use OpenRouter provider routing features"
    );
    assert_eq!(
        openrouter
            .direct_openai_compatible_route_parts()
            .as_ref()
            .map(|parts| parts.1.as_str()),
        Some("openai-compatible:my-gateway")
    );

    let messages = vec![crate::message::Message {
        role: crate::message::Role::User,
        content: vec![crate::message::ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = openrouter
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = futures::StreamExt::next(&mut stream).await {
            event.expect("stream event should parse");
        }
    });

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.starts_with("POST /v1/chat/completions "),
        "unexpected chat request: {request}"
    );
    assert!(
        request.contains(r#""model":"opaque/model@id""#),
        "request should use named profile default model: {request}"
    );
    assert!(
        request.contains(r#""stream_options":{"include_usage":true}"#),
        "named compatible stream must request the terminal usage chunk: {request}"
    );
    assert!(
        !request.contains(r#""provider":"#),
        "direct OpenAI-compatible request must not include OpenRouter provider routing object: {request}"
    );
    assert!(
        !request.contains("HTTP-Referer") && !request.contains("X-Title"),
        "direct OpenAI-compatible request must not include OpenRouter-only headers: {request}"
    );
}

/// Regression for issue #304: a `default_provider` pointing at an
/// `openai-compatible` profile must build requests with the direct
/// OpenAI-compatible request shape, NOT the OpenRouter request builder, even
/// when `model_catalog` is left enabled (the default). Using the OpenRouter
/// builder leaks the `provider` routing object / OpenRouter-only headers and
/// causes strict third-party gateways to reject the request with
/// 400 "Unrecognized chat message".
#[test]
fn default_named_openai_compatible_with_catalog_uses_direct_compatible_request_path() {
    let _lock = crate::storage::lock_test_env();
    // These tests construct MultiProvider directly (not via
    // with_clean_provider_test_env), so make sure the external runtime
    // factories are registered before startup paths need them.
    register_test_external_runtimes();
    let temp = tempfile::TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = OrEnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = OrEnvVarGuard::set("HOME", temp.path());
    let _appdata = OrEnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env_or();
    let _key = OrEnvVarGuard::set("TEST_DEFAULT_COMPAT_KEY", "test-key");
    let (api_base, request_rx) = spawn_single_response_chat_server_or();

    std::fs::create_dir_all(&jcode_home).expect("create test config dir");
    std::fs::write(
        jcode_home.join("config.toml"),
        format!(
            r#"
[provider]
default_provider = "my-gateway"

[providers.my-gateway]
type = "openai-compatible"
base_url = "{api_base}"
api_key_env = "TEST_DEFAULT_COMPAT_KEY"
default_model = "opaque/model@id"
"#
        ),
    )
    .expect("write test config");
    crate::config::invalidate_config_cache();

    let provider = MultiProvider::new_with_auth_status(crate::auth::AuthStatus::default());
    assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
    let openrouter = provider
        .openrouter_provider()
        .expect("openrouter execution slot");
    assert!(
        !openrouter.supports_provider_routing_features(),
        "named openai-compatible defaults must not use OpenRouter provider routing features even with catalog enabled"
    );

    let messages = vec![crate::message::Message {
        role: crate::message::Role::User,
        content: vec![crate::message::ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = openrouter
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = futures::StreamExt::next(&mut stream).await {
            event.expect("stream event should parse");
        }
    });

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.starts_with("POST /v1/chat/completions "),
        "unexpected chat request: {request}"
    );
    assert!(
        !request.contains(r#""provider":"#),
        "direct OpenAI-compatible request must not include OpenRouter provider routing object: {request}"
    );
    assert!(
        !request.contains("HTTP-Referer") && !request.contains("X-Title"),
        "direct OpenAI-compatible request must not include OpenRouter-only headers: {request}"
    );
}

#[test]
fn runtime_display_name_tracks_active_openai_compatible_profile() {
    // Regression for issue #329: switching to a direct OpenAI-compatible
    // profile (NVIDIA NIM) at runtime must surface that profile's display
    // name, not the fixed "OpenRouter" aggregator label. The machine-facing
    // `name()` stays "openrouter" because billing/routing logic keys off it.
    let _lock = crate::storage::lock_test_env();
    // These tests construct MultiProvider directly (not via
    // with_clean_provider_test_env), so make sure the external runtime
    // factories are registered before startup paths need them.
    register_test_external_runtimes();
    let temp = tempfile::TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = OrEnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = OrEnvVarGuard::set("HOME", temp.path());
    let _appdata = OrEnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env_or();

    // Configure both the OpenRouter aggregator and NVIDIA NIM credentials so
    // the slot can host either runtime. Set after the isolate guard, which
    // clears every profile api-key env var.
    let _or_key = OrEnvVarGuard::set("OPENROUTER_API_KEY", "or-test-key");
    let _nim_key = OrEnvVarGuard::set("NVIDIA_API_KEY", "nim-test-key");
    crate::config::invalidate_config_cache();

    let provider = MultiProvider::new_with_auth_status(crate::auth::AuthStatus::default());

    // Switch to a NVIDIA NIM model via the profile-prefixed model request.
    provider
        .set_model("nvidia-nim:nvidia/llama-3.1-nemotron-ultra-253b-v1")
        .expect("switch to nvidia-nim profile");

    assert_eq!(
        Provider::name(&provider),
        "OpenRouter",
        "machine-facing name must stay stable for billing/routing"
    );
    assert_eq!(
        Provider::display_name(&provider),
        "NVIDIA NIM",
        "header/UI display name must reflect the active runtime profile"
    );

    // Switching back to the plain OpenRouter aggregator restores the label.
    provider
        .set_model("anthropic/claude-sonnet-4")
        .expect("switch back to openrouter aggregator");
    assert_eq!(Provider::display_name(&provider), "OpenRouter");
}

/// A bare model id from an OpenAI-compatible catalog must route to the profile
/// that serves it, not to whichever provider happens to be active.
///
/// Regression: `/model celeris-1` while Anthropic was active failed with
/// "Model celeris-1 not supported by Anthropic provider", because bare ids
/// match none of the built-in model-name heuristics and fell through to the
/// active provider.
#[test]
fn bare_openai_compatible_model_ids_route_to_their_profile_not_the_active_provider() {
    with_clean_provider_test_env(|| {
        let rt = enter_test_runtime();
        let _runtime_guard = rt.enter();
        crate::env::set_var("CELERIS_API_KEY", "test-celeris-key");
        crate::env::set_var("META_MUSE_API_KEY", "test-meta-key");
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
            // The failing case: a Claude-family provider is active, so the old
            // fallthrough handed the bare id to Anthropic.
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
            .set_model("muse-spark-1.2")
            .expect("bare Muse model id should resolve to the Meta Model API profile");
        assert_eq!(provider.model(), "muse-spark-1.2");
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "meta-muse:muse-spark-1.2"
        );

        provider.set_active_provider(ActiveProvider::Claude);

        provider
            .set_model("celeris-1")
            .expect("bare Celeris model id should resolve to the Celeris profile");
        assert_eq!(provider.model(), "celeris-1");
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(
            provider.fork_model_switch_request(provider.active_provider(), &provider.model()),
            "celeris:celeris-1"
        );
    });
}

include!("model_resolution/catalog_profiles.rs");
include!("model_resolution/profile_switching.rs");
include!("model_resolution/credential_identity.rs");
