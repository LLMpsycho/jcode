use crate::config::AmbientConfig;
use crate::provider::{
    Provider, RouteSelection, RuntimeKey, configured_role_route, fork_for_agent_role,
    model_route_provider_labels_match,
};
use std::sync::Arc;

/// Select the ambient model and enforce its credential policy before tools,
/// a visible terminal, or a provider request can be started.
pub fn fork_ambient_provider(
    provider: &dyn Provider,
    config: &AmbientConfig,
) -> anyhow::Result<Arc<dyn Provider>> {
    let selected = fork_for_agent_role(
        provider,
        config.route.as_ref(),
        config.model.as_deref(),
        config.effort.as_deref(),
    )?;
    // Ambient inherits its credential restriction even when no model override
    // is configured; request-time failover must not cross into a paid route.
    selected.set_route_pinned(true);
    if !config.allow_api_keys && !uses_subscription_route(selected.as_ref(), config) {
        anyhow::bail!(
            "Ambient API-key usage is disabled (ambient.allow_api_keys=false); choose a signed-in subscription model in /agents ambient or explicitly enable API-key usage in config"
        );
    }
    Ok(selected)
}

fn subscription_runtime(runtime: &RuntimeKey) -> bool {
    matches!(
        runtime,
        RuntimeKey::JcodeSubscription
            | RuntimeKey::ClaudeOAuth
            | RuntimeKey::OpenAIOAuth
            | RuntimeKey::Copilot
            | RuntimeKey::Cursor
            | RuntimeKey::Antigravity
            | RuntimeKey::CodeAssistOAuth
    )
}

fn uses_subscription_route(provider: &dyn Provider, config: &AmbientConfig) -> bool {
    let credential = provider.active_resolved_credential();
    if credential.is_some_and(|credential| !credential.is_subscription()) {
        return false;
    }
    if let Some(route) = config.route.as_ref() {
        return subscription_runtime(&configured_role_route(route).runtime_key);
    }
    if credential.is_some_and(|credential| credential.is_subscription()) {
        return true;
    }
    if provider.direct_openai_compatible_route_parts().is_some() {
        return false;
    }

    // Providers without a dual-auth credential answer still expose canonical
    // route metadata. Inheritance is permitted only when every matching active
    // catalog route is a known subscription runtime; ambiguous/unknown auth
    // fails closed instead of guessing from the provider's name.
    let model = provider.model();
    let display_name = provider.display_name();
    let mut routes = provider
        .model_routes()
        .into_iter()
        .filter(|route| {
            route.available
                && route.model == model
                && model_route_provider_labels_match(&route.provider, &display_name)
        })
        .peekable();
    routes.peek().is_some()
        && routes.all(|route| {
            subscription_runtime(&RouteSelection::from_model_route(&route).runtime_key)
        })
}
