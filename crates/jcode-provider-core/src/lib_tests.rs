use super::*;

#[test]
fn metered_estimate_computes_reference_cost() {
    let estimate = RouteCheapnessEstimate::metered(
        RouteCostSource::Heuristic,
        RouteCostConfidence::Low,
        2_000_000,
        8_000_000,
        None,
        None,
    );
    assert_eq!(estimate.estimated_reference_cost_micros, Some(90_000));
}

#[test]
fn shared_http_client_reuses_builder() {
    let _a = shared_http_client();
    let _b = shared_http_client();
}

#[test]
fn fresh_transport_client_builds_distinct_clients() {
    // Each call must produce a brand-new client (new connection pool), not
    // a cached one: the whole point is that a retry after a transport
    // fault (e.g. TLS BadRecordMac) never reuses a possibly-poisoned
    // pooled connection.
    let _a = fresh_transport_client();
    let _b = fresh_transport_client();
}

#[test]
fn canonical_user_agent_identifies_jcode() {
    assert!(JCODE_USER_AGENT.starts_with("jcode/"));
}

#[test]
fn model_route_api_method_parser_keeps_profile_identity() {
    assert_eq!(
        ModelRouteApiMethod::parse("openai-compatible:cerebras"),
        ModelRouteApiMethod::OpenAiCompatible {
            profile_id: Some("cerebras".to_string())
        }
    );
    assert!(
        ModelRouteApiMethod::parse("openai-compatible:cerebras")
            .matches_openai_compatible_profile("CEREBRAS")
    );
    assert_eq!(
        ModelRouteApiMethod::parse("openai-api"),
        ModelRouteApiMethod::OpenAIApiKey
    );
    assert_eq!(
        ModelRouteApiMethod::parse("claude-api"),
        ModelRouteApiMethod::AnthropicApiKey
    );
}

#[test]
fn model_route_provider_label_matching_uses_aliases_without_substring_false_positives() {
    assert!(model_route_provider_labels_match("Anthropic", "Claude"));
    assert!(model_route_provider_labels_match("auto", "OpenRouter"));
    assert!(model_route_provider_labels_match(
        "GitHub Copilot",
        "Copilot"
    ));
    assert!(model_route_provider_labels_match("AWS Bedrock", "Bedrock"));
    assert!(!model_route_provider_labels_match(
        "OpenRouter/OpenAI",
        "OpenAI"
    ));
    assert!(!model_route_provider_labels_match("OpenAI", "OpenRouter"));
    assert!(!model_route_provider_labels_match("", ""));
    assert!(!model_route_provider_labels_related("OpenAI", ""));
}

#[test]
fn model_route_provider_key_matching_prefers_explicit_route_key() {
    assert!(model_route_provider_matches_key(
        Some("cerebras"),
        "Cerebras Cloud",
        "CEREBRAS"
    ));
    assert!(model_route_provider_matches_key(
        None,
        "Anthropic",
        "Claude"
    ));
    assert!(!model_route_provider_matches_key(
        Some("cerebras"),
        "Cerebras",
        "groq"
    ));
}

#[test]
fn model_route_provider_key_matching_folds_dual_auth_vocabularies() {
    // `default_provider = "anthropic-api"` and a route keyed `claude-api`
    // are two spellings of the same Anthropic API-key route, so they must
    // match even though their raw forms normalize differently.
    assert!(model_route_provider_matches_key(
        Some("claude-api"),
        "Anthropic",
        "anthropic-api",
    ));
    assert!(model_route_provider_matches_key(
        Some("anthropic-api-key"),
        "Anthropic",
        "claude-api",
    ));
    assert!(model_route_provider_matches_key(
        Some("openai-api"),
        "OpenAI",
        "openai-api-key",
    ));

    // The fold must NOT collapse the OAuth-vs-API distinction: an API-key
    // default must not light up the OAuth route (and vice versa).
    assert!(!model_route_provider_matches_key(
        Some("claude-oauth"),
        "Anthropic",
        "anthropic-api",
    ));
    assert!(!model_route_provider_matches_key(
        Some("openai-oauth"),
        "OpenAI",
        "openai-api",
    ));

    // A bare provider default pins no credential, so it keeps the historical
    // auth-method-agnostic behavior: it matches either dual-auth route via
    // the label fallback (model identity still narrows the picker default).
    assert!(model_route_provider_matches_key(
        Some("claude-oauth"),
        "Anthropic",
        "claude",
    ));
    assert!(model_route_provider_matches_key(
        Some("claude-api"),
        "Anthropic",
        "claude",
    ));
}

