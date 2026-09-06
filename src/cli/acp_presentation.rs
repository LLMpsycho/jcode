use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcpProfile {
    Standard,
    Extended,
    Full,
}

impl AcpProfile {
    pub(super) fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "extended" => Self::Extended,
            "full" => Self::Full,
            _ => Self::Standard,
        }
    }

    pub(super) fn is_extended(self) -> bool {
        matches!(self, Self::Extended | Self::Full)
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Extended => "extended",
            Self::Full => "full",
        }
    }
}

/// Session-scoped provider/model state used to surface ACP `configOptions`
/// (model selector, reasoning effort) and `usage_update` notifications.
#[derive(Clone, Debug, Default)]
pub(super) struct SessionUiState {
    pub(super) provider_name: Option<String>,
    pub(super) model: Option<String>,
    pub(super) available_models: Vec<String>,
    pub(super) reasoning_effort: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct TurnUsage {
    pub(super) reported: bool,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_read_tokens: Option<u64>,
    pub(super) cached_write_tokens: Option<u64>,
}

impl TurnUsage {
    pub(super) fn add(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cached_read_tokens: Option<u64>,
        cached_write_tokens: Option<u64>,
    ) {
        self.reported = true;
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        add_optional_tokens(&mut self.cached_read_tokens, cached_read_tokens);
        add_optional_tokens(&mut self.cached_write_tokens, cached_write_tokens);
    }

    pub(super) fn to_acp(&self) -> Option<Value> {
        if !self.reported {
            return None;
        }

        let total_tokens = self
            .input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cached_read_tokens.unwrap_or(0))
            .saturating_add(self.cached_write_tokens.unwrap_or(0));
        let mut object = serde_json::Map::from_iter([
            ("totalTokens".to_string(), json!(total_tokens)),
            ("inputTokens".to_string(), json!(self.input_tokens)),
            ("outputTokens".to_string(), json!(self.output_tokens)),
        ]);
        if let Some(tokens) = self.cached_read_tokens {
            object.insert("cachedReadTokens".to_string(), json!(tokens));
        }
        if let Some(tokens) = self.cached_write_tokens {
            object.insert("cachedWriteTokens".to_string(), json!(tokens));
        }
        Some(Value::Object(object))
    }
}

pub(super) fn add_optional_tokens(total: &mut Option<u64>, tokens: Option<u64>) {
    if let Some(tokens) = tokens {
        *total = Some(total.unwrap_or(0).saturating_add(tokens));
    }
}

pub(super) fn prompt_response(stop_reason: &str, usage: &TurnUsage) -> Value {
    let mut response = serde_json::Map::from_iter([("stopReason".to_string(), json!(stop_reason))]);
    if let Some(usage) = usage.to_acp() {
        response.insert("usage".to_string(), usage);
    }
    Value::Object(response)
}

impl SessionUiState {
    pub(super) fn from_history_fields(
        provider_name: Option<String>,
        provider_model: Option<String>,
        available_models: Vec<String>,
        reasoning_effort: Option<String>,
    ) -> Self {
        Self {
            provider_name,
            model: provider_model,
            available_models,
            reasoning_effort,
        }
    }

    pub(super) fn context_limit(&self) -> u64 {
        self.model
            .as_deref()
            .and_then(|model| {
                crate::provider::context_limit_for_model_with_provider(
                    model,
                    self.provider_name.as_deref(),
                )
            })
            .unwrap_or(crate::provider::DEFAULT_CONTEXT_LIMIT) as u64
    }
}

pub(super) const CONFIG_ID_MODEL: &str = "model";
pub(super) const CONFIG_ID_EFFORT: &str = "reasoning_effort";

pub(super) fn acp_available_commands() -> Vec<Value> {
    vec![
        json!({
            "name": "model",
            "description": "Switch the model for this session, or show the current model",
            "input": { "hint": "model id (optional)" },
        }),
        json!({
            "name": "models",
            "description": "List models available from the active provider",
        }),
        json!({
            "name": "effort",
            "description": "Set reasoning effort, or show the current effort",
            "input": { "hint": "none|minimal|low|medium|high|xhigh|max (optional)" },
        }),
    ]
}

