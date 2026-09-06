use super::*;
use std::sync::MutexGuard;

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let lock = crate::storage::lock_test_env();
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in keys {
            crate::env::remove_var(key);
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for EnvGuard {
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

fn route(model: &str, provider: &str, api_method: &str, available: bool) -> ModelRoute {
    ModelRoute {
        model: model.to_string(),
        provider: provider.to_string(),
        api_method: api_method.to_string(),
        available,
        detail: String::new(),
        cheapness: None,
    }
}

#[test]
fn api_key_login_replaces_stale_process_env_with_saved_file_key() {
    // Issue #453: a server process that inherited a stale ANTHROPIC_API_KEY
    // must start using the key that /login just wrote to anthropic.env.
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");
    crate::env::set_var("ANTHROPIC_API_KEY", "stale-inherited-key");
    sandbox
        .write_env_file("anthropic.env", "ANTHROPIC_API_KEY", "fresh-login-key")
        .expect("write env file");

    let mut auth = AuthChanged::new("claude-api");
    auth.credential_source = Some(crate::protocol::AuthCredentialSource::ApiKeyFile);
    auth.auth_method = Some(crate::protocol::AuthMethod::TuiPasteApiKey);
    let _ = activate_auth_change(&AuthActivationRequest::new(None, Some(auth)));

    assert_eq!(
        std::env::var("ANTHROPIC_API_KEY").as_deref(),
        Ok("fresh-login-key")
    );
    assert_eq!(
        crate::provider_catalog::load_api_key_from_env_or_config(
            "ANTHROPIC_API_KEY",
            "anthropic.env"
        )
        .as_deref(),
        Some("fresh-login-key"),
        "credential resolution must use the freshly saved key"
    );
}

#[test]
fn api_key_login_invalidates_auth_status_cached_before_activation() {
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");
    assert_eq!(
        crate::auth::AuthStatus::check_fast().openrouter,
        crate::auth::AuthState::NotConfigured
    );
    sandbox
        .write_env_file("openrouter.env", "OPENROUTER_API_KEY", "fresh-login-key")
        .expect("write env file");

    let mut auth = AuthChanged::new("openrouter");
    auth.credential_source = Some(crate::protocol::AuthCredentialSource::ApiKeyFile);
    auth.auth_method = Some(crate::protocol::AuthMethod::TuiPasteApiKey);
    let _ = activate_auth_change(&AuthActivationRequest::new(None, Some(auth)));

    assert_eq!(
        crate::auth::AuthStatus::check_fast().openrouter,
        crate::auth::AuthState::Available,
        "catalog refresh must not reuse the pre-activation auth snapshot"
    );
}

#[test]
fn legacy_hint_only_auth_change_still_syncs_saved_file_key() {
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");
    crate::env::set_var("ANTHROPIC_API_KEY", "stale-inherited-key");
    sandbox
        .write_env_file("anthropic.env", "ANTHROPIC_API_KEY", "fresh-login-key")
        .expect("write env file");

    let _ = activate_auth_change(&AuthActivationRequest::new(
        Some("anthropic-api".to_string()),
        None,
    ));

    assert_eq!(
        std::env::var("ANTHROPIC_API_KEY").as_deref(),
        Ok("fresh-login-key")
    );
}

#[test]
fn oauth_auth_change_does_not_touch_api_key_process_env() {
    let sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");
    crate::env::set_var("ANTHROPIC_API_KEY", "env-key-left-alone");
    sandbox
        .write_env_file("anthropic.env", "ANTHROPIC_API_KEY", "file-key")
        .expect("write env file");

    let mut auth = AuthChanged::new("claude-api");
    auth.auth_method = Some(crate::protocol::AuthMethod::OAuthBrowser);
    auth.credential_source = Some(crate::protocol::AuthCredentialSource::OAuthTokenStore);
    let _ = activate_auth_change(&AuthActivationRequest::new(None, Some(auth)));

    assert_eq!(
        std::env::var("ANTHROPIC_API_KEY").as_deref(),
        Ok("env-key-left-alone")
    );
}

#[test]
fn direct_auth_catalog_matching_preserves_oauth_vs_api_key_route_identity() {
    for (provider_id, provider_label, matching_provider, matching_method, stale_method) in [
        (
            "claude",
            "Anthropic/Claude",
            "Anthropic",
            "claude-oauth",
            "claude-api",
        ),
        (
            "claude-api",
            "Anthropic API",
            "Anthropic",
            "claude-api",
            "claude-oauth",
        ),
        (
            "openai",
            "OpenAI",
            "OpenAI",
            "openai-oauth",
            "openai-api-key",
        ),
        (
            "openai-api",
            "OpenAI API",
            "OpenAI",
            "openai-api-key",
            "openai-oauth",
        ),
    ] {
        let activation = AuthActivationResult {
            provider_id: Some(provider_id.to_string()),
            provider_label: Some(provider_label.to_string()),
            activated_model: Some("shared-model".to_string()),
            expected_runtime: None,
            expected_catalog_namespace: None,
        };
        let routes = vec![
            route("shared-model", matching_provider, stale_method, true),
            route("shared-model", matching_provider, matching_method, true),
        ];

        let report = validate_catalog_invariants(&activation, Some("shared-model"), &routes);
        assert!(
            report.ok(),
            "{provider_id} should match {matching_method}: {report:?}"
        );
        assert_eq!(report.selectable_provider_routes, 1);
        assert_eq!(
            report.route_sample,
            vec![format!("`shared-model` via {matching_method}")]
        );
        assert_eq!(
            provider_model_to_select_after_auth(&activation, Some("shared-model"), &routes),
            Some("shared-model".to_string()),
            "duplicate model IDs must force a provider-explicit model switch for {provider_id}"
        );
    }
}

#[test]
fn typed_auth_request_provider_id_wins_over_legacy_hint() {
    let request = AuthActivationRequest::new(
        Some("openai".to_string()),
        Some(AuthChanged::new("cerebras")),
    );

    assert_eq!(request.provider_id().as_deref(), Some("cerebras"));
    assert_eq!(
        provider_display_label(request.provider_id().as_deref()).as_deref(),
        Some("Cerebras")
    );
}

#[test]
fn direct_login_provider_ids_are_normalized_with_display_labels() {
    for (hint, normalized, label) in [
        ("claude", "claude", "Anthropic/Claude"),
        ("anthropic", "claude", "Anthropic/Claude"),
        ("anthropic-api", "claude-api", "Anthropic API"),
        ("claude-api", "claude-api", "Anthropic API"),
        ("openai", "openai", "OpenAI"),
        ("openai-key", "openai-api", "OpenAI API"),
        ("openrouter", "openrouter", "OpenRouter"),
        ("subscription", "jcode", "Jcode Subscription"),
        ("bedrock", "bedrock", "AWS Bedrock"),
        ("cursor", "cursor", "Cursor"),
        ("copilot", "copilot", "GitHub Copilot"),
        ("gemini", "gemini", "Google Gemini"),
        ("antigravity", "antigravity", "Antigravity"),
    ] {
        assert_eq!(normalized_auth_provider_id(Some(hint)), Some(normalized));
        assert_eq!(provider_display_label(Some(hint)).as_deref(), Some(label));
    }
}

#[test]
fn every_model_login_provider_has_explicit_lifecycle_normalization() {
    let mut missing = Vec::new();
    for provider in crate::provider_catalog::login_providers() {
        let is_non_model_auth_surface = matches!(
            provider.target,
            crate::provider_catalog::LoginProviderTarget::AutoImport
                | crate::provider_catalog::LoginProviderTarget::Google
        );
        let normalized = normalized_auth_provider_id(Some(provider.id));
        if is_non_model_auth_surface {
            assert!(
                normalized.is_none(),
                "non-model auth provider {} should stay out of model lifecycle normalization",
                provider.id
            );
        } else if normalized.is_none() {
            missing.push(provider.id);
        }
    }

    assert!(
        missing.is_empty(),
        "model login providers missing lifecycle normalization: {:?}",
        missing
    );
}

#[test]
fn direct_login_provider_activation_sets_runtime_identity_and_active_provider() {
    // Sandbox JCODE_HOME so activation's env-file credential sync (#453)
    // cannot read the developer's real ~/.config/jcode/*.env files and
    // leak keys into this process during the matrix run.
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");

    for (provider, runtime, active) in [
        ("claude", "claude", "claude"),
        ("claude-api", "claude-api", "claude"),
        ("openai", "openai", "openai"),
        ("openai-api", "openai-api", "openai"),
        ("openrouter", "openrouter", "openrouter"),
        ("jcode", "jcode", "openrouter"),
        ("bedrock", "bedrock", "bedrock"),
        ("cursor", "cursor", "cursor"),
        ("copilot", "copilot", "copilot"),
        ("gemini", "gemini", "gemini"),
        ("antigravity", "antigravity", "antigravity"),
    ] {
        crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        crate::env::remove_var("JCODE_ACTIVE_PROVIDER");
        crate::env::remove_var("JCODE_INITIAL_PROVIDER_EXPLICIT");

        let activation = activate_auth_change(&AuthActivationRequest::new(
            None,
            Some(AuthChanged::new(provider)),
        ));

        assert_eq!(activation.provider_id.as_deref(), Some(provider));
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").as_deref(),
            Ok(runtime)
        );
        assert_eq!(
            std::env::var("JCODE_ACTIVE_PROVIDER").as_deref(),
            Ok(active)
        );
        assert_eq!(
            std::env::var("JCODE_INITIAL_PROVIDER_EXPLICIT").as_deref(),
            Ok("1")
        );
    }
}

#[test]
fn direct_login_provider_descriptor_matrix_has_full_lifecycle_parity() {
    // Sandbox JCODE_HOME for the same reason as the activation matrix
    // above: keep the #453 credential sync away from real env files.
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().expect("sandbox");

    let mut covered = Vec::new();
    for provider in crate::provider_catalog::login_providers() {
        let Some((normalized, runtime, active, switch_prefix)) = (match provider.target {
            crate::provider_catalog::LoginProviderTarget::Jcode => {
                Some(("jcode", "jcode", "openrouter", ""))
            }
            crate::provider_catalog::LoginProviderTarget::Claude => {
                Some(("claude", "claude", "claude", "claude-oauth"))
            }
            crate::provider_catalog::LoginProviderTarget::ClaudeApiKey => {
                Some(("claude-api", "claude-api", "claude", "claude-api"))
            }
            crate::provider_catalog::LoginProviderTarget::OpenAi => {
                Some(("openai", "openai", "openai", "openai-oauth"))
            }
            crate::provider_catalog::LoginProviderTarget::OpenAiApiKey => {
                Some(("openai-api", "openai-api", "openai", "openai-api"))
            }
            crate::provider_catalog::LoginProviderTarget::OpenRouter => {
                Some(("openrouter", "openrouter", "openrouter", "openrouter"))
            }
            crate::provider_catalog::LoginProviderTarget::Bedrock => {
                Some(("bedrock", "bedrock", "bedrock", "bedrock"))
            }
            crate::provider_catalog::LoginProviderTarget::Cursor => {
                Some(("cursor", "cursor", "cursor", "cursor"))
            }
            crate::provider_catalog::LoginProviderTarget::Copilot => {
                Some(("copilot", "copilot", "copilot", "copilot"))
            }
            crate::provider_catalog::LoginProviderTarget::Gemini => {
                Some(("gemini", "gemini", "gemini", "gemini"))
            }
            crate::provider_catalog::LoginProviderTarget::Antigravity => {
                Some(("antigravity", "antigravity", "antigravity", "antigravity"))
            }
            _ => None,
        }) else {
            continue;
        };

        covered.push(provider.id);
        assert_eq!(
            normalized_auth_provider_id(Some(provider.id)),
            Some(normalized),
            "{} descriptor id must normalize into the auth lifecycle",
            provider.id
        );
        for alias in provider.aliases {
            assert_eq!(
                normalized_auth_provider_id(Some(alias)),
                Some(normalized),
                "{} alias `{}` must normalize into the same auth lifecycle provider",
                provider.id,
                alias
            );
        }
        assert_eq!(
            provider_display_label(Some(provider.id)).as_deref(),
            Some(provider.display_name),
            "{} descriptor display label must be user-visible auth label",
            provider.id
        );

        crate::env::remove_var("JCODE_RUNTIME_PROVIDER");
        crate::env::remove_var("JCODE_ACTIVE_PROVIDER");
        crate::env::remove_var("JCODE_INITIAL_PROVIDER_EXPLICIT");

        let activation = activate_auth_change(&AuthActivationRequest::new(
            None,
            Some(AuthChanged::new(provider.id)),
        ));
        assert_eq!(activation.provider_id.as_deref(), Some(normalized));
        assert_eq!(
            activation.provider_label.as_deref(),
            Some(provider.display_name)
        );
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").as_deref(),
            Ok(runtime)
        );
        assert_eq!(
            std::env::var("JCODE_ACTIVE_PROVIDER").as_deref(),
            Ok(active)
        );
        assert_eq!(
            std::env::var("JCODE_INITIAL_PROVIDER_EXPLICIT").as_deref(),
            Ok("1")
        );
        let expected_switch = if switch_prefix.is_empty() {
            "shared-model".to_string()
        } else {
            format!("{switch_prefix}:shared-model")
        };
        assert_eq!(
            activation.model_switch_request("ignored-runtime", "shared-model"),
            expected_switch,
            "{} direct auth model switch must preserve its canonical route identity",
            provider.id
        );
    }

    for expected in [
        "claude",
        "anthropic-api",
        "openai",
        "openai-api",
        "openrouter",
        "jcode",
        "bedrock",
        "cursor",
        "copilot",
        "gemini",
        "antigravity",
    ] {
        assert!(
            covered.contains(&expected),
            "direct provider parity matrix did not cover {expected}: {covered:?}"
        );
    }
}

