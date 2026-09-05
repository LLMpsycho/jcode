use super::*;
use crate::provider::{ModelRoute, RouteSelection, RuntimeKey, model_route_provider_labels_match};
use anyhow::{Result, bail};

pub(super) fn role_request(config: &AdvisorConfig) -> Option<&str> {
    config.model.as_deref().or(match config.mode {
        AdvisorMode::FinalReview => config.verification_model.as_deref(),
        AdvisorMode::Interactive | AdvisorMode::SelfdevGuardian => config.reviewer_model.as_deref(),
    })
}

pub(super) fn permitted(route: &ModelRoute, config: &AdvisorConfig) -> bool {
    route.available
        && config.allowed_runtime_keys.as_ref().is_none_or(|keys| {
            keys.contains(
                &RouteSelection::from_model_route(route)
                    .runtime_key
                    .stable_id(),
            )
        })
}

pub(super) fn current_runtime(route: &ModelRoute, provider: &dyn Provider) -> bool {
    if let Some((label, api, _)) = provider.direct_openai_compatible_route_parts() {
        return route.provider == label && route.api_method == api;
    }
    if !model_route_provider_labels_match(&route.provider, &provider.display_name()) {
        return false;
    }
    let runtime = RouteSelection::from_model_route(route).runtime_key;
    match (runtime, provider.active_resolved_credential()) {
        (RuntimeKey::ClaudeOAuth | RuntimeKey::OpenAIOAuth, Some(auth)) => auth.is_subscription(),
        (RuntimeKey::AnthropicApiKey | RuntimeKey::OpenAIApiKey, Some(auth)) => {
            !auth.is_subscription()
        }
        _ => true,
    }
}

pub(super) fn canonical_selection(
    provider: &dyn Provider,
    config: &AdvisorConfig,
    selection: &RouteSelection,
) -> Result<RouteSelection> {
    let matching = provider.model_routes().into_iter().find(|route| {
        let candidate = RouteSelection::from_model_route(route);
        permitted(route, config)
            && candidate.model == selection.model
            && candidate.runtime_key == selection.runtime_key
            && candidate.api_method == selection.api_method
            && candidate.provider_label == selection.provider_label
    });
    let Some(route) = matching else {
        bail!("advisor model route is unavailable or is not permitted; refresh /advisor models");
    };
    let mut canonical = RouteSelection::from_model_route(&route);
    // Catalog detail can contain endpoint configuration; it is presentation
    // metadata and is never needed to persist or execute an exact route.
    canonical.detail.clear();
    validate_persisted_selection(&canonical)?;
    Ok(canonical)
}

pub(super) fn validate_persisted_selection(selection: &RouteSelection) -> Result<()> {
    let runtime = selection.runtime_key.stable_id();
    let fields = [
        selection.model.as_str(),
        selection.api_method.as_str(),
        selection.provider_label.as_str(),
        runtime.as_str(),
    ];
    if fields.iter().any(|value| {
        value.len() > 256
            || value.chars().any(char::is_control)
            || redact_secrets(value) != *value
    }) || !selection.detail.is_empty()
    {
        bail!("advisor model route metadata is invalid");
    }
    Ok(())
}

pub(super) fn efforts(provider: &dyn Provider) -> Vec<String> {
    provider
        .available_efforts()
        .into_iter()
        .filter(|effort| !matches!(*effort, "swarm" | "swarm-deep"))
        .map(str::to_string)
        .collect()
}

pub(super) fn apply_override(
    provider: &dyn Provider,
    config: &AdvisorConfig,
    selection: Option<&model_selection::AdvisorModelOverride>,
) -> Result<()> {
    match selection {
        Some(model_selection::AdvisorModelOverride::Selected {
            selection,
            reasoning_effort,
        }) => {
            let canonical = canonical_selection(provider, config, selection)?;
            provider.set_route_selection(&canonical)?;
            if let Some(effort) = reasoning_effort {
                if !efforts(provider).contains(effort) {
                    bail!("reasoning effort is not supported by the selected advisor model");
                }
                provider.set_reasoning_effort(effort)?;
            }
            Ok(())
        }
        Some(model_selection::AdvisorModelOverride::Primary) => {
            let mut inherited = config.clone();
            inherited.model = None;
            inherited.reviewer_model = None;
            inherited.verification_model = None;
            apply(provider, &inherited)
        }
        None => apply(provider, config),
    }?;
    if provider.reasoning_effort().as_deref().is_some_and(|effort| matches!(effort, "swarm" | "swarm-deep")) {
        bail!("advisor requires a single-model reasoning effort; choose an effort in /advisor");
    }
    Ok(())
}

