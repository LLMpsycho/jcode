//! Model selection.

use super::*;

pub(super) fn provider_key_for_spawn_model(
    model: Option<&str>,
    provider_key_override: Option<&str>,
) -> Option<String> {
    if let Some(provider_key) = provider_key_override
        .map(str::trim)
        .filter(|provider_key| !provider_key.is_empty())
    {
        return Some(provider_key.to_string());
    }

    let model = model?.trim();
    if model.is_empty() {
        return None;
    }

    if let Some((prefix, _rest)) = model.split_once(':') {
        let prefix = prefix.trim();
        if crate::provider::provider_from_model_key(prefix).is_some()
            || crate::provider_catalog::resolve_openai_compatible_profile_selection(prefix)
                .is_some()
            || crate::config::config().providers.contains_key(prefix)
        {
            return Some(prefix.to_string());
        }
    }

    crate::provider::provider_for_model(model).map(str::to_string)
}

/// Split a configured swarm model that carries an explicit auth-route prefix
/// (`openai-api:`, `openai-oauth:`, `claude-api:`, `claude-oauth:`) into a
/// structured selection so spawned sessions pin the exact provider + auth
/// method instead of guessing from the bare model name.
///
/// Example: `agents.swarm_model = "openai-api:gpt-5.5"` resolves to
/// `model = gpt-5.5`, `provider_key = openai-api-key`,
/// `route_api_method = openai-api-key`, which makes every spawned agent use
/// GPT-5.5 on the OpenAI API key route regardless of the coordinator's model.
///
/// Returns `None` for models without such a prefix, or for prefixes that carry
/// no API-vs-OAuth decision (bare provider aliases, OpenRouter, Copilot, ...).
/// Those keep their prefixed model and route correctly via the existing
/// session-restore path.
pub(super) fn explicit_route_for_configured_model(model: &str) -> Option<SwarmSpawnSelection> {
    let (_, prefix, bare) = crate::provider::explicit_model_provider_prefix(model)?;
    let bare = bare.trim();
    if bare.is_empty() {
        return None;
    }
    // Only the dual-auth (Anthropic/OpenAI OAuth-vs-API) prefixes carry an
    // explicit credential decision worth pinning. The canonical parser maps the
    // prefix to its stable route id, which `ModelRouteApiMethod::parse` round-
    // trips back to the exact auth method when the spawned session is restored.
    let route_id = jcode_provider_core::AuthRoute::parse_explicit_credential_prefix(prefix)?
        .route_api_method();
    Some(SwarmSpawnSelection {
        model: Some(bare.to_string()),
        provider_key: Some(route_id.to_string()),
        route_api_method: Some(route_id.to_string()),
    })
}

/// True when a model string is one of the "inherit the coordinator" sentinels.
pub(super) fn is_inherit_sentinel(model: &str) -> bool {
    let trimmed = model.trim();
    trimmed.eq_ignore_ascii_case("inherit") || trimmed.eq_ignore_ascii_case("coordinator")
}

/// Selection that inherits the coordinator's model, provider key, and route.
pub(super) fn inherit_coordinator_selection(
    coordinator: &CoordinatorSpawnIdentity,
) -> SwarmSpawnSelection {
    SwarmSpawnSelection {
        model: coordinator.model.clone(),
        provider_key: coordinator
            .provider_key
            .clone()
            .or_else(|| provider_key_for_spawn_model(coordinator.model.as_deref(), None)),
        route_api_method: coordinator.route_api_method.clone(),
    }
}

/// Selection for a concrete model string (optionally route-prefixed like
/// `openai-api:gpt-5.5`), reconciled against the coordinator's identity.
pub(super) fn selection_for_concrete_model(
    model: String,
    coordinator: &CoordinatorSpawnIdentity,
) -> SwarmSpawnSelection {
    // A model may pin an explicit provider + auth route via a prefix
    // (e.g. "openai-api:gpt-5.5"). Honor it directly so spawned agents do
    // NOT inherit the coordinator's model/auth and instead use the
    // requested model on the requested API route.
    if let Some(selection) = explicit_route_for_configured_model(&model) {
        return selection;
    }

    // A concrete model only inherits the coordinator's provider_key/route
    // when it targets the same model; otherwise the route would point at
    // the wrong provider/auth mode.
    if coordinator.model.as_deref() == Some(model.as_str()) {
        SwarmSpawnSelection {
            model: Some(model.clone()),
            provider_key: coordinator
                .provider_key
                .clone()
                .or_else(|| provider_key_for_spawn_model(Some(&model), None)),
            route_api_method: coordinator.route_api_method.clone(),
        }
    } else {
        SwarmSpawnSelection {
            provider_key: provider_key_for_spawn_model(Some(&model), None),
            model: Some(model),
            route_api_method: None,
        }
    }
}

pub(super) fn resolve_swarm_spawn_selection(
    requested_model: Option<String>,
    configured_swarm_model: Option<String>,
    coordinator: &CoordinatorSpawnIdentity,
) -> SwarmSpawnSelection {
    // An explicit per-worker choice overrides the configured default. The
    // inheritance sentinels bypass even a concrete configured model.
    if let Some(model) = requested_model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
    {
        return if is_inherit_sentinel(&model) {
            inherit_coordinator_selection(coordinator)
        } else {
            selection_for_concrete_model(model, coordinator)
        };
    }
    // Treat empty strings and the explicit "inherit"/"coordinator" sentinels as
    // "no override": spawned swarm agents should inherit the coordinator's model
    // unless `agents.swarm_model` is deliberately set to a concrete model. This
    // avoids the surprising case where a stale `swarm_model` config pins every
    // spawned agent to an unrelated model/provider.
    let configured_swarm_model = configured_swarm_model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.trim().is_empty() && !is_inherit_sentinel(model));

    match configured_swarm_model {
        Some(model) => selection_for_concrete_model(model, coordinator),
        None => inherit_coordinator_selection(coordinator),
    }
}
