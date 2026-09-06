#[test]
fn named_provider_config_accepts_openai_compatible_spelling() {
    let cfg: crate::config::Config = toml::from_str(
        r#"
        [providers.my-gateway]
        type = "openai-compatible"
        base_url = "https://llm.example.com/v1"
        auth = "bearer"
        api_key_env = "MY_GATEWAY_API_KEY"
        default_model = "opaque/model@id"

        [[providers.my-gateway.models]]
        id = "opaque/model@id"
        input = ["text"]
        "#,
    )
    .expect("config should parse");

    let profile = cfg.providers.get("my-gateway").expect("profile");
    assert_eq!(
        profile.provider_type,
        crate::config::NamedProviderType::OpenAiCompatible
    );
    assert_eq!(profile.base_url, "https://llm.example.com/v1");
    assert_eq!(profile.default_model.as_deref(), Some("opaque/model@id"));
    assert_eq!(profile.models[0].id, "opaque/model@id");
}
#[test]
fn named_anthropic_compatible_profile_maps_endpoint_auth_headers_and_model() {
    let _lock = crate::storage::lock_test_env();
    let _guard = EnvGuard::save(&[
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_API_KEY_NAME",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_MODEL",
        "JCODE_RUNTIME_PROVIDER",
    ]);
    let previous_home = std::env::var_os("JCODE_HOME");
    let temp = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::config::Config::invalidate_cache();

    let config_path = crate::config::Config::path().expect("config path");
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config dir");
    std::fs::write(
        &config_path,
        r#"
        [providers.corporate-claude]
        type = "anthropic-compatible"
        base_url = "https://gateway.example.com/anthropic/v1/"
        auth = "bearer"
        api_key_env = "CORPORATE_CLAUDE_TOKEN"
        default_model = "claude-custom"

        [providers.corporate-claude.headers]
        x-tenant-id = "tenant-42"

        [[providers.corporate-claude.models]]
        id = "claude-custom"
        context_window = 128000
        "#,
    )
    .expect("write config");

    apply_named_provider_profile_env("corporate-claude").expect("apply Anthropic profile");
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_API_BASE").ok().as_deref(),
        Some("https://gateway.example.com/anthropic/v1")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_API_KEY_NAME")
            .ok()
            .as_deref(),
        Some("CORPORATE_CLAUDE_TOKEN")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_AUTH").ok().as_deref(),
        Some("bearer")
    );
    assert_eq!(
        std::env::var("JCODE_ANTHROPIC_MODEL").ok().as_deref(),
        Some("claude-custom")
    );
    let headers: std::collections::BTreeMap<String, String> = serde_json::from_str(
        &std::env::var("JCODE_ANTHROPIC_HEADERS").expect("custom headers env"),
    )
    .expect("headers JSON");
    assert_eq!(
        headers.get("x-tenant-id").map(String::as_str),
        Some("tenant-42")
    );
    assert_eq!(
        std::env::var("JCODE_RUNTIME_PROVIDER").ok().as_deref(),
        Some("anthropic-api")
    );

    if let Some(previous_home) = previous_home {
        crate::env::set_var("JCODE_HOME", previous_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}
#[test]
fn named_provider_profile_maps_to_openai_compatible_runtime_env() {
    let _lock = crate::storage::lock_test_env();
    let _guard = EnvGuard::save(&[
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_MODEL",
        "JCODE_OPENROUTER_STATIC_MODELS",
        "JCODE_OPENROUTER_AUTH_HEADER",
        "JCODE_OPENROUTER_AUTH_HEADER_NAME",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "MY_GATEWAY_API_KEY",
    ]);

    let cfg: crate::config::Config = toml::from_str(
        r#"
        [providers.my-gateway]
        type = "openai-compatible"
        base_url = "https://llm.example.com/v1/"
        auth = "header"
        auth_header = "x-api-key"
        api_key_env = "MY_GATEWAY_API_KEY"
        default_model = "opaque/model@id"
        model_catalog = false

        [[providers.my-gateway.models]]
        id = "opaque/model@id"

        [[providers.my-gateway.models]]
        id = "another-local-id"
        "#,
    )
    .expect("config should parse");

    apply_named_provider_profile_env_from_config("my-gateway", &cfg).expect("apply profile");

    assert_eq!(
        std::env::var("JCODE_OPENROUTER_API_BASE").ok().as_deref(),
        Some("https://llm.example.com/v1")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_API_KEY_NAME")
            .ok()
            .as_deref(),
        Some("MY_GATEWAY_API_KEY")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_PROVIDER_FEATURES")
            .ok()
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_TRANSPORT_STATE")
            .ok()
            .as_deref(),
        Some("direct-api-key")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_MODEL_CATALOG")
            .ok()
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_MODEL").ok().as_deref(),
        Some("opaque/model@id")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_STATIC_MODELS")
            .ok()
            .as_deref(),
        Some("opaque/model@id\nanother-local-id")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_AUTH_HEADER")
            .ok()
            .as_deref(),
        Some("api-key")
    );
    assert_eq!(
        std::env::var("JCODE_OPENROUTER_AUTH_HEADER_NAME")
            .ok()
            .as_deref(),
        Some("x-api-key")
    );
    assert_eq!(
        std::env::var("JCODE_NAMED_PROVIDER_PROFILE")
            .ok()
            .as_deref(),
        Some("my-gateway")
    );
}
#[test]
fn named_provider_inline_api_key_is_private_runtime_fallback() {
    let _lock = crate::storage::lock_test_env();
    let _guard = EnvGuard::save(&[
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_PROVIDER_MY_GATEWAY_API_KEY",
    ]);

    let cfg: crate::config::Config = toml::from_str(
        r#"
        [providers.my-gateway]
        type = "openai-compatible"
        base_url = "https://llm.example.com/v1"
        api_key = "inline-secret"
        "#,
    )
    .expect("config should parse");

    apply_named_provider_profile_env_from_config("my-gateway", &cfg).expect("apply profile");

    assert_eq!(
        std::env::var("JCODE_OPENROUTER_API_KEY_NAME")
            .ok()
            .as_deref(),
        Some("JCODE_PROVIDER_MY_GATEWAY_API_KEY")
    );
    assert_eq!(
        std::env::var("JCODE_PROVIDER_MY_GATEWAY_API_KEY")
            .ok()
            .as_deref(),
        Some("inline-secret")
    );
}
