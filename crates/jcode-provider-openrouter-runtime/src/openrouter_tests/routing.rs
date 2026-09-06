#[test]
fn test_configured_api_base_accepts_https() {
    let _lock = ENV_LOCK.lock();
    let prev = std::env::var("JCODE_OPENROUTER_API_BASE").ok();
    jcode_base::env::set_var(
        "JCODE_OPENROUTER_API_BASE",
        "https://api.groq.com/openai/v1/",
    );
    assert_eq!(configured_api_base(), "https://api.groq.com/openai/v1");
    if let Some(value) = prev {
        jcode_base::env::set_var("JCODE_OPENROUTER_API_BASE", value);
    } else {
        jcode_base::env::remove_var("JCODE_OPENROUTER_API_BASE");
    }
}
#[test]
fn test_configured_api_base_rejects_insecure_http_remote() {
    let _lock = ENV_LOCK.lock();
    let prev = std::env::var("JCODE_OPENROUTER_API_BASE").ok();
    jcode_base::env::set_var("JCODE_OPENROUTER_API_BASE", "http://example.com/v1");
    assert_eq!(configured_api_base(), DEFAULT_API_BASE);
    if let Some(value) = prev {
        jcode_base::env::set_var("JCODE_OPENROUTER_API_BASE", value);
    } else {
        jcode_base::env::remove_var("JCODE_OPENROUTER_API_BASE");
    }
}
#[test]
fn autodetects_single_saved_openai_compatible_profile() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();

    let opencode = jcode_base::provider_catalog::resolve_openai_compatible_profile(
        jcode_base::provider_catalog::OPENCODE_PROFILE,
    );
    write_test_api_key(
        &temp,
        &opencode.env_file,
        &opencode.api_key_env,
        "test-opencode-key",
    );

    assert_eq!(configured_api_base(), opencode.api_base);
    assert_eq!(configured_api_key_name(), opencode.api_key_env);
    assert_eq!(configured_env_file_name(), opencode.env_file);
    assert!(OpenRouterProvider::has_credentials());
}
#[test]
fn autodetects_single_saved_local_openai_compatible_profile() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();

    let lmstudio = jcode_base::provider_catalog::resolve_openai_compatible_profile(
        jcode_base::provider_catalog::LMSTUDIO_PROFILE,
    );
    let config_dir = test_config_dir(&temp).join("jcode");
    std::fs::create_dir_all(&config_dir).expect("create test config dir");
    std::fs::write(
        config_dir.join(&lmstudio.env_file),
        format!(
            "{}=1\n",
            jcode_base::provider_catalog::OPENAI_COMPAT_LOCAL_ENABLED_ENV
        ),
    )
    .expect("write local config");

    assert_eq!(configured_api_base(), lmstudio.api_base);
    assert_eq!(configured_api_key_name(), lmstudio.api_key_env);
    assert_eq!(configured_env_file_name(), lmstudio.env_file);
    assert!(configured_allow_no_auth());
    assert!(OpenRouterProvider::has_credentials());
}
#[test]
fn openrouter_transport_state_distinguishes_runtime_identities() {
    let _lock = ENV_LOCK.lock();
    // Isolate the on-disk config/credential lookup the same way the sibling
    // autodetect tests do, so this test does not read whatever provider
    // profile happens to be configured on the host machine.
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();

    assert_eq!(
        OpenRouterTransportState::from_current_env(None),
        OpenRouterTransportState::OpenRouterApiKey
    );
    assert!(OpenRouterTransportState::from_current_env(None).accrues_user_api_key_cost());
    assert!(OpenRouterTransportState::from_current_env(None).is_real_openrouter());

    jcode_base::env::set_var("JCODE_OPENROUTER_TRANSPORT_STATE", "direct-api-key");
    assert_eq!(
        OpenRouterTransportState::from_current_env(None),
        OpenRouterTransportState::DirectApiKey
    );
    jcode_base::env::remove_var("JCODE_OPENROUTER_TRANSPORT_STATE");

    jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", "openrouter");
    assert_eq!(
        OpenRouterTransportState::from_current_env(Some("openrouter")),
        OpenRouterTransportState::OpenRouterApiKey
    );
    assert!(OpenRouterTransportState::from_current_env(Some("openrouter")).is_real_openrouter());
    jcode_base::env::remove_var("JCODE_RUNTIME_PROVIDER");

    jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", "jcode");
    assert_eq!(
        OpenRouterTransportState::from_current_env(Some("jcode")),
        OpenRouterTransportState::JcodeSubscription
    );
    assert!(!OpenRouterTransportState::from_current_env(Some("jcode")).accrues_user_api_key_cost());

    jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", "openai-compatible");
    assert_eq!(
        OpenRouterTransportState::from_current_env(Some("openai-compatible")),
        OpenRouterTransportState::DirectApiKey
    );

    jcode_base::env::set_var("JCODE_OPENROUTER_ALLOW_NO_AUTH", "1");
    assert_eq!(
        OpenRouterTransportState::from_current_env(Some("openai-compatible")),
        OpenRouterTransportState::DirectNoAuth
    );
    assert!(
        !OpenRouterTransportState::from_current_env(Some("openai-compatible"))
            .accrues_user_api_key_cost()
    );

    jcode_base::env::remove_var("JCODE_OPENROUTER_ALLOW_NO_AUTH");
    jcode_base::env::remove_var("JCODE_RUNTIME_PROVIDER");
    jcode_base::env::set_var("JCODE_NAMED_PROVIDER_PROFILE", "my-gateway");
    assert_eq!(
        OpenRouterTransportState::from_current_env(None),
        OpenRouterTransportState::DirectApiKey
    );
}
#[test]
fn does_not_guess_when_multiple_saved_openai_compatible_profiles_exist() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();

    let opencode = jcode_base::provider_catalog::resolve_openai_compatible_profile(
        jcode_base::provider_catalog::OPENCODE_PROFILE,
    );
    let chutes = jcode_base::provider_catalog::resolve_openai_compatible_profile(
        jcode_base::provider_catalog::CHUTES_PROFILE,
    );
    write_test_api_key(
        &temp,
        &opencode.env_file,
        &opencode.api_key_env,
        "test-opencode-key",
    );
    write_test_api_key(
        &temp,
        &chutes.env_file,
        &chutes.api_key_env,
        "test-chutes-key",
    );

    assert_eq!(configured_api_base(), DEFAULT_API_BASE);
    assert_eq!(configured_api_key_name(), DEFAULT_API_KEY_NAME);
    assert_eq!(configured_env_file_name(), DEFAULT_ENV_FILE);
    assert!(!OpenRouterProvider::has_credentials());
}
#[test]
fn autodetected_profile_seeds_default_model_and_cache_namespace() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();

    let zai = jcode_base::provider_catalog::resolve_openai_compatible_profile(
        jcode_base::provider_catalog::ZAI_PROFILE,
    );
    write_test_api_key(&temp, &zai.env_file, &zai.api_key_env, "test-zai-key");

    let provider = OpenRouterProvider::new().expect("provider");
    assert_eq!(provider.model.blocking_read().clone(), "glm-4.5");
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_CACHE_NAMESPACE")
            .ok()
            .as_deref(),
        Some("zai")
    );
}
#[test]
fn test_parse_model_spec() {
    let (model, provider) = parse_model_spec("anthropic/claude-sonnet-4@Fireworks");
    assert_eq!(model, "anthropic/claude-sonnet-4");
    let provider = provider.expect("provider");
    assert_eq!(provider.name, "Fireworks");
    assert!(!provider.allow_fallbacks);

    let (model, provider) = parse_model_spec("anthropic/claude-sonnet-4@Fireworks!");
    assert_eq!(model, "anthropic/claude-sonnet-4");
    let provider = provider.expect("provider");
    assert_eq!(provider.name, "Fireworks");
    assert!(!provider.allow_fallbacks);

    let (model, provider) = parse_model_spec("moonshotai/kimi-k2.5@moonshot");
    assert_eq!(model, "moonshotai/kimi-k2.5");
    let provider = provider.expect("provider");
    assert_eq!(provider.name, "Moonshot AI");

    let (model, provider) = parse_model_spec("anthropic/claude-sonnet-4@auto");
    assert_eq!(model, "anthropic/claude-sonnet-4");
    assert!(provider.is_none());
}
#[test]
fn fork_preserves_explicit_provider_pin() {
    let provider = make_provider();
    provider
        .set_model("z-ai/glm-5.2@Novita")
        .expect("set explicitly pinned model");

    let fork = provider.fork();

    assert_eq!(fork.model(), "z-ai/glm-5.2");
    assert_eq!(
        fork.explicit_provider_pin_for_current_model().as_deref(),
        Some("Novita")
    );
}
#[test]
fn test_rank_providers_cache_priority() {
    let endpoints = vec![
        make_endpoint("FastCache", 50.0, 99.0, true, 0.0000002),
        make_endpoint("FasterNoCache", 60.0, 99.0, false, 0.0000001),
    ];

    let ranked = OpenRouterProvider::rank_providers_from_endpoints(&endpoints);
    assert_eq!(ranked.first().map(|s| s.as_str()), Some("FastCache"));
}
#[test]
fn test_rank_providers_speed_priority_among_cache_capable() {
    let endpoints = vec![
        make_endpoint("Fireworks", 120.0, 99.0, true, 0.0000013),
        make_endpoint("Moonshot AI", 80.0, 99.0, true, 0.0000010),
    ];

    let ranked = OpenRouterProvider::rank_providers_from_endpoints(&endpoints);
    assert_eq!(ranked.first().map(|s| s.as_str()), Some("Fireworks"));
}
#[test]
fn test_rank_providers_filters_down_providers() {
    let mut down_ep = make_endpoint("DownProvider", 200.0, 100.0, true, 0.0000001);
    down_ep.status = Some(1); // down
    let endpoints = vec![
        down_ep,
        make_endpoint("UpProvider", 50.0, 99.0, true, 0.0000002),
    ];

    let ranked = OpenRouterProvider::rank_providers_from_endpoints(&endpoints);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0], "UpProvider");
}
#[test]
fn test_background_refresh_waits_for_soft_ttl() {
    let provider = make_provider();

    assert!(!provider.should_background_refresh_model_catalog(
        MODEL_CATALOG_SOFT_REFRESH_SECS.saturating_sub(1)
    ));
    assert!(provider.should_background_refresh_model_catalog(MODEL_CATALOG_SOFT_REFRESH_SECS));
}
#[test]
fn test_background_refresh_is_throttled_between_attempts() {
    let provider = make_provider();
    assert!(provider.begin_background_model_catalog_refresh());
    assert!(!provider.should_background_refresh_model_catalog(MODEL_CATALOG_SOFT_REFRESH_SECS));

    OpenRouterProvider::finish_background_model_catalog_refresh(&provider.model_catalog_refresh);

    assert!(!provider.should_background_refresh_model_catalog(MODEL_CATALOG_SOFT_REFRESH_SECS));
}
#[test]
fn test_kimi_routing_uses_endpoints_or_fallback() {
    let provider = OpenRouterProvider {
        model: Arc::new(RwLock::new("moonshotai/kimi-k2.5".to_string())),
        ..make_provider()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let routing = rt.block_on(provider.effective_routing("moonshotai/kimi-k2.5"));
    let order = routing.order.expect("provider order should be set");
    // Should have providers - either from endpoint API or Kimi fallback
    assert!(
        !order.is_empty(),
        "Kimi routing should always produce a provider order"
    );
}
#[test]
fn observed_session_provider_pin_sticks_without_fallbacks() {
    // Simulates the KV-cache stickiness contract: after OpenRouter serves a
    // request for this model from a concrete provider (recorded as an
    // observed pin), every subsequent request must route to that exact same
    // provider with fallbacks disabled so the upstream prompt cache stays warm.
    let model = "anthropic/claude-sonnet-4.6";
    let provider = OpenRouterProvider {
        model: Arc::new(RwLock::new(model.to_string())),
        provider_pin: Arc::new(Mutex::new(Some(ProviderPin {
            model: model.to_string(),
            provider: "anthropic".to_string(),
            source: PinSource::Observed,
            allow_fallbacks: true,
            last_cache_read: None,
        }))),
        ..make_provider()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let routing = rt.block_on(provider.effective_routing(model));

    assert_eq!(
        routing.order.as_deref(),
        Some(["anthropic".to_string()].as_slice()),
        "observed session provider should be pinned exactly"
    );
    assert!(
        !routing.allow_fallbacks,
        "observed session pin must disable fallbacks to preserve the KV cache"
    );
}
#[test]
fn observed_pin_yields_to_explicit_user_routing_order() {
    // If the user explicitly narrowed routing themselves (base order set),
    // their configured order wins over the auto-observed session pin.
    let model = "anthropic/claude-sonnet-4.6";
    let base = ProviderRouting {
        order: Some(vec!["fireworks".to_string()]),
        ..Default::default()
    };
    let provider = OpenRouterProvider {
        model: Arc::new(RwLock::new(model.to_string())),
        provider_routing: Arc::new(RwLock::new(base)),
        provider_pin: Arc::new(Mutex::new(Some(ProviderPin {
            model: model.to_string(),
            provider: "anthropic".to_string(),
            source: PinSource::Observed,
            allow_fallbacks: true,
            last_cache_read: None,
        }))),
        ..make_provider()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let routing = rt.block_on(provider.effective_routing(model));

    assert_eq!(
        routing.order.as_deref(),
        Some(["fireworks".to_string()].as_slice()),
        "explicit user routing order should win over an observed session pin"
    );
}
#[test]
fn test_kimi_coding_header_detection_matches_endpoint_and_model() {
    assert!(should_send_kimi_coding_agent_headers(
        "https://api.kimi.com/coding/v1",
        None,
    ));
    assert!(should_send_kimi_coding_agent_headers(
        "https://coding.dashscope.aliyuncs.com/v1",
        None,
    ));
    assert!(should_send_kimi_coding_agent_headers(
        "https://coding-intl.dashscope.aliyuncs.com/v1",
        None,
    ));
    assert!(should_send_kimi_coding_agent_headers(
        "https://api.z.ai/api/coding/paas/v4",
        None,
    ));
    assert!(should_send_kimi_coding_agent_headers(
        "https://example.com/v1",
        Some("kimi-for-coding"),
    ));
    assert!(should_send_kimi_coding_agent_headers(
        "https://openrouter.ai/api/v1",
        Some("moonshotai/kimi-k2.5"),
    ));
    assert!(!should_send_kimi_coding_agent_headers(
        "https://api.openrouter.ai/api/v1",
        Some("anthropic/claude-sonnet-4"),
    ));
}
#[test]
fn test_openrouter_kimi_chat_request_includes_compat_user_agent() {
    let request = apply_kimi_coding_agent_headers(
        Client::new().post("https://openrouter.ai/api/v1/chat/completions"),
        "https://openrouter.ai/api/v1",
        Some("moonshotai/kimi-k2.5"),
    )
    .build()
    .expect("build request");
    assert!(
        request
            .headers()
            .get("User-Agent")
            .and_then(|value| value.to_str().ok())
            == Some(KIMI_CODING_USER_AGENT),
        "Kimi OpenRouter chat request should include compatibility User-Agent"
    );
}
#[test]
fn test_parse_next_event_accepts_compact_sse_data_and_reasoning_content() {
    let bytes = Bytes::from_static(
        b"data:{\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
    );
    let mut stream = OpenRouterStream::new(
        futures::stream::once(async move { Ok::<Bytes, reqwest::Error>(bytes) }),
        "kimi-for-coding".to_string(),
        Arc::new(Mutex::new(None)),
    );

    match futures::executor::block_on(stream.next()) {
        Some(Ok(StreamEvent::ThinkingDelta(text))) => assert_eq!(text, "thinking"),
        other => panic!("expected ThinkingDelta, got {:?}", other),
    }
}
#[test]
fn test_parse_next_event_emits_only_incremental_reasoning_content() {
    let chunks = vec![
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(
            b"data:{\"choices\":[{\"delta\":{\"reasoning_content\":\"Thinking\"}}]}\n\n",
        )),
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(
            b"data:{\"choices\":[{\"delta\":{\"reasoning_content\":\"Thinking more\"}}]}\n\n",
        )),
    ];
    let mut stream = OpenRouterStream::new(
        futures::stream::iter(chunks),
        "moonshotai/kimi-k2.5".to_string(),
        Arc::new(Mutex::new(None)),
    );

    match futures::executor::block_on(stream.next()) {
        Some(Ok(StreamEvent::ThinkingDelta(text))) => assert_eq!(text, "Thinking"),
        other => panic!("expected first ThinkingDelta, got {:?}", other),
    }
    match futures::executor::block_on(stream.next()) {
        Some(Ok(StreamEvent::ThinkingDelta(text))) => assert_eq!(text, " more"),
        other => panic!("expected incremental ThinkingDelta, got {:?}", other),
    }
}
#[test]
fn test_endpoint_detail_string() {
    let ep = EndpointInfo {
        provider_name: "TestProvider".to_string(),
        tag: None,
        pricing: ModelPricing {
            prompt: Some("0.00000045".to_string()),
            completion: Some("0.00000225".to_string()),
            input_cache_read: Some("0.00000007".to_string()),
            input_cache_write: Some("0.00000012".to_string()),
        },
        context_length: Some(131072),
        max_completion_tokens: Some(8192),
        quantization: Some("fp8".to_string()),
        uptime_last_30m: Some(99.5),
        latency_last_30m: Some(serde_json::json!({"p50": 500, "p75": 800})),
        throughput_last_30m: Some(serde_json::json!({"p50": 42, "p75": 55})),
        supports_implicit_caching: Some(true),
        status: Some(0),
    };
    let detail = ep.detail_string();
    assert!(
        detail.contains("$0.45/M"),
        "should contain price: {}",
        detail
    );
    assert!(detail.contains("100%"), "should contain uptime: {}", detail);
    assert!(
        detail.contains("out $2.25/M"),
        "should contain output price: {}",
        detail
    );
    assert!(
        detail.contains("cache write $0.12/M"),
        "should contain cache write price: {}",
        detail
    );
    assert!(
        detail.contains("cache read $0.07/M"),
        "should contain cache read price: {}",
        detail
    );
    assert!(
        detail.contains("500ms p50"),
        "should contain latency: {}",
        detail
    );
    assert!(
        detail.contains("42tps"),
        "should contain throughput: {}",
        detail
    );
    assert!(
        detail.contains("cache on"),
        "should contain cache: {}",
        detail
    );
    assert!(
        detail.contains("fp8"),
        "should contain quantization: {}",
        detail
    );
}
#[test]
fn runtime_display_name_for_profile_runtime_instance() {
    // Direct unit coverage of the per-instance resolver used by
    // `Provider::display_name`.
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let _key = EnvVarGuard::set("NVIDIA_API_KEY", "nim-test-key");

    let nim = OpenRouterProvider::new_openai_compatible_profile_runtime(
        jcode_base::provider_catalog::NVIDIA_NIM_PROFILE,
    )
    .expect("build nvidia-nim runtime");
    assert_eq!(nim.runtime_display_name(), "NVIDIA NIM");
    assert_eq!(Provider::name(&nim), "openrouter");
}
#[test]
fn jcode_subscription_runtime_has_explicit_display_and_route_identity() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let _base = EnvVarGuard::set(
        "JCODE_OPENROUTER_API_BASE",
        jcode_base::subscription_catalog::DEFAULT_JCODE_API_BASE,
    );
    let _key_name = EnvVarGuard::set(
        "JCODE_OPENROUTER_API_KEY_NAME",
        jcode_base::subscription_catalog::JCODE_API_KEY_ENV,
    );
    let _env_file = EnvVarGuard::set(
        "JCODE_OPENROUTER_ENV_FILE",
        jcode_base::subscription_catalog::JCODE_ENV_FILE,
    );
    let _provider_features = EnvVarGuard::set("JCODE_OPENROUTER_PROVIDER_FEATURES", "0");
    let _transport = EnvVarGuard::set("JCODE_OPENROUTER_TRANSPORT_STATE", "jcode-subscription");
    let _key = EnvVarGuard::set(
        jcode_base::subscription_catalog::JCODE_API_KEY_ENV,
        "jcode_test_subscription_key",
    );

    let provider = OpenRouterProvider::new().expect("build jcode subscription runtime");
    assert_eq!(provider.runtime_display_name(), "Jcode Subscription");
    assert_eq!(Provider::display_name(&provider), "Jcode Subscription");
    assert_eq!(Provider::name(&provider), "openrouter");
    assert_eq!(
        provider.direct_openai_compatible_route_parts(),
        Some((
            "Jcode Subscription".to_string(),
            "jcode-subscription".to_string(),
            jcode_base::subscription_catalog::DEFAULT_JCODE_API_BASE.to_string(),
        ))
    );
}
#[test]
fn non_subscription_runtimes_keep_existing_display_and_route_identity() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let _openrouter_key = EnvVarGuard::set("OPENROUTER_API_KEY", "openrouter-test-key");

    let openrouter =
        OpenRouterProvider::new_openrouter_api_key_runtime().expect("build OpenRouter runtime");
    assert_eq!(openrouter.runtime_display_name(), "OpenRouter");
    assert_eq!(Provider::display_name(&openrouter), "OpenRouter");
    assert_eq!(openrouter.direct_openai_compatible_route_parts(), None);

    let _base = EnvVarGuard::set("JCODE_OPENROUTER_API_BASE", "https://example.com/v1");
    let _key_name = EnvVarGuard::set("JCODE_OPENROUTER_API_KEY_NAME", "GENERIC_API_KEY");
    let _provider_features = EnvVarGuard::set("JCODE_OPENROUTER_PROVIDER_FEATURES", "0");
    let _transport = EnvVarGuard::set("JCODE_OPENROUTER_TRANSPORT_STATE", "direct-compatible");
    let _generic_key = EnvVarGuard::set("GENERIC_API_KEY", "generic-test-key");

    let compatible = OpenRouterProvider::new().expect("build generic compatible runtime");
    assert_eq!(compatible.runtime_display_name(), "OpenAI-compatible");
    assert_eq!(Provider::display_name(&compatible), "OpenAI-compatible");
    assert_eq!(
        compatible.direct_openai_compatible_route_parts(),
        Some((
            "OpenAI-compatible".to_string(),
            "openai-compatible".to_string(),
            "https://example.com/v1".to_string(),
        ))
    );
}
#[test]
fn custom_endpoint_using_jcode_key_name_is_not_a_subscription_runtime() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let _base = EnvVarGuard::set("JCODE_OPENROUTER_API_BASE", "https://example.com/v1");
    let _key_name = EnvVarGuard::set(
        "JCODE_OPENROUTER_API_KEY_NAME",
        jcode_base::subscription_catalog::JCODE_API_KEY_ENV,
    );
    let _provider_features = EnvVarGuard::set("JCODE_OPENROUTER_PROVIDER_FEATURES", "0");
    let _key = EnvVarGuard::set(
        jcode_base::subscription_catalog::JCODE_API_KEY_ENV,
        "custom-endpoint-test-key",
    );

    let provider = OpenRouterProvider::new().expect("build custom endpoint runtime");
    assert_eq!(provider.runtime_display_name(), "OpenAI-compatible");
    assert_eq!(
        provider.direct_openai_compatible_route_parts(),
        Some((
            "OpenAI-compatible".to_string(),
            "openai-compatible".to_string(),
            "https://example.com/v1".to_string(),
        ))
    );
}
