#[test]
fn post_auth_model_selection_prefers_anthropic_flagship_over_catalog_order() {
    // Live Anthropic catalogs list `claude-haiku-4-5-...` before the
    // flagship, and an API-key login supplies no activated model. Plain
    // catalog order would auto-select Haiku; the flagship-first fallback
    // must land on the curated quality-first default (Opus 5) instead.
    let activation = AuthActivationResult {
        provider_id: Some("claude-api".to_string()),
        provider_label: Some("Anthropic".to_string()),
        activated_model: None,
        expected_runtime: None,
        expected_catalog_namespace: None,
    };
    let routes = vec![
        route("claude-haiku-4-5-20251001", "Anthropic", "claude-api", true),
        route("claude-opus-4-6", "Anthropic", "claude-api", true),
        route("claude-opus-4-8", "Anthropic", "claude-api", true),
        route("claude-opus-5", "Anthropic", "claude-api", true),
        route("claude-fable-5", "Anthropic", "claude-api", true),
        route("claude-sonnet-4-6", "Anthropic", "claude-api", true),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-5"),
        "API-key login should auto-select the Anthropic flagship, not the first catalog route"
    );
}
#[test]
fn post_auth_model_selection_preserves_configured_claude_model() {
    let activation = AuthActivationResult {
        provider_id: Some("claude-api".to_string()),
        provider_label: Some("Anthropic".to_string()),
        activated_model: None,
        expected_runtime: None,
        expected_catalog_namespace: None,
    };
    let routes = vec![
        route(
            jcode_provider_core::DEFAULT_CLAUDE_MODEL,
            "Anthropic",
            "claude-api",
            true,
        ),
        route("claude-opus-4-6", "Anthropic", "claude-api", true),
    ];

    assert_eq!(
        provider_model_to_select_after_auth_with_configured_default(
            &activation,
            Some("claude-opus-4-6"),
            Some(jcode_provider_core::DEFAULT_CLAUDE_MODEL),
            &routes,
        )
        .as_deref(),
        Some("claude-opus-4-6")
    );
}
#[test]
fn post_auth_model_selection_prefers_claude_oauth_flagship() {
    let activation = AuthActivationResult {
        provider_id: Some("claude".to_string()),
        provider_label: Some("Anthropic".to_string()),
        activated_model: None,
        expected_runtime: None,
        expected_catalog_namespace: None,
    };
    let routes = vec![
        route("claude-haiku-4-5", "Anthropic", "claude-oauth", true),
        route("claude-opus-4-8", "Anthropic", "claude-oauth", true),
        route("claude-opus-5", "Anthropic", "claude-oauth", true),
        route("claude-fable-5", "Anthropic", "claude-oauth", true),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-5")
    );
}
#[test]
fn post_auth_model_selection_prefers_openai_flagship_over_catalog_order() {
    let activation = AuthActivationResult {
        provider_id: Some("openai-api".to_string()),
        provider_label: Some("OpenAI".to_string()),
        activated_model: None,
        expected_runtime: None,
        expected_catalog_namespace: None,
    };
    let routes = vec![
        route("gpt-5.1", "OpenAI", "openai-api", true),
        route("gpt-5.5", "OpenAI", "openai-api", true),
        route("gpt-5.6-sol", "OpenAI", "openai-api", true),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("gpt-5.6-sol")
    );
}
#[test]
fn global_default_route_prefers_gpt_5_6_over_fable_and_preserves_route() {
    let routes = vec![
        route("gpt-5.5", "OpenAI", "openai-api-key", true),
        route("claude-fable-5", "Anthropic", "anthropic-api-key", true),
        route("gpt-5.6-sol", "OpenAI", "openai-oauth", true),
    ];

    let selected = globally_preferred_default_route(&routes).expect("strongest route");
    assert_eq!(selected.model, "gpt-5.6-sol");
    assert_eq!(selected.provider, "OpenAI");
    assert_eq!(selected.api_method, "openai-oauth");
}
#[test]
fn global_default_route_uses_clean_gpt_5_6_then_fable_before_weaker_models() {
    let clean_release = vec![
        route("claude-fable-5", "Anthropic", "claude-oauth", true),
        route("gpt-5.6", "OpenAI", "openai-api-key", true),
    ];
    assert_eq!(
        globally_preferred_default_route(&clean_release)
            .as_ref()
            .map(|route| route.model.as_str()),
        Some("gpt-5.6")
    );

    let unavailable_gpt = vec![
        route("gpt-5.6-sol", "OpenAI", "openai-api-key", false),
        route("gpt-5.5", "OpenAI", "openai-api-key", true),
        route("claude-fable-5", "Anthropic", "claude-oauth", true),
    ];
    assert_eq!(
        globally_preferred_default_route(&unavailable_gpt)
            .as_ref()
            .map(|route| route.model.as_str()),
        Some("claude-fable-5")
    );
}
#[test]
fn global_default_route_ignores_unavailable_routes_and_preserves_unknown_order() {
    let routes = vec![
        route("provider-a-frontier", "Provider A", "provider-a", true),
        route("gpt-5.6-sol", "OpenAI", "openai-api-key", false),
        route("provider-b-frontier", "Provider B", "provider-b", true),
    ];

    let selected = globally_preferred_default_route(&routes).expect("fallback route");
    assert_eq!(selected.model, "provider-a-frontier");
    assert_eq!(selected.api_method, "provider-a");
}
#[test]
fn post_auth_model_selection_falls_back_when_quality_first_model_is_unavailable() {
    let claude = activation_for_provider_id("claude-api");
    let claude_routes = vec![
        route("claude-opus-5", "Anthropic", "claude-api", false),
        route("claude-fable-5", "Anthropic", "claude-api", true),
        route("claude-opus-4-8", "Anthropic", "claude-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&claude, None, &claude_routes).as_deref(),
        Some("claude-fable-5")
    );

    let openai = activation_for_provider_id("openai-api");
    let openai_routes = vec![
        route("gpt-5.6-sol", "OpenAI", "openai-api", false),
        route("gpt-5.5", "OpenAI", "openai-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&openai, None, &openai_routes).as_deref(),
        Some("gpt-5.5")
    );

    let openai_routes_with_clean_release = vec![
        route("gpt-5.6-sol", "OpenAI", "openai-api", false),
        route("gpt-5.5", "OpenAI", "openai-api", true),
        route("gpt-5.6", "OpenAI", "openai-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&openai, None, &openai_routes_with_clean_release)
            .as_deref(),
        Some("gpt-5.6"),
        "a clean same-generation release should beat GPT 5.5 when Sol is unavailable"
    );
}
#[test]
fn post_auth_model_selection_keeps_catalog_order_for_unranked_providers() {
    // OpenAI-compatible / namespaced providers have no curated flagship
    // order; the fallback must preserve live-catalog order for them.
    //
    // Selection consults the process-global namespaced catalog, so hold the
    // shared test-env lock (and a private JCODE_HOME) or a sibling test's
    // cached catalog decides this assertion depending on ordering.
    let _env = EnvGuard::new(&["JCODE_HOME"]);
    let temp = tempfile::tempdir().expect("tempdir");
    crate::env::set_var("JCODE_HOME", temp.path());
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: None,
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![
        route(
            "llama3.1-8b",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
        route(
            "qwen-3-235b-a22b-instruct-2507",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("llama3.1-8b"),
        "providers without a curated flagship order keep live-catalog order"
    );
}
#[test]
fn post_auth_model_selection_prefers_newest_live_release_for_unranked_provider() {
    let _env = EnvGuard::new(&["JCODE_HOME"]);
    let temp = tempfile::tempdir().expect("tempdir");
    crate::env::set_var("JCODE_HOME", temp.path());
    jcode_provider_openrouter::save_disk_cache_with_source_for_namespace(
        "cerebras",
        &[
            jcode_provider_openrouter::ModelInfo {
                id: "llama3.1-8b".to_string(),
                name: String::new(),
                context_length: None,
                pricing: Default::default(),
                created: Some(1_700_000_000),
            },
            jcode_provider_openrouter::ModelInfo {
                id: "qwen-3-235b-a22b-instruct-2507".to_string(),
                name: String::new(),
                context_length: None,
                pricing: Default::default(),
                created: Some(1_800_000_000),
            },
        ],
        Some("https://api.cerebras.ai/v1"),
    );

    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: None,
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![
        route(
            "llama3.1-8b",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
        route(
            "qwen-3-235b-a22b-instruct-2507",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("qwen-3-235b-a22b-instruct-2507"),
        "unranked providers should prefer the newest live release when the catalog includes release timestamps"
    );
}
#[test]
fn post_auth_auto_promotes_newer_frontier_release_not_yet_in_curated_list() {
    // The day Anthropic ships a stronger Opus than the curated flagship, the
    // live catalog carries it and it must auto-promote to the post-login
    // default without a code change. Here `claude-opus-4-9` beats the curated
    // baseline `claude-opus-4-8`.
    let activation = activation_for_provider_id("claude-api");
    let routes = vec![
        route("claude-haiku-4-5", "Anthropic", "claude-api", true),
        route("claude-opus-4-8", "Anthropic", "claude-api", true),
        route("claude-opus-4-9", "Anthropic", "claude-api", true),
        route("claude-sonnet-4-6", "Anthropic", "claude-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-4-9"),
        "a newer pure Opus flagship in the live catalog should auto-promote"
    );

    // Same for OpenAI: a future `gpt-5.7` beats the curated Sol 5.6 baseline.
    let activation = activation_for_provider_id("openai");
    let routes = vec![
        route("gpt-5-mini", "OpenAI", "openai", true),
        route("gpt-5.6-sol", "OpenAI", "openai", true),
        route("gpt-5.7", "OpenAI", "openai", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("gpt-5.7")
    );
}
#[test]
fn quality_first_defaults_are_not_displaced_by_lower_family_or_equal_release() {
    let claude = activation_for_provider_id("claude-api");
    let claude_routes = vec![
        route("claude-opus-4-9", "Anthropic", "claude-api", true),
        route("claude-fable-5", "Anthropic", "claude-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&claude, None, &claude_routes).as_deref(),
        Some("claude-fable-5"),
        "a newer lower-priority Opus release must not displace available Fable"
    );

    let openai = activation_for_provider_id("openai");
    let openai_routes = vec![
        route("gpt-5.6", "OpenAI", "openai", true),
        route("gpt-5.6-sol", "OpenAI", "openai", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&openai, None, &openai_routes).as_deref(),
        Some("gpt-5.6-sol"),
        "the base model at the same release must not displace the Sol quality profile"
    );
}
#[test]
fn post_auth_frontier_promotion_ignores_cheaper_and_specialized_variants() {
    // A newer *cheaper/specialized* variant must NOT auto-promote over the
    // curated flagship: only clean flagship ids qualify. Even though
    // `claude-haiku-5` and `gpt-6-mini`/`gpt-6-codex` have higher version
    // numbers, they carry non-flagship tier words and must be rejected, so
    // selection stays on the curated flagship.
    let activation = activation_for_provider_id("claude-api");
    let routes = vec![
        route("claude-haiku-5", "Anthropic", "claude-api", true),
        route("claude-opus-4-8", "Anthropic", "claude-api", true),
        route("claude-sonnet-5", "Anthropic", "claude-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-4-8"),
        "cheaper/other-family models must not auto-promote over the curated Opus flagship"
    );

    let activation = activation_for_provider_id("openai");
    let routes = vec![
        route("gpt-6-mini", "OpenAI", "openai", true),
        route("gpt-6-codex", "OpenAI", "openai", true),
        route("gpt-5.5", "OpenAI", "openai", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("gpt-5.5"),
        "mini/codex variants must not auto-promote over the clean gpt flagship"
    );
}
#[test]
fn post_auth_frontier_promotion_no_op_when_curated_is_still_newest() {
    // When the live catalog contains nothing newer than the curated flagship,
    // the curated quality order decides and frontier promotion is a no-op.
    let activation = activation_for_provider_id("claude-api");
    let routes = vec![
        route("claude-haiku-4-5-20251001", "Anthropic", "claude-api", true),
        route("claude-opus-4-6", "Anthropic", "claude-api", true),
        route("claude-opus-4-8", "Anthropic", "claude-api", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-4-8")
    );
}
#[test]
fn post_auth_frontier_promotion_covers_bedrock_and_gemini() {
    // Bedrock: a newer Opus 5 (vendor-prefixed + dated) auto-promotes over the
    // curated Opus 4 baseline, and never falls back to the year-old 3.5.
    let activation = activation_for_provider_id("bedrock");
    let routes = vec![
        route(
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "AWS Bedrock",
            "bedrock",
            true,
        ),
        route(
            "anthropic.claude-opus-4-20250514-v1:0",
            "AWS Bedrock",
            "bedrock",
            true,
        ),
        route(
            "anthropic.claude-opus-5-20260101-v1:0",
            "AWS Bedrock",
            "bedrock",
            true,
        ),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("anthropic.claude-opus-5-20260101-v1:0"),
        "a newer Bedrock Opus must auto-promote over the curated Opus 4"
    );

    // Gemini: a newer pro auto-promotes; a newer flash never displaces it.
    let activation = activation_for_provider_id("gemini");
    let routes = vec![
        route(
            "gemini-2.5-flash",
            "Google Gemini",
            "code-assist-oauth",
            true,
        ),
        route(
            "gemini-3-pro-preview",
            "Google Gemini",
            "code-assist-oauth",
            true,
        ),
        route(
            "gemini-4-pro-preview",
            "Google Gemini",
            "code-assist-oauth",
            true,
        ),
        route(
            "gemini-9-flash-preview",
            "Google Gemini",
            "code-assist-oauth",
            true,
        ),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("gemini-4-pro-preview"),
        "the newest Gemini *pro* must win; a higher-numbered flash must not"
    );
}
#[test]
fn frontier_version_parsing_and_compare() {
    let fams = &[
        FrontierFamily {
            prefix: "claude-opus",
            flagship_token: None,
        },
        FrontierFamily {
            prefix: "gpt",
            flagship_token: None,
        },
    ];
    // Clean flagship ids parse with a version vector.
    let opus = parse_frontier_model("claude-opus-4-8", fams).expect("opus parses");
    assert_eq!(opus.family, "claude-opus");
    assert_eq!(opus.version, vec![4, 8]);
    let gpt = parse_frontier_model("gpt-5.5", fams).expect("gpt parses");
    assert_eq!(gpt.family, "gpt");
    assert_eq!(gpt.version, vec![5, 5]);
    // Dated id parses on the canonical base.
    assert_eq!(
        parse_frontier_model("claude-opus-4-9-20260101", fams)
            .expect("dated opus parses")
            .version,
        vec![4, 9]
    );
    // Specialized/cheap tiers and other families are rejected.
    assert!(parse_frontier_model("claude-haiku-5", fams).is_none());
    assert!(parse_frontier_model("gpt-6-mini", fams).is_none());
    assert!(parse_frontier_model("gpt-5-codex", fams).is_none());
    assert!(parse_frontier_model("claude-sonnet-5", fams).is_none());
    // Version comparison is component-wise with zero-padding.
    assert_eq!(version_cmp(&[4, 8], &[4, 9]), std::cmp::Ordering::Less);
    assert_eq!(version_cmp(&[5], &[5, 1]), std::cmp::Ordering::Less);
    assert_eq!(version_cmp(&[6], &[5, 9]), std::cmp::Ordering::Greater);
    assert_eq!(version_cmp(&[5, 5], &[5, 5]), std::cmp::Ordering::Equal);

    // Bedrock vendor-prefixed/versioned ids normalize to the bare Claude
    // family and parse as flagship.
    let bedrock = parse_frontier_model(
        "us.anthropic.claude-opus-4-20250514-v1:0",
        &[FrontierFamily {
            prefix: "claude-opus",
            flagship_token: None,
        }],
    )
    .expect("bedrock opus parses");
    assert_eq!(bedrock.version, vec![4]);

    // Gemini flagship token: `pro` is required and `flash`/`lite` are rejected.
    let gem_fams = &[FrontierFamily {
        prefix: "gemini",
        flagship_token: Some("pro"),
    }];
    let gpro = parse_frontier_model("gemini-3-pro-preview", gem_fams).expect("gemini pro");
    assert_eq!(gpro.family, "gemini");
    assert_eq!(gpro.version, vec![3]);
    assert_eq!(
        parse_frontier_model("gemini-3.1-pro", gem_fams)
            .expect("gemini 3.1 pro")
            .version,
        vec![3, 1]
    );
    assert!(
        parse_frontier_model("gemini-3-flash", gem_fams).is_none(),
        "gemini flash is the cheap tier and must not be a frontier flagship"
    );
    assert!(
        parse_frontier_model("gemini-2.5-flash-lite", gem_fams).is_none(),
        "gemini flash-lite must be rejected"
    );
}
#[test]
fn normalize_model_for_preference_strips_hosted_prefixes_and_suffixes() {
    assert_eq!(
        normalize_model_for_preference("us.anthropic.claude-opus-4-20250514-v1:0"),
        "claude-opus-4"
    );
    assert_eq!(
        normalize_model_for_preference("anthropic.claude-3-5-sonnet-20241022-v2:0"),
        "claude-3-5-sonnet"
    );
    assert_eq!(
        normalize_model_for_preference("models/gemini-3-pro-preview"),
        "gemini-3-pro"
    );
    assert_eq!(
        normalize_model_for_preference("accounts/fireworks/models/qwen3-coder"),
        "qwen3-coder"
    );
    // Non-hosted ids are unchanged apart from canonicalization.
    assert_eq!(
        normalize_model_for_preference("claude-haiku-4-5-20251001"),
        "claude-haiku-4-5"
    );
    assert_eq!(normalize_model_for_preference("gpt-5.5"), "gpt-5.5");
}