#[test]
fn model_route_recommendation_policy_is_provider_aware() {
    assert!(model_route_metadata_is_recommended(
        "gpt-5.5",
        "OpenAI",
        "openai-oauth",
        true
    ));
    assert!(!model_route_metadata_is_recommended(
        "gpt-5.5",
        "OpenAI",
        "openai-api-key",
        true
    ));
    assert!(!model_route_metadata_is_recommended(
        "gpt-5.5", "Copilot", "copilot", true
    ));
    assert!(!model_route_metadata_is_recommended(
        "gpt-5.5",
        "OpenAI",
        "openai-oauth",
        false
    ));
    assert!(model_route_metadata_is_recommended(
        "claude-opus-4-8",
        "Anthropic",
        "claude-oauth",
        true
    ));
    assert!(model_route_metadata_is_recommended(
        "claude-opus-4-8",
        "Anthropic",
        "claude-api",
        true
    ));
    assert!(model_route_metadata_is_recommended(
        "claude-opus-4-8",
        "Anthropic",
        "claude-oauth",
        true
    ));
    assert!(model_route_metadata_is_recommended(
        "claude-opus-4-8",
        "Anthropic",
        "claude-api",
        true
    ));
    assert!(!model_route_metadata_is_recommended(
        "claude-opus-4-8",
        "Anthropic",
        "openrouter",
        true
    ));
    assert!(!model_route_metadata_is_recommended(
        "deepseek/deepseek-v4-pro",
        "auto",
        "openrouter",
        true
    ));
}

struct SnapshotTestProvider;

#[async_trait]
impl Provider for SnapshotTestProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        unreachable!("snapshot test does not call complete")
    }

    fn name(&self) -> &str {
        "snapshot-provider"
    }

    fn model(&self) -> String {
        "snapshot-model".to_string()
    }

    fn available_models_display(&self) -> Vec<String> {
        vec!["snapshot-model".to_string()]
    }

    fn model_routes(&self) -> Vec<ModelRoute> {
        vec![ModelRoute {
            model: "snapshot-model".to_string(),
            provider: "Snapshot".to_string(),
            api_method: "snapshot-api".to_string(),
            available: true,
            detail: "test route".to_string(),
            cheapness: None,
        }]
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(SnapshotTestProvider)
    }
}

#[test]
fn model_catalog_snapshot_materializes_provider_catalog_contract() {
    let snapshot = ModelCatalogSnapshot::from_provider(&SnapshotTestProvider);

    assert_eq!(snapshot.provider_name.as_deref(), Some("snapshot-provider"));
    assert_eq!(snapshot.provider_model.as_deref(), Some("snapshot-model"));
    assert_eq!(snapshot.available_models, ["snapshot-model"]);
    assert!(snapshot.has_routes());
    assert_eq!(snapshot.model_routes[0].api_method, "snapshot-api");
}

#[test]
fn runtime_key_distinguishes_openrouter_from_direct_compatible_profile() {
    assert_eq!(
        RuntimeKey::from_api_method(&ModelRouteApiMethod::parse("openrouter"), "auto"),
        RuntimeKey::OpenRouter
    );
    assert_eq!(
        RuntimeKey::from_api_method(
            &ModelRouteApiMethod::parse("openai-compatible:nvidia-nim"),
            "NVIDIA NIM",
        ),
        RuntimeKey::OpenAiCompatible {
            profile_id: Some("nvidia-nim".to_string())
        }
    );
}

#[test]
fn route_selection_preserves_runtime_identity_from_model_route() {
    let selection = RouteSelection::from_model_route(&ModelRoute {
        model: "openrouter/owl-alpha".to_string(),
        provider: "OpenRouter".to_string(),
        api_method: "openrouter".to_string(),
        available: true,
        detail: "https://openrouter.ai/api/v1".to_string(),
        cheapness: None,
    });
    assert_eq!(selection.model, "openrouter/owl-alpha");
    assert_eq!(selection.runtime_key, RuntimeKey::OpenRouter);
    assert_eq!(selection.api_method, "openrouter");

    let selection = RouteSelection::from_model_route(&ModelRoute {
        model: "nvidia/example".to_string(),
        provider: "NVIDIA NIM".to_string(),
        api_method: "openai-compatible:nvidia-nim".to_string(),
        available: true,
        detail: "https://integrate.api.nvidia.com/v1".to_string(),
        cheapness: None,
    });
    assert_eq!(
        selection.runtime_key,
        RuntimeKey::OpenAiCompatible {
            profile_id: Some("nvidia-nim".to_string())
        }
    );
    assert_eq!(selection.provider_label, "NVIDIA NIM");
}
