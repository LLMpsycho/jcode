#[test]
fn test_info_widget_remote_openai_uses_remote_provider_for_usage_and_context() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("OpenAI".to_string());
    app.remote_provider_model = Some("gpt-5.4".to_string());
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::Oauth);
    app.update_context_limit_for_model("gpt-5.4");

    let data = crate::tui::TuiState::info_widget_data(&app);

    assert_eq!(data.provider_name.as_deref(), Some("OpenAI"));
    assert_eq!(data.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(data.context_limit, Some(1_000_000));
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::OpenAIOAuth
    );
    assert_eq!(
        data.usage_info.as_ref().map(|info| info.provider),
        Some(crate::tui::info_widget::UsageProvider::OpenAI)
    );
}
#[test]
fn test_info_widget_remote_model_falls_back_to_model_provider_detection() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_model = Some("gpt-5.4".to_string());
    app.update_context_limit_for_model("gpt-5.4");

    let data = crate::tui::TuiState::info_widget_data(&app);

    assert_eq!(data.context_limit, Some(1_000_000));
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::Unknown
    );
    assert!(
        data.usage_info.is_none(),
        "provider/model detection alone must not guess subscription billing"
    );
}
#[test]
fn test_info_widget_remote_opencode_shows_cost_based_usage() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("opencode".to_string());
    app.remote_provider_model = Some("qwen3-coder".to_string());
    app.token_accounting.total_input_tokens = 12_000;
    app.token_accounting.total_output_tokens = 3_400;

    let data = crate::tui::TuiState::info_widget_data(&app);

    assert_eq!(data.provider_name.as_deref(), Some("opencode"));
    let usage = data.usage_info.as_ref().expect("opencode usage info");
    assert_eq!(
        usage.provider,
        crate::tui::info_widget::UsageProvider::CostBased
    );
    assert!(usage.available);
    assert_eq!(usage.input_tokens, 12_000);
    assert_eq!(usage.output_tokens, 3_400);
}
#[test]
fn test_info_widget_remote_anthropic_api_key_shows_cost_based_usage() {
    // Remote Anthropic sessions billed via API key (server resolves
    // ResolvedCredential::ApiKey) should display cost-based usage instead of
    // subscription bars, mirroring local behavior. OAuth subscription sessions
    // (server resolves ResolvedCredential::Oauth) keep the subscription usage
    // provider.
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("Claude".to_string());
    app.remote_provider_model = Some("claude-sonnet-4-20250514".to_string());
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::ApiKey);
    app.token_accounting.total_input_tokens = 12_000;
    app.token_accounting.total_output_tokens = 3_400;

    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::AnthropicApiKey
    );
    let usage = data
        .usage_info
        .as_ref()
        .expect("remote anthropic api-key usage info");
    assert_eq!(
        usage.provider,
        crate::tui::info_widget::UsageProvider::CostBased
    );
    assert_eq!(usage.input_tokens, 12_000);
    assert_eq!(usage.output_tokens, 3_400);

    // OAuth subscription keeps subscription bars; the server now reports the
    // resolved credential directly, so the widget reflects AnthropicOAuth.
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::Oauth);
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::AnthropicOAuth
    );
    assert_eq!(
        data.usage_info.as_ref().map(|info| info.provider),
        Some(crate::tui::info_widget::UsageProvider::Anthropic)
    );
}
#[test]
fn test_info_widget_remote_openai_billing_follows_resolved_credential() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("OpenAI".to_string());
    app.remote_provider_model = Some("gpt-5.4".to_string());
    app.token_accounting.total_input_tokens = 12_000;
    app.token_accounting.total_output_tokens = 3_400;

    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::ApiKey);
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::OpenAIApiKey
    );
    let usage = data.usage_info.as_ref().expect("OpenAI API cost info");
    assert_eq!(
        usage.provider,
        crate::tui::info_widget::UsageProvider::CostBased
    );
    assert_eq!(usage.input_tokens, 12_000);
    assert_eq!(usage.output_tokens, 3_400);

    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::Oauth);
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::OpenAIOAuth
    );
    assert_eq!(
        data.usage_info.as_ref().map(|usage| usage.provider),
        Some(crate::tui::info_widget::UsageProvider::OpenAI)
    );

    app.remote_resolved_credential = None;
    app.session.route_api_method = None;
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::Unknown
    );
    assert!(data.usage_info.is_none());
}
#[test]
fn test_info_widget_remote_openai_uses_explicit_route_when_credential_is_missing() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("OpenAI".to_string());
    app.remote_provider_model = Some("gpt-5.4".to_string());
    app.remote_resolved_credential = None;

    app.session.route_api_method = Some("openai-api-key".to_string());
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::OpenAIApiKey
    );
    assert_eq!(
        data.usage_info.as_ref().map(|usage| usage.provider),
        Some(crate::tui::info_widget::UsageProvider::CostBased)
    );

    app.session.route_api_method = Some("openai-oauth".to_string());
    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::OpenAIOAuth
    );
    assert_eq!(
        data.usage_info.as_ref().map(|usage| usage.provider),
        Some(crate::tui::info_widget::UsageProvider::OpenAI)
    );
}
#[test]
fn test_info_widget_local_direct_api_runtime_shows_cost_based_usage() {
    let _guard = crate::storage::lock_test_env();
    let tracked_env = [
        "JCODE_RUNTIME_PROVIDER",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_TRANSPORT_STATE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_PROVIDER_PROFILE_ACTIVE",
        "JCODE_PROVIDER_PROFILE_NAME",
    ];
    let saved_env = tracked_env
        .iter()
        .map(|&key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();
    for &key in &tracked_env {
        crate::env::remove_var(key);
    }

    let cases = [
        (
            "claude-api",
            "anthropic",
            "claude-sonnet-4-6",
            crate::tui::info_widget::AuthMethod::AnthropicApiKey,
        ),
        (
            "openai-api",
            "openai",
            "gpt-5.4",
            crate::tui::info_widget::AuthMethod::OpenAIApiKey,
        ),
        (
            "openrouter",
            "openrouter",
            "anthropic/claude-sonnet-4",
            crate::tui::info_widget::AuthMethod::OpenRouterApiKey,
        ),
        (
            "openai-compatible",
            "openrouter",
            "direct-compatible-model",
            crate::tui::info_widget::AuthMethod::ApiKey,
        ),
        (
            "openai-compatible",
            "cerebras",
            "gpt-oss-120b",
            crate::tui::info_widget::AuthMethod::ApiKey,
        ),
        (
            "bedrock",
            "bedrock",
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            crate::tui::info_widget::AuthMethod::ApiKey,
        ),
    ];

    for (runtime_provider, provider_name, model, expected_auth) in cases {
        crate::env::set_var("JCODE_RUNTIME_PROVIDER", runtime_provider);
        crate::env::remove_var("JCODE_OPENROUTER_ALLOW_NO_AUTH");
        crate::auth::AuthStatus::invalidate_cache();

        let mut app = create_named_provider_test_app(provider_name, model);
        app.streaming.streaming_input_tokens = 1_000;
        app.streaming.streaming_output_tokens = 1_000;
        app.token_accounting.total_input_tokens = 12_000;
        app.token_accounting.total_output_tokens = 3_400;
        app.update_cost_impl();

        assert!(
            app.cost.total_cost > 0.0,
            "{runtime_provider} should accrue token cost"
        );

        let data = crate::tui::TuiState::info_widget_data(&app);
        assert_eq!(data.auth_method, expected_auth);
        let usage = data
            .usage_info
            .as_ref()
            .expect("direct API runtime usage info");
        assert_eq!(
            usage.provider,
            crate::tui::info_widget::UsageProvider::CostBased
        );
        assert_eq!(usage.input_tokens, 12_000);
        assert_eq!(usage.output_tokens, 3_400);
        assert!(usage.total_cost > 0.0);
    }

    crate::env::set_var("JCODE_RUNTIME_PROVIDER", "jcode");
    crate::env::remove_var("JCODE_OPENROUTER_ALLOW_NO_AUTH");
    let mut app = create_named_provider_test_app("openrouter", "subscription-model");
    app.streaming.streaming_input_tokens = 1_000;
    app.streaming.streaming_output_tokens = 1_000;
    app.token_accounting.total_input_tokens = 12_000;
    app.token_accounting.total_output_tokens = 3_400;
    app.update_cost_impl();
    assert_eq!(app.cost.total_cost, 0.0);

    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::Unknown
    );
    assert!(data.usage_info.is_none());

    crate::env::set_var("JCODE_RUNTIME_PROVIDER", "openai-compatible");
    crate::env::set_var("JCODE_OPENROUTER_ALLOW_NO_AUTH", "1");
    let mut app = create_named_provider_test_app("openrouter", "local-model");
    app.streaming.streaming_input_tokens = 1_000;
    app.streaming.streaming_output_tokens = 1_000;
    app.token_accounting.total_input_tokens = 12_000;
    app.token_accounting.total_output_tokens = 3_400;
    app.update_cost_impl();
    assert_eq!(app.cost.total_cost, 0.0);

    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::Unknown
    );
    assert!(data.usage_info.is_none());

    for (key, value) in saved_env {
        if let Some(value) = value {
            crate::env::set_var(key, value);
        } else {
            crate::env::remove_var(key);
        }
    }
    crate::auth::AuthStatus::invalidate_cache();
}
#[test]
fn test_anthropic_api_cost_accounts_for_split_cache_tokens() {
    // Anthropic reports usage with *split* accounting: `input_tokens` already
    // excludes cache-read and cache-creation tokens. The cost figure must
    //   - bill fresh input at the input rate,
    //   - bill cache-read tokens at the (cheaper) cache-read rate WITHOUT also
    //     subtracting them from the fresh input (double subtraction), and
    //   - bill cache-creation (cache-write) tokens, which Anthropic charges at a
    //     premium over the input rate.
    let _guard = crate::storage::lock_test_env();
    let saved_runtime = std::env::var_os("JCODE_RUNTIME_PROVIDER");
    crate::env::set_var("JCODE_RUNTIME_PROVIDER", "claude-api");
    crate::auth::AuthStatus::invalidate_cache();

    // claude-sonnet-4-6 API pricing: input $3/Mtok, output $15/Mtok,
    // cache-read $0.30/Mtok. Cache-write (1h TTL) is billed at 2x input = $6/Mtok.
    let mut app = create_named_provider_test_app("anthropic", "claude-sonnet-4-6");
    crate::provider::anthropic::set_cache_ttl_1h(true);

    // A representative cold turn: most of the prompt is freshly written to cache,
    // a little is read back, and only a small uncached remainder is fresh input.
    app.streaming.streaming_input_tokens = 1_000; // uncached fresh input
    app.streaming.streaming_cache_read_tokens = Some(40_000); // served from cache
    app.streaming.streaming_cache_creation_tokens = Some(100_000); // written to cache (premium)
    app.streaming.streaming_output_tokens = 2_000;
    app.update_cost_impl();

    // Expected:
    //   fresh input:    1_000  * $3   / 1e6 = $0.003
    //   output:         2_000  * $15  / 1e6 = $0.030
    //   cache read:    40_000  * $0.3 / 1e6 = $0.012
    //   cache write:  100_000  * $6   / 1e6 = $0.600
    //   total                                = $0.645
    let expected = 0.003 + 0.030 + 0.012 + 0.600;
    assert!(
        (app.cost.total_cost - expected).abs() < 1e-4,
        "anthropic split-accounting cost should be ~${expected:.4}, got ${:.4}",
        app.cost.total_cost
    );

    if let Some(value) = saved_runtime {
        crate::env::set_var("JCODE_RUNTIME_PROVIDER", value);
    } else {
        crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
    }
    crate::auth::AuthStatus::invalidate_cache();
}
#[test]
fn test_remote_anthropic_api_key_accrues_cost_from_token_usage() {
    // The default interactive TUI is a remote client: it receives per-call
    // ServerEvent::TokenUsage but never runs the local finish_turn cost path.
    // Anthropic API-key sessions must still accrue a dollar cost from those
    // events (the server reports tokens, not cost), and OAuth subscription
    // sessions must stay at $0.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("Claude".to_string());
    app.remote_provider_model = Some("claude-sonnet-4-6".to_string());
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::ApiKey);
    crate::provider::anthropic::set_cache_ttl_1h(true);

    // One completed call with split-accounting cache telemetry.
    app.handle_server_event(
        crate::protocol::ServerEvent::TokenUsage {
            input: 1_000,
            output: 2_000,
            cache_read_input: Some(40_000),
            cache_creation_input: Some(100_000),
        },
        &mut remote,
    );

    // Same expected math as the local split-accounting test:
    //   input 1_000 * $3 + output 2_000 * $15 + read 40_000 * $0.3
    //   + write 100_000 * ($3 * 2x) = $0.645
    let expected = 0.003 + 0.030 + 0.012 + 0.600;
    assert!(
        (app.cost.total_cost - expected).abs() < 1e-4,
        "remote anthropic api-key cost should be ~${expected:.4}, got ${:.4}",
        app.cost.total_cost
    );
    assert_eq!(app.token_accounting.total_input_tokens, 1_000);
    assert_eq!(app.token_accounting.total_output_tokens, 2_000);

    // OAuth subscription sessions are not metered per token; cost stays $0.
    let mut oauth_app = create_test_app();
    oauth_app.is_remote = true;
    oauth_app.remote_provider_name = Some("Claude".to_string());
    oauth_app.remote_provider_model = Some("claude-sonnet-4-6".to_string());
    oauth_app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::Oauth);
    oauth_app.handle_server_event(
        crate::protocol::ServerEvent::TokenUsage {
            input: 1_000,
            output: 2_000,
            cache_read_input: Some(40_000),
            cache_creation_input: Some(100_000),
        },
        &mut remote,
    );
    assert_eq!(oauth_app.cost.total_cost, 0.0);
    assert_eq!(oauth_app.token_accounting.total_input_tokens, 1_000);
}
#[test]
fn test_resumed_session_seeds_cost_from_history_token_totals() {
    // Reopening an older session restores token totals from history but never
    // ran the live per-call cost path, so the cost widget showed $0. The resume
    // path must price the restored totals once to seed total_cost.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("Claude".to_string());
    app.remote_provider_model = Some("claude-sonnet-4-6".to_string());
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::ApiKey);
    crate::provider::anthropic::set_cache_ttl_1h(true);

    let totals = crate::protocol::TokenUsageTotals {
        messages_with_token_usage: 3,
        input_tokens: 1_000,
        output_tokens: 2_000,
        cache_reported_input_tokens: 1_000,
        cache_read_input_tokens: 40_000,
        cache_creation_input_tokens: 100_000,
    };
    app.seed_cost_from_history_totals(&totals);

    // Same split-accounting math as the live-call test above.
    let expected = 0.003 + 0.030 + 0.012 + 0.600;
    assert!(
        (app.cost.total_cost - expected).abs() < 1e-4,
        "resumed session cost should be seeded to ~${expected:.4}, got ${:.4}",
        app.cost.total_cost
    );

    // Idempotent: a repeated history snapshot must not double the cost.
    app.seed_cost_from_history_totals(&totals);
    assert!(
        (app.cost.total_cost - expected).abs() < 1e-4,
        "re-seeding must overwrite (not accrue), got ${:.4}",
        app.cost.total_cost
    );

    // OAuth subscription sessions are not metered per token; cost stays $0.
    let mut oauth_app = create_test_app();
    oauth_app.is_remote = true;
    oauth_app.remote_provider_name = Some("Claude".to_string());
    oauth_app.remote_provider_model = Some("claude-sonnet-4-6".to_string());
    oauth_app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::Oauth);
    oauth_app.seed_cost_from_history_totals(&totals);
    assert_eq!(oauth_app.cost.total_cost, 0.0);
}
#[test]
fn test_remote_fast_mode_tier_bills_premium_rates_and_reprices_on_toggle() {
    // `/fast on` (priority tier) bills premium per-token rates on Opus 4.6
    // ($30/$150 vs $5/$25). The pricing memo key includes the tier so toggling
    // fast mode mid-session re-resolves prices instead of reusing stale ones.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_name = Some("Claude".to_string());
    app.remote_provider_model = Some("claude-opus-4-6".to_string());
    app.remote_resolved_credential = Some(jcode_provider_core::ResolvedCredential::ApiKey);

    // Each TokenUsage below simulates a separate completed API call, so reset
    // the per-call usage bookkeeping between them (a real session does this at
    // call start).
    let reset_call_state = |app: &mut App| {
        app.kv_cache.current_api_usage_recorded = false;
        app.streaming.streaming_input_tokens = 0;
        app.streaming.streaming_output_tokens = 0;
        app.streaming.streaming_cache_read_tokens = None;
        app.streaming.streaming_cache_creation_tokens = None;
    };

    // Standard tier first: 1k in / 1k out = $0.005 + $0.025 = $0.030.
    app.remote_service_tier = None;
    reset_call_state(&mut app);
    app.handle_server_event(
        crate::protocol::ServerEvent::TokenUsage {
            input: 1_000,
            output: 1_000,
            cache_read_input: None,
            cache_creation_input: None,
        },
        &mut remote,
    );
    let standard_cost = app.cost.total_cost;
    assert!(
        (standard_cost - 0.030).abs() < 1e-4,
        "standard-tier cost should be ~$0.030, got ${standard_cost:.4}"
    );

    // Fast mode on: same usage now bills $0.030 + $0.150 = $0.180.
    app.remote_service_tier = Some("auto".to_string());
    reset_call_state(&mut app);
    app.handle_server_event(
        crate::protocol::ServerEvent::TokenUsage {
            input: 1_000,
            output: 1_000,
            cache_read_input: None,
            cache_creation_input: None,
        },
        &mut remote,
    );
    let fast_call_cost = app.cost.total_cost - standard_cost;
    assert!(
        (fast_call_cost - 0.180).abs() < 1e-4,
        "fast-mode call cost should be ~$0.180, got ${fast_call_cost:.4}"
    );

    // Fast mode off again: pricing drops back to standard rates.
    app.remote_service_tier = None;
    reset_call_state(&mut app);
    let before = app.cost.total_cost;
    app.handle_server_event(
        crate::protocol::ServerEvent::TokenUsage {
            input: 1_000,
            output: 1_000,
            cache_read_input: None,
            cache_creation_input: None,
        },
        &mut remote,
    );
    let off_call_cost = app.cost.total_cost - before;
    assert!(
        (off_call_cost - 0.030).abs() < 1e-4,
        "post-toggle standard cost should be ~$0.030, got ${off_call_cost:.4}"
    );
}
#[test]
fn test_info_widget_local_gemini_shows_oauth_auth_method() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp dir");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp.path());

    let path = crate::auth::gemini::tokens_path().expect("gemini tokens path");
    crate::storage::write_json_secret(
        &path,
        &serde_json::json!({
            "access_token": "at-123",
            "refresh_token": "rt-456",
            "expires_at": 4102444800000i64,
            "email": "user@example.com"
        }),
    )
    .expect("write gemini tokens");
    crate::auth::AuthStatus::invalidate_cache();

    let app = create_gemini_test_app();
    let data = crate::tui::TuiState::info_widget_data(&app);

    assert_eq!(data.provider_name.as_deref(), Some("gemini"));
    assert_eq!(data.model.as_deref(), Some("gemini-2.5-pro"));
    assert_eq!(
        data.auth_method,
        crate::tui::info_widget::AuthMethod::GeminiOAuth
    );
    assert!(data.usage_info.is_none());

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::auth::AuthStatus::invalidate_cache();
}