#[test]
fn model_switch_request_prefixes_openai_compatible_profiles_with_profile_id() {
    assert_eq!(
        model_switch_request_for_provider_id(Some("cerebras"), "mock-auth", "llama3.1-8b"),
        "cerebras:llama3.1-8b"
    );
    assert_eq!(
        model_switch_request_for_provider_id(Some("cerebras"), "openrouter", "llama3.1-8b"),
        "cerebras:llama3.1-8b"
    );
}

#[test]
fn model_switch_request_is_provider_explicit_for_all_auth_providers() {
    for (provider, expected) in [
        ("claude", "claude-oauth:shared-model"),
        ("anthropic", "claude-oauth:shared-model"),
        ("anthropic-api", "claude-api:shared-model"),
        ("openai", "openai-oauth:shared-model"),
        ("openai-api", "openai-api:shared-model"),
        ("openrouter", "openrouter:shared-model"),
        ("jcode", "shared-model"),
        ("azure-openai", "openrouter:shared-model"),
        ("bedrock", "bedrock:shared-model"),
        ("cursor", "cursor:shared-model"),
        ("copilot", "copilot:shared-model"),
        ("gemini", "gemini:shared-model"),
        ("antigravity", "antigravity:shared-model"),
        ("cerebras", "cerebras:shared-model"),
    ] {
        assert_eq!(
            model_switch_request_for_provider_id(Some(provider), "mock-auth", "shared-model"),
            expected,
            "{provider} auth switch request must route explicitly so duplicate model IDs cannot select the wrong provider"
        );
    }
}

