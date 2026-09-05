use super::{DEFAULT_MAX_TOKENS, Sidecar, SidecarBackend};
use crate::config::ConfigModelRoute;
use crate::message::{ContentBlock, Message, Role, StreamEvent};
use crate::provider::{Provider, fork_for_agent_role};
use anyhow::{Context, Result};
use futures::StreamExt;
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct SidecarConfigurationError(pub(super) Arc<str>);

impl fmt::Display for SidecarConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Memory sidecar model configuration: {}", self.0)
    }
}

impl std::error::Error for SidecarConfigurationError {}

pub(super) fn require_toolless_provider(provider: &dyn Provider) -> Result<()> {
    if !provider.supports_toolless_requests() {
        return Err(SidecarConfigurationError(Arc::from(
            "the selected provider cannot disable its built-in tools; select another memory model in /agents",
        ))
        .into());
    }
    Ok(())
}

pub(super) async fn complete_on_selected_route(
    provider: &dyn Provider,
    system: &str,
    prompt: &str,
) -> Result<String> {
    require_toolless_provider(provider)?;
    let messages = [Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: prompt.to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    let mut response = provider
        .complete_on_selected_route(&messages, &[], system, None)
        .await?;
    let mut text = String::new();
    while let Some(event) = response.next().await {
        if let StreamEvent::TextDelta(delta) = event? {
            text.push_str(&delta);
        }
    }
    Ok(text)
}

impl Sidecar {
    pub(super) fn with_role_provider(
        route: Option<&ConfigModelRoute>,
        legacy_model: Option<&str>,
        effort: Option<&str>,
        provider: Option<Arc<dyn Provider>>,
    ) -> Self {
        let selected = (|| -> Result<Arc<dyn Provider>> {
            let provider = provider.context(
                "no active provider is available; sign in and select the memory model again in /agents",
            )?;
            let selected = fork_for_agent_role(provider.as_ref(), route, legacy_model, effort)?;
            require_toolless_provider(selected.as_ref())?;
            Ok(selected)
        })();
        let (provider, initialization_error) = match selected {
            Ok(provider) => (Some(provider), None),
            Err(error) => (None, Some(Arc::from(format!("{error:#}")))),
        };
        let model = provider
            .as_ref()
            .map(|provider| provider.model())
            .unwrap_or_else(|| {
                route
                    .map(|route| route.model.as_str())
                    .or(legacy_model)
                    .unwrap_or("unavailable")
                    .to_string()
            });
        Self {
            client: crate::provider::shared_http_client(),
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
            backend: SidecarBackend::Provider,
            provider,
            reasoning_override: effort.map(str::to_string),
            initialization_error,
        }
    }
}

#[cfg(test)]
#[path = "role_selection_tests.rs"]
mod tests;