pub(super) fn insert_session_configuration(result: &mut Value, state: &SessionUiState) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let config_options = session_config_options(state);
    if !config_options.is_empty() {
        object.insert("configOptions".to_string(), Value::Array(config_options));
    }
    if let Some(models) = session_models(state) {
        object.insert("models".to_string(), models);
    }
}

pub(super) fn session_models(state: &SessionUiState) -> Option<Value> {
    let current = state.model.as_deref()?;
    let mut models = state.available_models.clone();
    if !models.iter().any(|candidate| candidate == current) {
        models.insert(0, current.to_string());
    }
    Some(json!({
        "availableModels": models
            .into_iter()
            .map(|model| json!({ "modelId": model, "name": model }))
            .collect::<Vec<_>>(),
        "currentModelId": current,
    }))
}

pub(super) fn available_efforts(state: &SessionUiState) -> Vec<&'static str> {
    crate::provider::inferred_reasoning_efforts(
        state.provider_name.as_deref(),
        state.model.as_deref(),
    )
    .into_iter()
    // `swarm`/`swarm-deep` are TUI sentinels, not provider effort levels.
    .filter(|effort| !effort.starts_with("swarm"))
    .collect()
}

/// Build the ACP `configOptions` array (model selector plus reasoning effort)
/// from the current session provider state. Empty when the daemon reported no
/// usable model state.
pub(super) fn session_config_options(state: &SessionUiState) -> Vec<Value> {
    let mut options = Vec::new();

    if let Some(model) = state.model.as_deref() {
        let mut models = state.available_models.clone();
        if !models.iter().any(|candidate| candidate == model) {
            models.insert(0, model.to_string());
        }
        let select_options: Vec<Value> = models
            .iter()
            .map(|name| json!({ "value": name, "name": name }))
            .collect();
        options.push(json!({
            "type": "select",
            "id": CONFIG_ID_MODEL,
            "name": "Model",
            "category": "model",
            "currentValue": model,
            "options": select_options,
        }));
    }

    let efforts = available_efforts(state);
    if !efforts.is_empty() {
        let current = state
            .reasoning_effort
            .as_deref()
            .filter(|effort| efforts.contains(effort))
            .unwrap_or_else(|| {
                if efforts.contains(&"medium") {
                    "medium"
                } else {
                    efforts[0]
                }
            });
        let select_options: Vec<Value> = efforts
            .iter()
            .map(|name| json!({ "value": name, "name": name }))
            .collect();
        options.push(json!({
            "type": "select",
            "id": CONFIG_ID_EFFORT,
            "name": "Reasoning effort",
            "category": "thought_level",
            "currentValue": current,
            "options": select_options,
        }));
    }

    options
}

pub(super) fn agent_message_chunk(text: String) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": {
            "type": "text",
            "text": text,
        }
    })
}

pub(super) fn tool_title(name: &str) -> String {
    match name {
        "bash" => "Running shell command".to_string(),
        "read" => "Reading file".to_string(),
        "write" => "Writing file".to_string(),
        "edit" | "multiedit" | "patch" | "apply_patch" => "Editing files".to_string(),
        "agentgrep" | "grep" | "glob" | "ls" => "Searching workspace".to_string(),
        "webfetch" | "websearch" => "Fetching web content".to_string(),
        other => other.replace('_', " "),
    }
}

pub(crate) fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" => "read",
        "write" | "edit" | "multiedit" | "patch" | "apply_patch" => "edit",
        "bash" | "bg" | "selfdev" => "execute",
        "agentgrep" | "grep" | "glob" | "ls" | "session_search" | "conversation_search" => "search",
        "webfetch" | "websearch" | "codesearch" => "fetch",
        _ => "other",
    }
}