#[test]
fn jcode_auth_lifecycle_matches_only_managed_subscription_routes() {
    let activation = AuthActivationResult {
        provider_id: Some("jcode".to_string()),
        provider_label: Some("Jcode Subscription".to_string()),
        activated_model: Some("gpt-5.5".to_string()),
        expected_runtime: Some("jcode-subscription".to_string()),
        expected_catalog_namespace: Some("jcode-subscription".to_string()),
    };
    let routes = vec![
        route("gpt-5.5", "OpenRouter", "openrouter", true),
        route(
            "gpt-5.5",
            "Jcode Subscription",
            crate::subscription_catalog::JCODE_ROUTE_API_METHOD,
            true,
        ),
    ];

    let report = validate_catalog_invariants(&activation, Some("gpt-5.5"), &routes);
    assert!(
        report.ok(),
        "canonical Jcode route should match: {report:?}"
    );
    assert_eq!(report.selectable_provider_routes, 1);
    assert_eq!(
        report.route_sample,
        vec![format!(
            "`gpt-5.5` via {}",
            crate::subscription_catalog::JCODE_ROUTE_API_METHOD
        )]
    );
    assert_eq!(
        activation.model_switch_request("Jcode Subscription", "gpt-5.5"),
        "gpt-5.5"
    );
}

