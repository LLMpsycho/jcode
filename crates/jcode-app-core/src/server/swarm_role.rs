use super::{
    CoordinatorSpawnIdentity, SwarmSpawnSelection, is_inherit_sentinel, resolve_swarm_spawn_effort,
    resolve_swarm_spawn_selection,
};
use crate::config::AgentsConfig;
use crate::provider::{RouteSelection, RuntimeKey, configured_role_route};

pub(in crate::server) struct ResolvedSwarmRole {
    pub selection: SwarmSpawnSelection,
    pub route: Option<RouteSelection>,
    pub effort: Option<String>,
}

pub(in crate::server) fn resolve(
    config: &AgentsConfig,
    coordinator: &CoordinatorSpawnIdentity,
    requested_effort: Option<&str>,
) -> ResolvedSwarmRole {
    resolve_with_model(config, coordinator, None, requested_effort)
}

pub(in crate::server) fn resolve_with_model(
    config: &AgentsConfig,
    coordinator: &CoordinatorSpawnIdentity,
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
) -> ResolvedSwarmRole {
    let requested_model = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    let session_model = coordinator
        .subagent_model
        .as_ref()
        .filter(|model| !model.trim().is_empty() && !is_inherit_sentinel(model));
    let override_model = requested_model.or_else(|| session_model.map(String::as_str));
    let route = override_model
        .is_none()
        .then_some(config.swarm_route.as_ref())
        .flatten()
        .map(configured_role_route);
    let selection = if let Some(route) = route.as_ref() {
        SwarmSpawnSelection {
            model: Some(if route.runtime_key == RuntimeKey::OpenRouter {
                route.routed_model_spec()
            } else {
                route.model.clone()
            }),
            provider_key: Some(route.runtime_key.stable_id()),
            route_api_method: Some(route.api_method.clone()),
        }
    } else {
        resolve_swarm_spawn_selection(
            override_model.map(str::to_string),
            config.swarm_model.clone(),
            coordinator,
        )
    };
    let inherits_model = override_model.map(is_inherit_sentinel).unwrap_or_else(|| {
        route.is_none()
            && config
                .swarm_model
                .as_ref()
                .is_none_or(|model| model.trim().is_empty() || is_inherit_sentinel(model))
    });
    // A per-worker or per-session model must not pick up effort for a different
    // configured model. Explicit inheritance retains the coordinator's effort.
    let configured_effort = override_model
        .is_none()
        .then_some(config.swarm_effort.as_deref())
        .flatten();
    let effort = resolve_swarm_spawn_effort(requested_effort, configured_effort).or_else(|| {
        inherits_model
            .then(|| coordinator.reasoning_effort.clone())
            .flatten()
    });
    ResolvedSwarmRole {
        selection,
        route,
        effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigModelRoute;

    fn coordinator() -> CoordinatorSpawnIdentity {
        CoordinatorSpawnIdentity {
            model: Some("gpt-5.5".into()),
            provider_key: Some("openai-oauth".into()),
            route_api_method: Some("openai-oauth".into()),
            reasoning_effort: Some("high".into()),
            ..Default::default()
        }
    }

    #[test]
    fn swarm_role_keeps_exact_route_pin_and_effort() {
        let mut config = AgentsConfig::default();
        config.swarm_route = Some(ConfigModelRoute {
            model: "claude-opus-4-6".into(),
            api_method: "openrouter".into(),
            provider_label: "Anthropic".into(),
        });
        config.swarm_effort = Some("low".into());
        let resolved = resolve(&config, &coordinator(), None);
        assert_eq!(
            resolved.selection.model.as_deref(),
            Some("anthropic/claude-opus-4-6@Anthropic")
        );
        assert_eq!(resolved.route.unwrap().runtime_key, RuntimeKey::OpenRouter);
        assert_eq!(
            resolved.selection.route_api_method.as_deref(),
            Some("openrouter")
        );
        assert_eq!(resolved.effort.as_deref(), Some("low"));
        assert_eq!(
            resolve(&config, &coordinator(), Some("high"))
                .effort
                .as_deref(),
            Some("high")
        );
    }

    #[test]
    fn swarm_session_model_pin_precedes_global_role_without_wrong_effort() {
        let mut config = AgentsConfig::default();
        config.swarm_route = Some(ConfigModelRoute {
            model: "gpt-5.5".into(),
            api_method: "openai-oauth".into(),
            provider_label: "OpenAI".into(),
        });
        config.swarm_effort = Some("high".into());
        let mut coordinator = coordinator();
        coordinator.subagent_model = Some("claude-oauth:claude-opus-4-6".into());
        let resolved = resolve(&config, &coordinator, None);
        assert!(resolved.route.is_none());
        assert_eq!(resolved.selection.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(
            resolved.selection.route_api_method.as_deref(),
            Some("claude-oauth")
        );
        assert!(resolved.effort.is_none());
    }

    #[test]
    fn swarm_inherit_preserves_coordinator_model_auth_and_effort() {
        let resolved = resolve(&AgentsConfig::default(), &coordinator(), None);
        assert_eq!(resolved.selection.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            resolved.selection.route_api_method.as_deref(),
            Some("openai-oauth")
        );
        assert_eq!(resolved.effort.as_deref(), Some("high"));
    }
    #[test]
    fn explicit_worker_choice_overrides_global_route_and_session_pin() {
        let mut config = AgentsConfig::default();
        config.swarm_route = Some(ConfigModelRoute {
            model: "claude-opus-4-6".into(),
            api_method: "claude-oauth".into(),
            provider_label: "Anthropic".into(),
        });
        config.swarm_effort = Some("max".into());
        let mut parent = coordinator();
        parent.subagent_model = Some("claude-oauth:claude-opus-4-6".into());
        let chosen = resolve_with_model(&config, &parent, Some("openai-api:gpt-5.5"), Some("low"));
        assert!(chosen.route.is_none());
        assert_eq!(
            chosen.selection.route_api_method.as_deref(),
            Some("openai-api-key")
        );
        assert_eq!(chosen.effort.as_deref(), Some("low"));
        let inherit = resolve_with_model(&config, &parent, Some("inherit"), None);
        assert!(inherit.route.is_none());
        assert_eq!(
            inherit.selection.route_api_method.as_deref(),
            Some("openai-oauth")
        );
        assert_eq!(inherit.effort.as_deref(), Some("high"));
        let fresh = resolve_with_model(&config, &parent, Some("openai-api:gpt-5.5"), None);
        assert!(
            fresh.effort.is_none(),
            "do not leak configured effort onto another route"
        );
    }
}