/// Resolve against the provider's authenticated route catalog, then use the
/// existing structured selection API on the private fork. No primary prefix,
/// credential pin, or default provider is mutated. Failed roles never fall back.
pub(super) fn apply(provider: &dyn Provider, config: &AdvisorConfig) -> Result<()> {
    let request = role_request(config);
    if request.is_none() && config.allowed_runtime_keys.is_none() {
        return Ok(()); // Inherit the already-selected primary route unchanged.
    }
    let current_model = provider.model();
    let requested = request.unwrap_or(&current_model).trim();
    if requested.is_empty() {
        bail!("empty advisor model route");
    }
    let mut routes: Vec<_> = provider
        .model_routes()
        .into_iter()
        .filter(|route| {
            let selection = RouteSelection::from_model_route(route);
            permitted(route, config)
                && (selection.routed_model_spec() == requested || route.model == requested)
                && (request.is_some() || current_runtime(route, provider))
        })
        .collect();
    // A bare model can have several credential routes. Preserve the current
    // runtime when possible; otherwise require an explicit canonical route.
    if routes.len() > 1 {
        let current: Vec<_> = routes
            .iter()
            .filter(|route| current_runtime(route, provider))
            .cloned()
            .collect();
        if current.len() == 1 {
            routes = current;
        }
    }
    if routes.len() != 1 {
        bail!(
            "advisor role has no unique permitted, available route; select a canonical /model route and check allowed_runtime_keys"
        );
    }
    if request.is_some() {
        provider.set_route_selection(&RouteSelection::from_model_route(&routes[0]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToolDefinition;
    use crate::provider::EventStream;
    use async_trait::async_trait;

    struct CatalogProvider {
        routes: Vec<ModelRoute>,
        selected: Mutex<Option<RouteSelection>>,
    }

    #[async_trait]
    impl Provider for CatalogProvider {
        fn name(&self) -> &str {
            "openai"
        }
        fn model(&self) -> String {
            "coder".into()
        }
        fn model_routes(&self) -> Vec<ModelRoute> {
            self.routes.clone()
        }
        fn set_route_selection(&self, route: &RouteSelection) -> Result<()> {
            *self.selected.lock().expect("selection") = Some(route.clone());
            Ok(())
        }
        async fn complete(
            &self,
            _: &[Message],
            _: &[ToolDefinition],
            _: &str,
            _: Option<&str>,
        ) -> Result<EventStream> {
            bail!("routing must not send evidence")
        }
        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self {
                routes: self.routes.clone(),
                selected: Mutex::new(None),
            })
        }
    }

    fn provider() -> CatalogProvider {
        CatalogProvider {
            routes: vec![
                ModelRoute {
                    model: "reviewer".into(),
                    provider: "OpenAI".into(),
                    api_method: "openai-api".into(),
                    available: true,
                    detail: String::new(),
                    cheapness: None,
                },
                ModelRoute {
                    model: "verifier".into(),
                    provider: "OpenAI".into(),
                    api_method: "openai-api".into(),
                    available: true,
                    detail: String::new(),
                    cheapness: None,
                },
            ],
            selected: Mutex::new(None),
        }
    }

    #[test]
    fn mode_selects_its_role_only_on_the_fork() {
        let primary = provider();
        let fork = primary.fork();
        let config = AdvisorConfig {
            reviewer_model: Some("reviewer".into()),
            verification_model: Some("verifier".into()),
            ..AdvisorConfig::default()
        };
        apply(fork.as_ref(), &config).expect("reviewer");
        assert!(primary.selected.lock().expect("selection").is_none());
        apply(
            &primary,
            &AdvisorConfig {
                mode: AdvisorMode::FinalReview,
                ..config
            },
        )
        .expect("verifier");
        assert_eq!(
            primary
                .selected
                .lock()
                .expect("selection")
                .as_ref()
                .expect("route")
                .model,
            "verifier"
        );
    }

    #[test]
    fn denied_unavailable_or_unknown_roles_fail_before_selection() {
        let mut primary = provider();
        let config = AdvisorConfig {
            model: Some("reviewer".into()),
            allowed_runtime_keys: Some(vec![]),
            ..AdvisorConfig::default()
        };
        assert!(apply(&primary, &config).is_err());
        assert!(primary.selected.lock().expect("selection").is_none());
        primary.routes[0].available = false;
        assert!(
            apply(
                &primary,
                &AdvisorConfig {
                    allowed_runtime_keys: None,
                    ..config
                }
            )
            .is_err()
        );
        let config = AdvisorConfig {
            model: Some("not-in-catalog".into()),
            ..AdvisorConfig::default()
        };
        assert!(apply(&primary, &config).is_err());
    }

    #[test]
    fn advisor_permission_keys_accept_only_the_exact_canonical_runtime() {
        let primary = provider();
        let config = AdvisorConfig {
            model: Some("openai-api:reviewer".into()),
            allowed_runtime_keys: Some(vec!["openai-api-key".into()]),
            ..AdvisorConfig::default()
        };
        apply(&primary, &config).expect("explicit permitted API-key route");
        assert_eq!(
            primary
                .selected
                .lock()
                .expect("selection")
                .as_ref()
                .expect("route")
                .runtime_key,
            RuntimeKey::OpenAIApiKey
        );
        let denied = AdvisorConfig {
            allowed_runtime_keys: Some(vec!["openai-oauth".into()]),
            ..config
        };
        assert!(apply(&primary, &denied).is_err());
    }

    #[test]
    fn advisor_ambiguous_bare_model_requires_a_canonical_route() {
        let mut primary = provider();
        primary.routes[0].provider = "Anthropic".into();
        primary.routes[0].api_method = "anthropic-api".into();
        primary.routes[1] = primary.routes[0].clone();
        primary.routes[1].api_method = "claude".into();
        let config = AdvisorConfig {
            reviewer_model: Some("reviewer".into()),
            ..AdvisorConfig::default()
        };
        assert!(apply(&primary, &config).is_err());
        assert!(primary.selected.lock().expect("selection").is_none());
    }
}
