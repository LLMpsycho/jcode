use super::*;

impl BridgeState {
    pub(super) fn note_models(&mut self, event: &Value) {
        self.model_catalog_loaded = true;
        if let Some(models) = event["available_models"].as_array() {
            let names: Vec<String> = models
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect();
            self.available_models = names;
        }
        if let Some(model) = event["provider_model"].as_str() {
            self.current_model = Some(model.to_string());
        }
        if let Some(provider) = event["provider_name"].as_str() {
            self.note_provider(provider);
        }
        if event.get("reasoning_effort").is_some() {
            self.current_effort = event["reasoning_effort"].as_str().map(str::to_string);
        }
        if let Some(routes) = event["available_model_routes"].as_array() {
            self.available_routes = routes
                .iter()
                .filter_map(|route| {
                    Some(ModelRouteInfo {
                        model: route["model"].as_str()?.to_string(),
                        provider: route["provider"].as_str()?.to_string(),
                        api_method: route["api_method"].as_str()?.to_string(),
                        available: route["available"].as_bool().unwrap_or(false),
                        detail: route["detail"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect();
        }
    }

    pub(super) fn note_provider(&mut self, provider: &str) {
        if self.current_provider.as_deref() != Some(provider) {
            // Effort is provider-specific. ModelChanged and auth pushes can
            // omit it, so never carry the previous provider's setting across.
            self.current_effort = None;
        }
        self.current_provider = Some(provider.to_string());
    }

    pub(super) fn runtime_info(&self) -> ApiEvent {
        ApiEvent::RuntimeInfo {
            session_id: self.session_id.as_deref().unwrap_or("").to_owned(),
            provider: self.current_provider.clone(),
            model: self.current_model.clone(),
            reasoning_effort: self.current_effort.clone(),
            routes: self.available_routes.clone(),
        }
    }

    pub(super) fn model_info(&self, session_id: String, event: &Value) -> ApiEvent {
        ApiEvent::ModelInfo {
            session_id,
            provider: event["provider_name"].as_str().map(str::to_string),
            model: event["provider_model"].as_str().map(str::to_string),
            reasoning_effort: event["reasoning_effort"]
                .as_str()
                .map(str::to_string)
                .or_else(|| self.current_effort.clone()),
        }
    }
}
