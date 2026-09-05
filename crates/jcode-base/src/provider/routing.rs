use super::{ActiveProvider, MultiProvider, RouteSelection, external};
use anyhow::Result;

impl MultiProvider {
    pub(super) fn set_model_on_unnamed_compatible_route(
        &self,
        selection: &RouteSelection,
    ) -> Result<()> {
        let runtime = self
            .openrouter_provider()
            .filter(|runtime| {
                runtime
                    .direct_openai_compatible_route_parts()
                    .is_some_and(|(label, api, _)| {
                        label == selection.provider_label && api == selection.api_method
                    })
            })
            .ok_or_else(|| anyhow::anyhow!("Selected OpenAI-compatible runtime is unavailable"))?;
        runtime.set_model(&selection.model)?;
        self.clear_active_openai_compatible_profile();
        self.set_active_provider(ActiveProvider::OpenRouter);
        Ok(())
    }

    /// A structured aggregator route is an exact transport choice. Legacy
    /// model switching intentionally preserves custom endpoints, so it cannot
    /// be used here without risking the selected route's credentials.
    pub(super) fn set_model_on_explicit_openrouter_route(
        &self,
        selection: &RouteSelection,
    ) -> Result<()> {
        let runtime = match self
            .openrouter_provider()
            .filter(|runtime| runtime.supports_provider_routing_features())
        {
            Some(runtime) => runtime,
            None => external::instantiate_openrouter_runtime(
                external::OpenRouterRuntimeSpec::OpenRouterApiKey,
            )?,
        };
        if !runtime.supports_provider_routing_features() {
            anyhow::bail!("Selected OpenRouter runtime is unavailable");
        }
        runtime.set_model(&selection.routed_model_spec())?;
        *self
            .openrouter
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(runtime);
        self.clear_active_openai_compatible_profile();
        self.set_active_provider(ActiveProvider::OpenRouter);
        Ok(())
    }
}

pub(crate) fn anthropic_oauth_route_availability(model: &str) -> (bool, String) {
    if model.ends_with("[1m]") && !crate::usage::has_extra_usage() {
        (false, "requires extra usage".to_string())
    } else if model.contains("opus") && !crate::auth::claude::is_max_subscription() {
        (false, "requires Max subscription".to_string())
    } else {
        (true, String::new())
    }
}

pub(crate) fn anthropic_api_key_route_availability(model: &str) -> (bool, String) {
    if model.ends_with("[1m]") && !crate::usage::has_extra_usage() {
        (false, "requires extra usage".to_string())
    } else {
        (true, String::new())
    }
}
