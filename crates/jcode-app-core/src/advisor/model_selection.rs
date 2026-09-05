use super::*;
use crate::protocol::{AdvisorModelOptions, AdvisorModelSettings};
use crate::provider::RouteSelection;
use anyhow::{Result, bail};

/// Session-owned route identity only. Credentials, endpoints, and provider
/// context remain in the existing authenticated provider implementations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum AdvisorModelOverride {
    Primary,
    Selected {
        selection: RouteSelection,
        reasoning_effort: Option<String>,
    },
}

impl AdvisorManager {
    pub fn saved_model_settings(
        &self,
        session: &str,
        config: &AdvisorConfig,
    ) -> AdvisorModelSettings {
        let selected = self.model_override(session);
        let follows_primary = matches!(selected, Some(AdvisorModelOverride::Primary))
            || (selected.is_none() && routing::role_request(config).is_none());
        let (selection, reasoning_effort) = match selected {
            Some(AdvisorModelOverride::Selected {
                selection,
                reasoning_effort,
            }) => (Some(selection), reasoning_effort),
            _ => (None, None),
        };
        AdvisorModelSettings {
            enabled: self.is_enabled(session, config.enabled),
            selection,
            reasoning_effort,
            follows_primary,
        }
    }

    pub fn model_summary(&self, session: &str, config: &AdvisorConfig) -> String {
        match self.model_override(session) {
            Some(AdvisorModelOverride::Selected {
                selection,
                reasoning_effort,
            }) => format!(
                "model {} via {} ({}); effort {}",
                selection.model,
                selection.provider_label,
                selection.api_method,
                reasoning_effort.as_deref().unwrap_or("provider default")
            ),
            Some(AdvisorModelOverride::Primary) => "model and effort follow primary".into(),
            None => routing::role_request(config)
                .map(|model| {
                    format!(
                        "configured model {}",
                        truncate_utf8(redact_secrets(model), 256)
                    )
                })
                .unwrap_or_else(|| "model and effort follow primary".into()),
        }
    }

