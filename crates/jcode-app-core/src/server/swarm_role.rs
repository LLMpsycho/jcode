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
    let session_model = coordinator
        .subagent_model
        .as_ref()
        .filter(|model| !model.trim().is_empty() && !is_inherit_sentinel(model));
    let configured_route = session_model
        .is_none()
        .then_some(config.swarm_route.as_ref())
        .flatten();
    let route = configured_route.map(configured_role_route);
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
            session_model
                .cloned()
                .or_else(|| config.swarm_model.clone()),
            coordinator,
        )
    };
    let inherits_model = session_model.is_none()
        && route.is_none()
        && config
            .swarm_model
            .as_ref()
            .is_none_or(|model| model.trim().is_empty() || is_inherit_sentinel(model));
    let configured_effort = session_model
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
}
