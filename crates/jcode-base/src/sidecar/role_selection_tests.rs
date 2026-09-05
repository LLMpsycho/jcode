use super::*;
use crate::message::{Message, StreamEvent, ToolDefinition};
use crate::provider::{EventStream, ModelRoute, RouteSelection};
use crate::sidecar::{SidecarErrorKind, classify_error};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RoleProvider {
    model: Mutex<String>,
    effort: Mutex<String>,
    calls: Arc<AtomicUsize>,
    toolless: bool,
}

impl RoleProvider {
    fn new(toolless: bool) -> Self {
        Self {
            model: Mutex::new("main-model".to_string()),
            effort: Mutex::new("low".to_string()),
            calls: Arc::new(AtomicUsize::new(0)),
            toolless,
        }
    }
}

#[async_trait::async_trait]
impl Provider for RoleProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        anyhow::bail!("Memory must not use the transport that permits automatic failover")
    }

    async fn complete_on_selected_route(
        &self,
        _messages: &[Message],
        tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        assert!(tools.is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = format!("{}:{}", self.model(), self.reasoning_effort().unwrap());
        Ok(Box::pin(futures::stream::once(async move {
            Ok(StreamEvent::TextDelta(text))
        })))
    }

    fn name(&self) -> &str {
        "memory-test"
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: Mutex::new(self.model()),
            effort: Mutex::new(self.reasoning_effort().unwrap()),
            calls: self.calls.clone(),
            toolless: self.toolless,
        })
    }

    fn model_routes(&self) -> Vec<ModelRoute> {
        vec![ModelRoute {
            model: "gpt-5.4".to_string(),
            provider: "OpenAI".to_string(),
            api_method: "openai-oauth".to_string(),
            available: true,
            detail: "signed-in account".to_string(),
            cheapness: None,
        }]
    }

    fn set_model(&self, model: &str) -> Result<()> {
        anyhow::ensure!(model == "copilot:claude-opus-4.6", "Unknown model");
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn set_route_selection(&self, selection: &RouteSelection) -> Result<()> {
        *self.model.lock().unwrap() = selection.model.clone();
        Ok(())
    }

    fn reasoning_effort(&self) -> Option<String> {
        Some(self.effort.lock().unwrap().clone())
    }

    fn available_efforts(&self) -> Vec<&'static str> {
        vec!["low", "high"]
    }

    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        *self.effort.lock().unwrap() = effort.to_string();
        Ok(())
    }

    fn supports_toolless_requests(&self) -> bool {
        self.toolless
    }
}

fn selected_route() -> ConfigModelRoute {
    ConfigModelRoute {
        model: "gpt-5.4".to_string(),
        provider_label: "OpenAI".to_string(),
        api_method: "openai-oauth".to_string(),
    }
}

#[tokio::test]
async fn explicit_memory_route_and_effort_are_used_without_changing_the_primary() {
    let primary = Arc::new(RoleProvider::new(true));
    let sidecar = Sidecar::with_role_provider(
        Some(&selected_route()),
        Some("legacy-model-ignored"),
        Some("high"),
        Some(primary.clone()),
    );

    assert_eq!(sidecar.complete("system", "user").await.unwrap(), "gpt-5.4:high");
    assert_eq!(sidecar.model_name(), "gpt-5.4");
    assert_eq!(primary.model(), "main-model");
    assert_eq!(primary.reasoning_effort().as_deref(), Some("low"));
    assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn legacy_memory_override_uses_the_selected_signed_in_provider() {
    let primary = Arc::new(RoleProvider::new(true));
    let sidecar = Sidecar::with_role_provider(
        None,
        Some("copilot:claude-opus-4.6"),
        Some("high"),
        Some(primary.clone()),
    );

    assert_eq!(
        sidecar.complete("system", "user").await.unwrap(),
        "copilot:claude-opus-4.6:high"
    );
    assert_eq!(primary.model(), "main-model");
}

#[tokio::test]
async fn unavailable_memory_selection_never_falls_back_or_retries_permanently() {
    let primary = Arc::new(RoleProvider::new(true));
    let mut unavailable = selected_route();
    unavailable.api_method = "openai-api".to_string();
    for sidecar in [
        Sidecar::with_role_provider(Some(&unavailable), None, None, Some(primary.clone())),
        Sidecar::with_role_provider(
            Some(&selected_route()), None, Some("swarm"), Some(primary.clone()),
        ),
        Sidecar::with_role_provider(Some(&selected_route()), None, None, None),
    ] {
        let error = sidecar.complete("system", "user").await.unwrap_err();
        assert_eq!(classify_error(&error), SidecarErrorKind::Permanent);
        assert!(error.to_string().contains("Memory sidecar model configuration"));
    }
    assert_eq!(primary.calls.load(Ordering::SeqCst), 0);
    assert_eq!(primary.model(), "main-model");
}

#[tokio::test]
async fn memory_rejects_providers_that_cannot_disable_hosted_tools() {
    let primary = Arc::new(RoleProvider::new(false));
    let sidecar = Sidecar::with_role_provider(Some(&selected_route()), None, None, Some(primary.clone()));
    let error = sidecar.complete("system", "user").await.unwrap_err();
    assert!(error.to_string().contains("cannot disable its built-in tools"));
    assert_eq!(classify_error(&error), SidecarErrorKind::Permanent);
    assert_eq!(primary.calls.load(Ordering::SeqCst), 0);
}