    pub fn begin_model_selection(&self, session: &str) -> u64 {
        let id = self
            .next_review_id
            .fetch_add(1, AtomicOrdering::Relaxed)
            .saturating_add(1);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions
                .entry(session.to_string())
                .or_default()
                .model_selection_id = id;
        }
        id
    }

    fn model_override(&self, session: &str) -> Option<AdvisorModelOverride> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(session)?.model_override.clone())
    }

    pub fn model_settings(
        &self,
        session: &str,
        provider: &dyn Provider,
        config: &AdvisorConfig,
    ) -> AdvisorModelSettings {
        let selected = self.model_override(session);
        let follows_primary = matches!(selected, Some(AdvisorModelOverride::Primary))
            || (selected.is_none() && routing::role_request(config).is_none());
        let enabled = self.is_enabled(session, config.enabled);
        if let Some(AdvisorModelOverride::Selected {
            selection,
            reasoning_effort,
        }) = selected.as_ref()
        {
            return AdvisorModelSettings {
                enabled,
                selection: Some(selection.clone()),
                reasoning_effort: reasoning_effort.clone(),
                follows_primary: false,
            };
        }
        if follows_primary {
            return AdvisorModelSettings {
                enabled,
                selection: current_selection(provider, config),
                reasoning_effort: provider.reasoning_effort(),
                follows_primary,
            };
        }
        let fork = provider.fork();
        let resolved = routing::apply_override(fork.as_ref(), config, selected.as_ref()).is_ok();
        let selection = resolved
            .then(|| current_selection(fork.as_ref(), config))
            .flatten();
        AdvisorModelSettings {
            enabled,
            selection,
            reasoning_effort: resolved.then(|| fork.reasoning_effort()).flatten(),
            follows_primary,
        }
    }

    pub fn model_options(
        &self,
        session: &str,
        provider: &dyn Provider,
        config: &AdvisorConfig,
        selection: Option<&RouteSelection>,
    ) -> Result<AdvisorModelOptions> {
        let available_routes = provider
            .model_routes()
            .into_iter()
            .filter(|route| routing::permitted(route, config))
            .take(10_000)
            .map(|mut route| {
                route.detail = truncate_utf8(redact_secrets(&route.detail), 256);
                route
            })
            .collect();
        let Some(selection) = selection else {
            return Ok(AdvisorModelOptions {
                selection: None,
                reasoning_effort: None,
                available_routes,
                available_efforts: Vec::new(),
            });
        };
        let canonical = routing::canonical_selection(provider, config, selection)?;
        let fork = provider.fork();
        fork.set_route_selection(&canonical)?;
        let available_efforts = routing::efforts(fork.as_ref());
        let saved_effort = match self.model_override(session) {
            Some(AdvisorModelOverride::Selected {
                selection,
                reasoning_effort,
            }) if selection == canonical => reasoning_effort,
            _ => None,
        };
        let reasoning_effort = saved_effort
            .or_else(|| fork.reasoning_effort())
            .filter(|effort| available_efforts.contains(effort));
        Ok(AdvisorModelOptions {
            selection: Some(canonical),
            reasoning_effort,
            available_routes,
            available_efforts,
        })
    }

    pub fn select_model(
        &self,
        session: &str,
        provider: &dyn Provider,
        config: &AdvisorConfig,
        selection: RouteSelection,
        reasoning_effort: Option<String>,
        request_id: u64,
    ) -> Result<AdvisorModelSettings> {
        let selection = routing::canonical_selection(provider, config, &selection)?;
        let reasoning_effort = reasoning_effort.map(|effort| effort.trim().to_lowercase());
        if reasoning_effort
            .as_ref()
            .is_some_and(|effort| effort.len() > 32)
        {
            bail!("reasoning effort is invalid");
        }
        let selected = AdvisorModelOverride::Selected {
            selection,
            reasoning_effort,
        };
        // Validate on a private fork before touching durable session controls.
        routing::apply_override(provider.fork().as_ref(), config, Some(&selected))?;
        self.store_model_override(session, selected, request_id)?;
        Ok(self.model_settings(session, provider, config))
    }

    pub fn use_primary_model(
        &self,
        session: &str,
        provider: &dyn Provider,
        config: &AdvisorConfig,
        request_id: u64,
    ) -> Result<AdvisorModelSettings> {
        routing::apply_override(
            provider.fork().as_ref(),
            config,
            Some(&AdvisorModelOverride::Primary),
        )?;
        self.store_model_override(session, AdvisorModelOverride::Primary, request_id)?;
        Ok(self.model_settings(session, provider, config))
    }

    fn store_model_override(
        &self,
        session: &str,
        selected: AdvisorModelOverride,
        request_id: u64,
    ) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("advisor state unavailable"))?;
        let runtime = sessions.entry(session.to_string()).or_default();
        if runtime.model_selection_id != request_id {
            bail!("advisor model selection was superseded by a newer control request");
        }
        let previous_selection = runtime.model_override.replace(selected);
        let previous_enabled = runtime.enabled_override.replace(true);
        // Old provider completions cannot publish under a new model choice.
        runtime.pending = None;
        runtime.active_review_id = 0;
        runtime.private_context.clear();
        clear_queued_notes(runtime);
        runtime.status = AdvisorStatus::Idle;
        runtime.last_error = None;
        if let Err(error) = self.persist(session, runtime) {
            runtime.model_override = previous_selection;
            runtime.enabled_override = previous_enabled;
            runtime.status = AdvisorStatus::Failed;
            return Err(error);
        }
        Ok(())
    }
}

fn current_selection(provider: &dyn Provider, config: &AdvisorConfig) -> Option<RouteSelection> {
    let model = provider.model();
    let mut routes = provider.model_routes().into_iter().filter(|route| {
        route.model == model
            && routing::current_runtime(route, provider)
            && routing::permitted(route, config)
    });
    let mut selection = RouteSelection::from_model_route(&routes.next()?);
    if routes.next().is_some() {
        return None;
    }
    selection.detail.clear();
    Some(selection)
}

#[cfg(test)]
#[path = "model_selection_tests.rs"]
mod tests;
