use super::{ModelRouteApiMethod, Provider, RouteSelection, RuntimeKey};
use crate::config::ConfigModelRoute;
use anyhow::{Context, Result, bail};
use std::sync::Arc;

/// Reconstruct executable route identity from credential-free role settings.
pub fn configured_role_route(route: &ConfigModelRoute) -> RouteSelection {
    RouteSelection {
        model: route.model.clone(),
        runtime_key: RuntimeKey::from_api_method(
            &ModelRouteApiMethod::parse(&route.api_method),
            &route.provider_label,
        ),
        api_method: route.api_method.clone(),
        provider_label: route.provider_label.clone(),
        detail: String::new(),
    }
}

/// Configure a private provider for an agent role without changing the main
/// session. Explicit selections fail visibly if their route or effort is no
/// longer available; they never fall back to another account or model.
pub fn fork_for_agent_role(
    provider: &dyn Provider,
    route: Option<&ConfigModelRoute>,
    legacy_model: Option<&str>,
    effort: Option<&str>,
) -> Result<Arc<dyn Provider>> {
    let selection = route.map(configured_role_route);
    if let Some(selection) = selection.as_ref()
        && !provider.model_routes().iter().any(|candidate| {
            candidate.available
                && candidate.model == selection.model
                && candidate.api_method == selection.api_method
                && candidate.provider == selection.provider_label
        })
    {
        bail!("Selected agent model route is unavailable; choose an available model in /agents");
    }

    let fork = provider.fork();
    if let Some(selection) = selection.as_ref() {
        fork.set_route_selection(selection)
            .context("Could not select the configured agent model route")?;
    } else if let Some(model) = legacy_model.map(str::trim).filter(|model| {
        !model.is_empty()
            && !model.eq_ignore_ascii_case("inherit")
            && !model.eq_ignore_ascii_case("coordinator")
    }) {
        if fork.model() != model {
            fork.set_model(model)
                .context("Could not select the configured agent model")?;
        }
    }

    if let Some(effort) = effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        if matches!(effort, "swarm" | "swarm-deep") || !fork.available_efforts().contains(&effort) {
            bail!(
                "Selected reasoning effort is unavailable for the agent model; choose its effort again in /agents"
            );
        }
        fork.set_reasoning_effort(effort)
            .context("Could not apply the configured agent reasoning effort")?;
    }
    if route.is_some()
        || legacy_model.is_some_and(|model| {
            !model.trim().is_empty()
                && !model.eq_ignore_ascii_case("inherit")
                && !model.eq_ignore_ascii_case("coordinator")
        })
    {
        fork.set_route_pinned(true);
    }
    Ok(fork)
}

#[cfg(test)]
#[path = "tests/agent_roles.rs"]
mod tests;
