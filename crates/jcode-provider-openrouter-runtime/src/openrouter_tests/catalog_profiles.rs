#[test]
fn openai_compatible_models_endpoint_allows_minimal_model_objects() {
    let parsed = parse_openai_compatible_models_response(
        r#"{
            "object": "list",
            "data": [
                {"id": "glm-51-nvfp4", "object": "model", "created": null, "owned_by": null},
                {"id": "gte-qwen2-7b", "object": "model"}
            ]
        }"#,
    )
    .expect("minimal OpenAI-compatible /models response should parse");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, "glm-51-nvfp4");
    assert_eq!(parsed[0].name, "");
}
#[test]
fn openai_compatible_models_endpoint_allows_chutes_numeric_pricing() {
    let parsed = parse_openai_compatible_models_response(
        r#"{
            "object": "list",
            "data": [{
                "id": "Qwen/Qwen3-32B-TEE",
                "root": "Qwen/Qwen3-32B-FP8",
                "price": {
                    "input": {"tao": 0.0002439746644509701, "usd": 0.08},
                    "output": {"tao": 0.0007319239933529102, "usd": 0.24}
                },
                "object": "model",
                "parent": null,
                "created": 1778439139,
                "pricing": {
                    "prompt": 0.08,
                    "completion": 0.24,
                    "input_cache_read": 0.04
                },
                "owned_by": "sglang",
                "context_length": 40960,
                "supported_features": ["json_mode", "tools"]
            }]
        }"#,
    )
    .expect("Chutes /models response with numeric pricing should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "Qwen/Qwen3-32B-TEE");
    assert_eq!(parsed[0].pricing.prompt.as_deref(), Some("0.08"));
    assert_eq!(parsed[0].pricing.completion.as_deref(), Some("0.24"));
    assert_eq!(parsed[0].pricing.input_cache_read.as_deref(), Some("0.04"));
}
#[test]
fn openai_compatible_models_endpoint_allows_together_top_level_array() {
    let parsed = parse_openai_compatible_models_response(
        r#"[
            {
                "id": "Austism/chronos-hermes-13b",
                "object": "model",
                "created": 1692896905,
                "type": "chat",
                "display_name": "Chronos Hermes (13B)",
                "context_length": 2048,
                "pricing": {
                    "input": 0.3,
                    "output": 0.3,
                    "cached_input": 0.2
                }
            }
        ]"#,
    )
    .expect("Together /models top-level array should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "Austism/chronos-hermes-13b");
    assert_eq!(parsed[0].name, "Chronos Hermes (13B)");
    assert_eq!(parsed[0].context_length, Some(2048));
    assert_eq!(parsed[0].pricing.prompt.as_deref(), Some("0.3"));
    assert_eq!(parsed[0].pricing.completion.as_deref(), Some("0.3"));
    assert_eq!(parsed[0].pricing.input_cache_read.as_deref(), Some("0.2"));
}
#[test]
fn openai_compatible_models_endpoint_allows_models_array_with_name_ids() {
    let parsed = parse_openai_compatible_models_response(
        r#"{
            "models": [{
                "name": "accounts/fireworks/models/example",
                "displayName": "Example Fireworks Model",
                "contextLength": 8192
            }]
        }"#,
    )
    .expect("models array with name-based identifiers should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "accounts/fireworks/models/example");
    assert_eq!(parsed[0].name, "accounts/fireworks/models/example");
    assert_eq!(parsed[0].context_length, Some(8192));
}
#[test]
fn openai_compatible_models_endpoint_reads_llamacpp_meta_n_ctx() {
    // llama.cpp's /v1/models only exposes the context window inside `meta`
    // (issue #447). The `data` entry mirrors llama.cpp's response shape.
    let parsed = parse_openai_compatible_models_response(
        r#"{
            "object": "list",
            "data": [{
                "id": "unsloth/gemma-4-31B-it-UD-Q8_K_XL",
                "object": "model",
                "created": 1783253170,
                "owned_by": "llamacpp",
                "meta": {
                    "vocab_type": 2,
                    "n_vocab": 262144,
                    "n_ctx": 262144,
                    "n_ctx_train": 262144,
                    "n_embd": 5376
                }
            }]
        }"#,
    )
    .expect("llama.cpp /v1/models response should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "unsloth/gemma-4-31B-it-UD-Q8_K_XL");
    assert_eq!(parsed[0].context_length, Some(262144));
}
#[test]
fn named_openai_compatible_provider_sets_catalog_cache_namespace() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let _key = EnvVarGuard::set("TEST_NAMED_COMPAT_KEY", "test-key");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "https://llm.example.com/v1".to_string(),
        api_key_env: Some("TEST_NAMED_COMPAT_KEY".to_string()),
        model_catalog: true,
        default_model: Some("example-model".to_string()),
        ..Default::default()
    };

    let _provider = OpenRouterProvider::new_named_openai_compatible("example-compat", &profile)
        .expect("named profile should initialize");

    assert_eq!(
        std::env::var("JCODE_OPENROUTER_CACHE_NAMESPACE").as_deref(),
        Ok("example-compat")
    );
}
#[test]
fn named_openai_compatible_provider_exposes_static_models_as_routes() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let _key = EnvVarGuard::set("TEST_NAMED_COMPAT_KEY", "test-key");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "https://llm.example.com/v1".to_string(),
        api_key_env: Some("TEST_NAMED_COMPAT_KEY".to_string()),
        model_catalog: true,
        default_model: Some("glm-51-nvfp4".to_string()),
        models: vec![jcode_base::config::NamedProviderModelConfig {
            id: "glm-51-nvfp4".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("comtegra-test", &profile)
        .expect("named profile should initialize");
    let routes = provider.model_routes();

    assert!(routes.iter().any(|route| {
        route.model == "glm-51-nvfp4"
            && route.api_method == "openai-compatible:comtegra-test"
            && route.available
    }));
}
#[test]
fn direct_openai_compatible_provider_advertises_image_input_support() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("local-vision-model".to_string()),
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("local-compat", &profile)
        .expect("local named profile should initialize without auth");

    assert!(provider.supports_image_input());
}
#[test]
fn named_openai_compatible_provider_uses_per_model_image_input_support() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("vision-model".to_string()),
        models: vec![
            jcode_base::config::NamedProviderModelConfig {
                id: "vision-model".to_string(),
                input: vec!["text".to_string(), "image".to_string()],
                ..Default::default()
            },
            jcode_base::config::NamedProviderModelConfig {
                id: "text-model".to_string(),
                input: vec!["text".to_string()],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("local-compat", &profile)
        .expect("local named profile should initialize without auth");

    assert!(provider.supports_image_input());
    provider.set_model("text-model").expect("switch model");
    assert!(!provider.supports_image_input());
    provider
        .set_model("local-compat:vision-model")
        .expect("switch using qualified model");
    assert!(provider.supports_image_input());
}
#[test]
fn named_openai_compatible_model_with_omitted_input_preserves_image_support() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("text-model".to_string()),
        models: vec![jcode_base::config::NamedProviderModelConfig {
            id: "text-model".to_string(),
            context_window: Some(200_000),
            ..Default::default()
        }],
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("local-compat", &profile)
        .expect("local named profile should initialize without auth");

    assert!(provider.supports_image_input());
}
#[test]
fn named_openai_compatible_model_with_empty_input_preserves_image_support() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("text-model".to_string()),
        models: vec![jcode_base::config::NamedProviderModelConfig {
            id: "text-model".to_string(),
            reasoning: None,
            reasoning_effort: None,
            input: Vec::new(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("local-compat", &profile)
        .expect("local named profile should initialize without auth");

    assert!(provider.supports_image_input());
}
#[test]
fn direct_deepseek_profile_does_not_advertise_image_input_support() {
    let provider = OpenRouterProvider {
        profile_id: Some("deepseek".to_string()),
        supports_provider_features: false,
        ..make_custom_compatible_provider()
    };

    assert!(!provider.supports_image_input());
}
#[test]
fn direct_zai_profile_does_not_advertise_image_input_support() {
    let provider = OpenRouterProvider {
        profile_id: Some("zai".to_string()),
        supports_provider_features: false,
        ..make_custom_compatible_provider()
    };

    assert!(!provider.supports_image_input());
}
#[test]
fn direct_deepseek_profile_omits_image_url_parts() {
    let _lock = ENV_LOCK.lock();
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        profile_id: Some("deepseek".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };
    let messages = vec![Message {
        role: Role::User,
        content: vec![
            ContentBlock::Text {
                text: "describe this".to_string(),
                cache_control: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "aW1hZ2U=".to_string(),
            },
        ],
        timestamp: None,
        tool_duration_ms: None,
    }];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        !request.contains(r#""type":"image_url""#),
        "DeepSeek request must not contain unsupported image_url content parts: {request}"
    );
    assert!(
        request.contains("Image omitted"),
        "DeepSeek request should preserve a textual placeholder for omitted images: {request}"
    );
}
#[test]
fn minimax_profile_exposes_static_models_before_catalog_refresh() {
    let models = jcode_base::provider_catalog::openai_compatible_profile_static_models(
        jcode_provider_metadata::MINIMAX_PROFILE,
    );
    assert!(models.iter().any(|model| model == "MiniMax-M2.7"));
    assert!(models.iter().any(|model| model == "MiniMax-M2.7-highspeed"));
    assert!(models.iter().any(|model| model == "MiniMax-M2"));
}
#[test]
fn cerebras_profile_exposes_live_chat_models_before_catalog_refresh() {
    assert_eq!(
        jcode_provider_metadata::CEREBRAS_PROFILE.default_model,
        Some("gpt-oss-120b")
    );

    let models = jcode_base::provider_catalog::openai_compatible_profile_static_models(
        jcode_provider_metadata::CEREBRAS_PROFILE,
    );

    assert!(
        !models.iter().any(|model| model == "qwen-3-coder-480b"),
        "old Cerebras default is no longer returned by the live /models catalog"
    );
    assert!(models.iter().any(|model| model == "gpt-oss-120b"));
    assert!(models.iter().any(|model| model == "zai-glm-4.7"));
    assert!(
        !models
            .iter()
            .any(|model| model == "qwen-3-235b-a22b-instruct-2507")
    );
    assert!(!models.iter().any(|model| model == "llama3.1-8b"));
}
#[test]
fn openai_compatible_profiles_with_unverified_live_catalogs_have_static_fallbacks() {
    let cases = [
        (jcode_provider_metadata::OPENCODE_PROFILE, "minimax-m2.7"),
        (jcode_provider_metadata::OPENCODE_GO_PROFILE, "kimi-k2.5"),
        (jcode_provider_metadata::ZAI_PROFILE, "glm-4.7"),
        (
            jcode_provider_metadata::AI302_PROFILE,
            "qwen3-235b-a22b-instruct-2507",
        ),
        (jcode_provider_metadata::BASETEN_PROFILE, "zai-org/GLM-4.7"),
        (jcode_provider_metadata::CORTECS_PROFILE, "kimi-k2.5"),
        (jcode_provider_metadata::KIMI_PROFILE, "kimi-for-coding"),
        (jcode_provider_metadata::FIRMWARE_PROFILE, "kimi-k2.5"),
        (
            jcode_provider_metadata::HUGGING_FACE_PROFILE,
            "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        ),
        (jcode_provider_metadata::MOONSHOT_PROFILE, "kimi-k2.5"),
        (
            jcode_provider_metadata::NEBIUS_PROFILE,
            "openai/gpt-oss-120b",
        ),
        (
            jcode_provider_metadata::SCALEWAY_PROFILE,
            "qwen3-coder-30b-a3b-instruct",
        ),
        (
            jcode_provider_metadata::STACKIT_PROFILE,
            "openai/gpt-oss-120b",
        ),
        (jcode_provider_metadata::PERPLEXITY_PROFILE, "sonar"),
        (
            jcode_provider_metadata::DEEPINFRA_PROFILE,
            "moonshotai/Kimi-K2-Instruct",
        ),
        (
            jcode_provider_metadata::FIREWORKS_PROFILE,
            "accounts/fireworks/routers/kimi-k2p5-turbo",
        ),
        (jcode_provider_metadata::XIAOMI_MIMO_PROFILE, "mimo-v2.5"),
        (jcode_provider_metadata::META_MUSE_PROFILE, "muse-spark-1.2"),
        (
            jcode_provider_metadata::ALIBABA_CODING_PLAN_PROFILE,
            "qwen3-coder-plus",
        ),
    ];

    for (profile, expected_model) in cases {
        let models = jcode_base::provider_catalog::openai_compatible_profile_static_models(profile);
        assert!(
            models.iter().any(|model| model == expected_model),
            "{} should expose static fallback model {expected_model}; got {models:?}",
            profile.id
        );
    }
}
#[test]
fn profiles_use_endpoint_default_max_tokens() {
    let _lock = ENV_LOCK.lock();
    let _override = EnvVarGuard::remove("JCODE_OPENROUTER_MAX_TOKENS");

    assert_eq!(
        OpenRouterProvider::configured_max_tokens(Some("comtegra")),
        None
    );
    assert_eq!(
        OpenRouterProvider::configured_max_tokens(Some("deepseek")),
        None
    );
    assert_eq!(
        OpenRouterProvider::configured_max_tokens(Some("celeris")),
        None
    );
}
#[test]
fn max_tokens_env_overrides_profile_default() {
    let _lock = ENV_LOCK.lock();
    let _override = EnvVarGuard::set("JCODE_OPENROUTER_MAX_TOKENS", "4096");

    assert_eq!(
        OpenRouterProvider::configured_max_tokens(Some("comtegra")),
        Some(4096)
    );
}
#[test]
fn openai_compatible_model_catalog_refresh_calls_models_endpoint_and_updates_display() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _namespace = EnvVarGuard::set(
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "test-openai-compatible-flow",
    );
    let (api_base, request_rx) = spawn_single_response_models_server(
        r#"{
            "object": "list",
            "data": [
                {"id": "live-login-flow-model", "object": "model", "context_length": 131072}
            ]
        }"#,
    );
    let provider = OpenRouterProvider {
        api_base,
        model: Arc::new(RwLock::new("live-login-flow-model".to_string())),
        auth: ProviderAuth::AuthorizationBearer {
            token: "sk-live-catalog".to_string(),
            label: "OPENAI_COMPAT_API_KEY".to_string(),
        },
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: None,
        reasoning_effort_support: None,
        static_models: vec!["static-login-flow-fallback".to_string()],
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let fetched = rt
        .block_on(provider.refresh_models())
        .expect("refresh fake model catalog");
    assert_eq!(fetched[0].id, "live-login-flow-model");
    assert_eq!(provider.context_window(), 131_072);

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.starts_with("GET /v1/models "),
        "unexpected catalog request: {request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-live-catalog"),
        "catalog request should include saved API key auth header: {request}"
    );
    assert!(
        request.to_ascii_lowercase().contains("user-agent: jcode/"),
        "catalog requests must include a User-Agent because providers like Cerebras reject bare HTTP clients: {request}"
    );

    let display = provider.available_models_display();
    assert!(display.iter().any(|model| model == "live-login-flow-model"));
    assert!(
        display
            .iter()
            .any(|model| model == "static-login-flow-fallback"),
        "static fallback/default models should remain visible alongside live catalog models: {display:?}"
    );

    let fresh_provider = OpenRouterProvider {
        api_base: provider.api_base.clone(),
        model: Arc::new(RwLock::new("live-login-flow-model".to_string())),
        auth: provider.auth.clone(),
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: None,
        reasoning_effort_support: None,
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };
    assert_eq!(fresh_provider.context_window(), 131_072);
}
#[test]
fn built_in_openai_compatible_static_models_drop_out_after_live_catalog() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _namespace = EnvVarGuard::set(
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "test-cerebras-live-catalog-filters-static-fallback",
    );
    let (api_base, _request_rx) = spawn_single_response_models_server(
        r#"{
            "object": "list",
            "data": [
                {"id": "qwen-3-235b-a22b-instruct-2507", "object": "model"},
                {"id": "zai-glm-4.7", "object": "model"},
                {"id": "gpt-oss-120b", "object": "model"}
            ]
        }"#,
    );
    let provider = OpenRouterProvider {
        api_base,
        auth: ProviderAuth::AuthorizationBearer {
            token: "sk-live-catalog".to_string(),
            label: "CEREBRAS_API_KEY".to_string(),
        },
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: Some("cerebras".to_string()),
        static_models: vec!["gpt-oss-120b".to_string(), "zai-glm-4.7".to_string()],
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(provider.refresh_models())
        .expect("refresh fake model catalog");

    let display = provider.available_models_display();
    assert!(display.iter().any(|model| model == "gpt-oss-120b"));
    assert!(display.iter().any(|model| model == "zai-glm-4.7"));
    assert!(
        display
            .iter()
            .any(|model| model == "qwen-3-235b-a22b-instruct-2507"),
        "live catalog chat-capable models should remain visible: {display:?}"
    );
}
#[test]
fn direct_openai_compatible_static_models_are_marked_as_fallback_before_live_catalog() {
    let provider = OpenRouterProvider {
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: Some("opencode".to_string()),
        static_models: vec!["minimax-m2.7".to_string()],
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };

    let routes = provider.model_routes();
    let route = routes
        .iter()
        .find(|route| route.model == "minimax-m2.7")
        .expect("static fallback route should be present before live catalog fetch");

    assert!(
        route
            .detail
            .contains("fallback: static provider model list"),
        "fallback routes should be clearly labeled in the model picker: {route:?}"
    );
}
#[test]
fn cerebras_live_catalog_models_are_selectable_on_explicit_switch() {
    let provider = OpenRouterProvider {
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: Some("cerebras".to_string()),
        static_models: vec!["gpt-oss-120b".to_string()],
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };

    provider
        .set_model("zai-glm-4.7")
        .expect("live Cerebras model should be selectable");
    assert_eq!(provider.model(), "zai-glm-4.7");
    provider
        .set_model("gpt-oss-120b")
        .expect("default Cerebras model should remain selectable");
    assert_eq!(provider.model(), "gpt-oss-120b");
}
#[test]
fn direct_deepseek_profile_uses_static_1m_context_when_catalog_is_absent() {
    let _lock = ENV_LOCK.lock();
    let _base = EnvVarGuard::set("JCODE_OPENROUTER_API_BASE", "https://api.deepseek.com");
    let _key_name = EnvVarGuard::set("JCODE_OPENROUTER_API_KEY_NAME", "DEEPSEEK_API_KEY");
    let _api_key = EnvVarGuard::set("DEEPSEEK_API_KEY", "test");
    let _namespace = EnvVarGuard::set("JCODE_OPENROUTER_CACHE_NAMESPACE", "deepseek");
    let _model = EnvVarGuard::set("JCODE_OPENROUTER_MODEL", "deepseek-v4-flash");
    let _catalog = EnvVarGuard::set("JCODE_OPENROUTER_MODEL_CATALOG", "0");

    let provider = OpenRouterProvider::new().expect("provider");

    assert_eq!(provider.context_window(), 1_000_000);
}
#[test]
fn explicit_cached_context_window_precedes_zai_family_fallback() {
    let model = "glm-5.3-issue-1087";
    jcode_base::provider::populate_context_limits(HashMap::from([(model.to_string(), 1_000_000)]));
    let provider = OpenRouterProvider {
        model: Arc::new(RwLock::new(model.to_string())),
        profile_id: Some("zai".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    assert_eq!(
        jcode_base::provider_catalog::openai_compatible_profile_context_limit("zai", model),
        Some(200_000),
        "the regression requires a conflicting static family guess"
    );
    assert_eq!(provider.context_window(), 1_000_000);
}
#[test]
fn named_openai_compatible_model_context_window_overrides_default() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let mut config = jcode_base::config::NamedProviderConfig {
        base_url: "https://compat.example.test/v1".to_string(),
        api_key: Some("test".to_string()),
        default_model: Some("custom-long-context".to_string()),
        models: vec![jcode_base::config::NamedProviderModelConfig {
            id: "custom-long-context".to_string(),
            context_window: Some(512_000),
            reasoning: None,
            reasoning_effort: None,
            input: Vec::new(),
        }],
        ..Default::default()
    };
    config.model_catalog = false;

    let provider =
        OpenRouterProvider::new_named_openai_compatible("custom", &config).expect("provider");

    assert_eq!(provider.context_window(), 512_000);
}
#[test]
fn named_profile_context_window_survives_provider_qualified_model() {
    // Regression for #403: if the runtime model transiently carries the
    // session-routing `<profile>:<model>` prefix, context_window() must still
    // resolve the configured per-model context_window rather than falling
    // through to the (large) provider default and over-budgeting the request.
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let mut config = jcode_base::config::NamedProviderConfig {
        base_url: "http://10.15.15.53:8080/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("qwen3.6-35b-a2000-128k".to_string()),
        models: vec![jcode_base::config::NamedProviderModelConfig {
            id: "qwen3.6-35b-a2000-128k".to_string(),
            context_window: Some(131_072),
            reasoning: None,
            reasoning_effort: None,
            input: Vec::new(),
        }],
        ..Default::default()
    };
    config.model_catalog = false;
    config.requires_api_key = Some(false);

    let provider = OpenRouterProvider::new_named_openai_compatible("cachyai-a2000", &config)
        .expect("provider");

    // Simulate the poisoned/qualified runtime model that #403 reported.
    {
        let mut model = provider.model.try_write().expect("model lock");
        *model = "cachyai-a2000:qwen3.6-35b-a2000-128k".to_string();
    }

    assert_eq!(provider.context_window(), 131_072);
}
#[test]
fn named_openai_compatible_loads_api_key_from_env_file() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp dir");
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let _api_key = EnvVarGuard::remove("CUSTOM_API_KEY");
    write_test_api_key(&temp, "custom.env", "CUSTOM_API_KEY", "from-env-file");

    let config = jcode_base::config::NamedProviderConfig {
        base_url: "https://compat.example.test/v1".to_string(),
        api_key_env: Some("CUSTOM_API_KEY".to_string()),
        env_file: Some("custom.env".to_string()),
        default_model: Some("custom-model".to_string()),
        ..Default::default()
    };

    OpenRouterProvider::new_named_openai_compatible("custom", &config)
        .expect("provider should load key from env file");
}
#[test]
fn custom_compatible_provider_preserves_claude_like_model_ids() {
    let provider = make_custom_compatible_provider();

    provider.set_model("claude-opus4.6-thinking").unwrap();

    assert_eq!(provider.model(), "claude-opus4.6-thinking");
}
#[test]
fn custom_compatible_provider_preserves_at_sign_model_ids() {
    let provider = make_custom_compatible_provider();

    provider.set_model("gpt-5.4@OpenAI").unwrap();

    assert_eq!(provider.model(), "gpt-5.4@OpenAI");
}
#[test]
fn named_profile_set_model_strips_own_session_routing_prefix() {
    // Session restore persists `<profile>:<model>`; the standalone provider
    // must normalize its own profile prefix back to the bare model id so the
    // upstream API never sees `tokenrouter:MiniMax-M3` (issues #382/#383/#363).
    let provider = OpenRouterProvider {
        profile_id: Some("tokenrouter".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    provider.set_model("tokenrouter:MiniMax-M3").unwrap();
    assert_eq!(provider.model(), "MiniMax-M3");

    // Bare ids still work unchanged.
    provider.set_model("MiniMax-M3").unwrap();
    assert_eq!(provider.model(), "MiniMax-M3");
}
#[test]
fn named_profile_set_model_strips_other_known_profile_prefix() {
    // A session saved under one built-in OpenAI-compatible profile and
    // reattached under another must still normalize to the bare model id.
    let provider = OpenRouterProvider {
        profile_id: Some("tokenrouter".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    provider.set_model("kimi:kimi-for-coding").unwrap();
    assert_eq!(provider.model(), "kimi-for-coding");
}
#[test]
fn named_profile_set_model_keeps_builtin_routing_prefixes() {
    // Built-in provider routing prefixes must round-trip verbatim so a user can
    // switch the active provider from a saved session.
    let provider = OpenRouterProvider {
        profile_id: Some("tokenrouter".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    for spec in [
        "claude-oauth:claude-opus-4-8",
        "openai-api:gpt-5.4",
        "copilot:gpt-5.4",
    ] {
        provider.set_model(spec).unwrap();
        assert_eq!(provider.model(), spec, "spec {spec} must be preserved");
    }
}
#[test]
fn named_profile_set_model_keeps_unknown_prefix_with_colon() {
    // A `:`-bearing id whose prefix is neither this profile nor a known
    // built-in profile must be preserved verbatim (it may be a real model id).
    let provider = OpenRouterProvider {
        profile_id: Some("tokenrouter".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    provider.set_model("some-vendor:weird-model").unwrap();
    assert_eq!(provider.model(), "some-vendor:weird-model");
}
#[test]
fn openrouter_provider_normalizes_bare_pinned_model_ids() {
    let provider = make_provider();

    provider.set_model("gpt-5.4@OpenAI").unwrap();

    assert_eq!(provider.model(), "openai/gpt-5.4");
}