#[test]
fn post_auth_model_selection_reselects_duplicate_model_name_from_matching_provider_route() {
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: Some("llama3.1-8b".to_string()),
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![
        route(
            "llama3.1-8b",
            "Other Gateway",
            "openai-compatible:other",
            true,
        ),
        route(
            "llama3.1-8b",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, Some("llama3.1-8b"), &routes),
        Some("llama3.1-8b".to_string()),
        "duplicate model IDs must force an explicit provider-profile model switch"
    );
}

#[test]
fn catalog_invariants_pass_when_selected_model_matches_provider_route() {
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: Some("llama3.1-8b".to_string()),
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![
        route("gpt-5.5", "OpenAI", "openai", true),
        route(
            "llama3.1-8b",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
    ];

    let report = validate_catalog_invariants(&activation, Some("llama3.1-8b"), &routes);

    assert!(
        report.ok(),
        "unexpected warning: {:?}",
        report.warning_message()
    );
    assert_eq!(report.selectable_provider_routes, 1);
}

#[test]
fn catalog_invariants_reject_generic_openai_compatible_route_for_namespaced_auth() {
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: Some("llama3.1-8b".to_string()),
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![route("llama3.1-8b", "Cerebras", "openai-compatible", true)];

    let report = validate_catalog_invariants(&activation, Some("llama3.1-8b"), &routes);

    assert!(
        !report.ok(),
        "generic openai-compatible route should not satisfy namespaced auth: {report:?}"
    );
    assert_eq!(report.selectable_provider_routes, 0);
    assert!(
        report
            .warning_message()
            .expect("warning")
            .contains("Expected selectable Cerebras model routes")
    );
}

#[test]
fn catalog_invariants_warn_when_selected_model_is_from_stale_provider() {
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: Some("llama3.1-8b".to_string()),
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![route("gpt-5.5", "OpenAI", "openai", true)];

    let report = validate_catalog_invariants(&activation, Some("gpt-5.5"), &routes);

    assert!(!report.ok());
    let warning = report.warning_message().expect("warning expected");
    assert!(warning.contains("Expected selectable Cerebras model routes"));
    assert!(warning.contains("Selected model: `gpt-5.5`"));
}

#[test]
fn post_auth_model_selection_prefers_matching_provider_route_over_stale_model() {
    let activation = AuthActivationResult {
        provider_id: Some("cerebras".to_string()),
        provider_label: Some("Cerebras".to_string()),
        activated_model: Some("qwen-3-235b-a22b-instruct-2507".to_string()),
        expected_runtime: Some("openai-compatible".to_string()),
        expected_catalog_namespace: Some("cerebras".to_string()),
    };
    let routes = vec![
        route("gpt-5.5", "OpenAI", "openai", true),
        route(
            "qwen-3-235b-a22b-instruct-2507",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
        route(
            "llama3.1-8b",
            "Cerebras",
            "openai-compatible:cerebras",
            true,
        ),
    ];

    assert_eq!(
        provider_model_to_select_after_auth(&activation, Some("gpt-5.5"), &routes).as_deref(),
        Some("qwen-3-235b-a22b-instruct-2507")
    );
    assert_eq!(
        provider_model_to_select_after_auth(
            &activation,
            Some("qwen-3-235b-a22b-instruct-2507"),
            &routes
        ),
        None
    );
}

/// The set of canonical provider ids whose post-login fallback must apply a
/// curated flagship-first order. These are the providers that expose
/// Claude/OpenAI models under their bare canonical ids and report no
/// `activated_model`, so a "cheap model first" catalog would otherwise
/// auto-select the wrong default. Kept here as the single source of truth
/// the exhaustive walk asserts against.
const RANKED_PROVIDER_IDS: &[&str] = &[
    "claude",
    "claude-api",
    "openai",
    "openai-api",
    "copilot",
    "cursor",
    "bedrock",
    "azure-openai",
    "gemini",
    "antigravity",
];

fn activation_for_provider_id(provider_id: &str) -> AuthActivationResult {
    AuthActivationResult {
        provider_id: Some(provider_id.to_string()),
        provider_label: provider_display_label(Some(provider_id)),
        activated_model: None,
        expected_runtime: None,
        expected_catalog_namespace: None,
    }
}

/// Exhaustive walk: every login provider descriptor is classified as ranked
/// (curated flagship order) or unranked (catalog order), and the
/// classification must exactly match RANKED_PROVIDER_IDS. This is the guard
/// that catches a newly added provider that proxies Claude/OpenAI models but
/// forgets to opt into the flagship-first fallback.
#[test]
fn post_auth_model_selection_classifies_every_login_provider() {
    let mut ranked_seen: std::collections::BTreeSet<String> = Default::default();
    for descriptor in crate::provider_catalog::login_providers() {
        let Some(provider_id) = normalized_auth_provider_id(Some(descriptor.id)) else {
            // AutoImport / non-runtime descriptors have no activation id.
            continue;
        };
        let activation = activation_for_provider_id(provider_id);
        let ranked = !provider_preferred_model_orders(&activation).is_empty();
        let expected = RANKED_PROVIDER_IDS.contains(&provider_id);
        assert_eq!(
            ranked, expected,
            "login provider `{}` (id `{}`) classified ranked={ranked}, expected {expected}; \
             if this is a new Claude/OpenAI-proxying provider add it to \
             provider_preferred_model_orders + RANKED_PROVIDER_IDS, otherwise leave it unranked",
            descriptor.id, provider_id
        );
        if ranked {
            ranked_seen.insert(provider_id.to_string());
        }
    }
    let expected_ranked: std::collections::BTreeSet<String> = RANKED_PROVIDER_IDS
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        ranked_seen, expected_ranked,
        "the ranked providers reachable from the login catalog drifted from RANKED_PROVIDER_IDS"
    );
}

/// Exhaustive walk: for every ranked provider, an adversarial catalog that
/// lists the cheapest model first must still auto-select the curated
/// flagship after login. This is the direct regression for the live
/// Anthropic API-key login that auto-selected Haiku instead of Opus.
#[test]
fn post_auth_model_selection_picks_flagship_for_every_ranked_provider() {
    // (provider_id, api_method, provider_display, cheap_first_routes, expected flagship)
    let cases: &[(&str, &str, &str, &[&str], &str)] = &[
        (
            "claude",
            "claude-oauth",
            "Anthropic",
            &["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-8"],
            "claude-opus-4-8",
        ),
        (
            "claude-api",
            "claude-api",
            "Anthropic",
            &[
                "claude-haiku-4-5-20251001",
                "claude-sonnet-4-6",
                "claude-opus-4-8",
            ],
            "claude-opus-4-8",
        ),
        (
            "openai",
            "openai-oauth",
            "OpenAI",
            &["gpt-5-nano", "gpt-5.1", "gpt-5.5"],
            "gpt-5.5",
        ),
        (
            "openai-api",
            "openai-api-key",
            "OpenAI",
            &["gpt-5-mini", "gpt-5.1", "gpt-5.5"],
            "gpt-5.5",
        ),
        (
            // Copilot proxies Claude under canonical ids: Opus must beat Haiku.
            "copilot",
            "copilot",
            "Copilot",
            &["claude-haiku-4-5", "gpt-5.5", "claude-opus-4-8"],
            "claude-opus-4-8",
        ),
        (
            // Cursor likewise: an all-OpenAI catalog still picks the flagship.
            "cursor",
            "cursor",
            "Cursor",
            &["gpt-5-nano", "gpt-5.1", "gpt-5.5"],
            "gpt-5.5",
        ),
        (
            // Bedrock lists year-old Claude first; the curated order must
            // still pick Opus 4 over claude-3-5-sonnet. Bedrock ids carry the
            // vendor prefix + version tag, normalized away before ranking.
            "bedrock",
            "bedrock",
            "AWS Bedrock",
            &[
                "anthropic.claude-3-5-sonnet-20241022-v2:0",
                "anthropic.claude-3-5-haiku-20241022-v1:0",
                "anthropic.claude-sonnet-4-20250514-v1:0",
                "anthropic.claude-opus-4-20250514-v1:0",
            ],
            "anthropic.claude-opus-4-20250514-v1:0",
        ),
        (
            // Azure hosts the OpenAI family over the OpenRouter transport.
            "azure-openai",
            "openrouter",
            "Azure OpenAI",
            &["gpt-5-mini", "gpt-5.1", "gpt-5.5"],
            "gpt-5.5",
        ),
        (
            // Gemini's flagship tier is `pro`; a flash-first catalog must
            // still pick the strongest pro model.
            "gemini",
            "code-assist-oauth",
            "Google Gemini",
            &["gemini-2.5-flash", "gemini-2.5-pro", "gemini-3-pro-preview"],
            "gemini-3-pro-preview",
        ),
        (
            // Antigravity also serves Gemini models (https transport).
            "antigravity",
            "https",
            "Antigravity",
            &["gemini-2.5-flash", "gemini-2.5-pro", "gemini-3-pro-preview"],
            "gemini-3-pro-preview",
        ),
    ];

    // Guard: the hand-written cases must cover every ranked provider, or the
    // "for_every_ranked_provider" claim silently rots when a new ranked
    // provider is added without a matching case.
    let covered: std::collections::BTreeSet<&str> =
        cases.iter().map(|(provider_id, ..)| *provider_id).collect();
    let expected_covered: std::collections::BTreeSet<&str> =
        RANKED_PROVIDER_IDS.iter().copied().collect();
    assert_eq!(
        covered, expected_covered,
        "flagship cases drifted from RANKED_PROVIDER_IDS; add a cheap-first case for any \
         newly ranked provider so its flagship selection is actually exercised"
    );

    for (provider_id, api_method, provider_display, models, expected) in cases {
        let activation = activation_for_provider_id(provider_id);
        let routes: Vec<ModelRoute> = models
            .iter()
            .map(|model| route(model, provider_display, api_method, true))
            .collect();
        assert_eq!(
            provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
            Some(*expected),
            "provider `{provider_id}` should auto-select flagship `{expected}` from a \
             cheap-first catalog, not the first route `{}`",
            models[0]
        );
    }
}

/// Copilot proxies both families; the cross-family tie-break must prefer the
/// Claude flagship over the OpenAI flagship to mirror jcode's default model.
#[test]
fn post_auth_model_selection_copilot_prefers_claude_family_over_openai() {
    let activation = activation_for_provider_id("copilot");
    let routes = vec![
        route("gpt-5.5", "Copilot", "copilot", true),
        route("claude-opus-4-8", "Copilot", "copilot", true),
    ];
    assert_eq!(
        provider_model_to_select_after_auth(&activation, None, &routes).as_deref(),
        Some("claude-opus-4-8"),
        "copilot tie-break should prefer the Claude flagship family first"
    );
}

#[test]
fn onboarding_frontier_provider_preference_matrix() {
    use crate::auth::{AuthState, AuthStatus, ProviderAuth};

    let none = AuthStatus::default();
    assert_eq!(preferred_frontier_auth_provider(&none), None);

    let openai_api = AuthStatus {
        openai: AuthState::Available,
        openai_has_api_key: true,
        ..AuthStatus::default()
    };
    assert_eq!(
        preferred_frontier_auth_provider(&openai_api),
        Some("openai-api")
    );

    let anthropic_api = AuthStatus {
        anthropic: ProviderAuth {
            state: AuthState::Available,
            has_api_key: true,
            ..ProviderAuth::default()
        },
        ..AuthStatus::default()
    };
    assert_eq!(
        preferred_frontier_auth_provider(&anthropic_api),
        Some("claude-api")
    );

    let both_oauth = AuthStatus {
        openai: AuthState::Available,
        openai_has_oauth: true,
        openai_oauth_state: AuthState::Available,
        anthropic: ProviderAuth {
            state: AuthState::Available,
            has_oauth: true,
            oauth_state: AuthState::Available,
            ..ProviderAuth::default()
        },
        ..AuthStatus::default()
    };
    assert_eq!(
        preferred_frontier_auth_provider(&both_oauth),
        Some("claude"),
        "Claude is the quality-first default when both frontier providers work"
    );

    let openai_api_and_oauth = AuthStatus {
        openai: AuthState::Available,
        openai_has_oauth: true,
        openai_oauth_state: AuthState::Available,
        openai_has_api_key: true,
        ..AuthStatus::default()
    };
    assert_eq!(
        preferred_frontier_auth_provider(&openai_api_and_oauth),
        Some("openai"),
        "OAuth is preferred over an API key within one provider family"
    );
}

include!("lifecycle_tests/frontier_selection.rs");
