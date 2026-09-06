//! Compatible model selection.

use super::*;

impl MultiProvider {
    pub(super) fn openai_compatible_model_prefix(
        model: &str,
    ) -> Option<(crate::provider_catalog::OpenAiCompatibleProfile, &str)> {
        let (prefix, rest) = model.split_once(':')?;
        if explicit_model_provider_prefix(model).is_some() {
            return None;
        }
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }

        let profile = crate::provider_catalog::openai_compatible_profile_by_id(prefix)?;
        Some((profile, rest))
    }

    /// Find the configured OpenAI-compatible profile that serves a bare model
    /// id, using the live route catalog as the source of truth.
    ///
    /// Route specs from the picker carry a `<profile>:<model>` prefix, but
    /// hand-typed `/model <id>` and saved sessions can carry the bare id. The
    /// active profile wins when several profiles serve the same id, so a
    /// re-select of the current model never silently hops endpoints.
    pub(super) fn openai_compatible_profile_owning_model(
        &self,
        model: &str,
    ) -> Option<crate::provider_catalog::OpenAiCompatibleProfile> {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }

        let active_profile_id = ProviderRegistry::new(self).active_compatible_profile_id();
        let mut fallback: Option<String> = None;
        for route in self.fresh_routes_memo_entry().routes {
            if !route.available || route.model != model {
                continue;
            }
            let Some(profile_id) = route
                .api_method
                .strip_prefix("openai-compatible:")
                .map(str::trim)
                .filter(|profile_id| !profile_id.is_empty())
            else {
                continue;
            };
            if active_profile_id.as_deref() == Some(profile_id) {
                fallback = Some(profile_id.to_string());
                break;
            }
            if fallback.is_none() {
                fallback = Some(profile_id.to_string());
            }
        }

        crate::provider_catalog::openai_compatible_profile_by_id(&fallback?)
    }

    /// Return the active direct OpenAI-compatible runtime when its own catalog
    /// serves `model`. Bare model switches must stay on that runtime rather than
    /// rebinding the shared slot to native OpenRouter.
    pub(super) fn active_openai_compatible_profile_serving_model(
        &self,
        model: &str,
    ) -> Option<Arc<dyn Provider>> {
        if self.active_provider() != ActiveProvider::OpenRouter {
            return None;
        }
        let provider = self.active_openrouter_execution_provider()?;
        if provider.supports_provider_routing_features() {
            return None;
        }
        let (_, api_method, _) = provider.direct_openai_compatible_route_parts()?;
        self.fresh_routes_memo_entry()
            .routes
            .iter()
            .any(|route| route.available && route.model == model && route.api_method == api_method)
            .then_some(provider)
    }

    /// Parse a `<name>:<model>` spec whose prefix is a user-defined named
    /// provider profile from config (`[providers.<name>]`). Built-in provider
    /// prefixes and catalog profile ids take precedence and never reach here.
    pub(super) fn named_provider_profile_model_prefix(model: &str) -> Option<(String, String)> {
        let (prefix, rest) = model.split_once(':')?;
        if explicit_model_provider_prefix(model).is_some()
            || Self::openai_compatible_model_prefix(model).is_some()
        {
            return None;
        }
        let prefix = prefix.trim();
        let rest = rest.trim();
        if prefix.is_empty() || rest.is_empty() {
            return None;
        }
        crate::config::config()
            .providers
            .contains_key(prefix)
            .then(|| (prefix.to_string(), rest.to_string()))
    }
}
