use super::*;
use crate::message::{Message, ToolDefinition};
use crate::provider::{EventStream, ModelRoute};
use async_trait::async_trait;
use std::sync::Mutex;

struct RoleProvider {
    selection: Mutex<RouteSelection>,
    effort: Mutex<String>,
    available: bool,
}

fn route(api: &str, label: &str) -> ConfigModelRoute {
    ConfigModelRoute {
        model: "gpt-5.5".into(),
        api_method: api.into(),
        provider_label: label.into(),
    }
}

impl RoleProvider {
    fn new() -> Self {
        Self {
            selection: Mutex::new(configured_role_route(&route("openai-api-key", "OpenAI"))),
            effort: Mutex::new("high".into()),
            available: true,
        }
    }
}

#[async_trait]
impl Provider for RoleProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        bail!("role selection tests do not send requests")
    }

    fn name(&self) -> &str { "role-test" }
    fn model(&self) -> String { self.selection.lock().unwrap().model.clone() }
    fn model_routes(&self) -> Vec<ModelRoute> {
        [route("openai-oauth", "OpenAI"), route("openrouter", "OpenAI"), route("openai-compatible:team", "Team")]
            .into_iter()
            .map(|route| ModelRoute {
                model: route.model,
                provider: route.provider_label,
                api_method: route.api_method,
                available: self.available,
                detail: "not persisted".into(),
                cheapness: None,
            })
            .collect()
    }
    fn set_route_selection(&self, selection: &RouteSelection) -> Result<()> {
        *self.selection.lock().unwrap() = selection.clone();
        Ok(())
    }
    fn active_resolved_credential(&self) -> Option<jcode_provider_core::ResolvedCredential> {
        Some(match self.selection.lock().unwrap().runtime_key {
            RuntimeKey::OpenAIOAuth => jcode_provider_core::ResolvedCredential::Oauth,
            _ => jcode_provider_core::ResolvedCredential::ApiKey,
        })
    }
    fn available_efforts(&self) -> Vec<&'static str> { vec!["low", "high", "swarm"] }
    fn reasoning_effort(&self) -> Option<String> { Some(self.effort.lock().unwrap().clone()) }
    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        *self.effort.lock().unwrap() = effort.into();
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            selection: Mutex::new(self.selection.lock().unwrap().clone()),
            effort: Mutex::new(self.effort.lock().unwrap().clone()),
            available: self.available,
        })
    }
}

#[test]
fn configured_roles_preserve_oauth_custom_profiles_and_openrouter_pins() {
    assert_eq!(configured_role_route(&route("openai-oauth", "OpenAI")).runtime_key, RuntimeKey::OpenAIOAuth);
    assert_eq!(configured_role_route(&route("openai-compatible:team", "Team")).runtime_key,
        RuntimeKey::OpenAiCompatible { profile_id: Some("team".into()) });
    let pinned = configured_role_route(&route("openrouter", "OpenAI"));
    assert_eq!(pinned.runtime_key, RuntimeKey::OpenRouter);
    assert_eq!(pinned.routed_model_spec(), "openai/gpt-5.5@OpenAI");
    assert!(pinned.detail.is_empty());
}

#[test]
fn configured_role_effort_and_route_do_not_mutate_primary() {
    let primary = RoleProvider::new();
    let selected = fork_for_agent_role(&primary, Some(&route("openai-oauth", "OpenAI")), None, Some("low")).unwrap();
    assert_eq!(selected.reasoning_effort().as_deref(), Some("low"));
    assert_eq!(selected.active_resolved_credential(), Some(jcode_provider_core::ResolvedCredential::Oauth));
    assert_eq!(primary.reasoning_effort().as_deref(), Some("high"));
    assert_eq!(primary.selection.lock().unwrap().runtime_key, RuntimeKey::OpenAIApiKey);
}

#[test]
fn configured_role_unavailable_route_or_effort_fails_without_fallback() {
    let mut primary = RoleProvider::new();
    let saved = route("openai-oauth", "OpenAI");
    for effort in ["unsupported", "swarm"] {
        assert!(fork_for_agent_role(&primary, Some(&saved), None, Some(effort)).is_err());
    }
    assert!(fork_for_agent_role(&primary, Some(&route("openai-oauth", "Other account")), None, None).is_err());
    primary.available = false;
    assert!(fork_for_agent_role(&primary, Some(&saved), None, None).is_err());
    assert_eq!(primary.reasoning_effort().as_deref(), Some("high"));
}

#[test]
fn inherited_role_preserves_primary_effort() {
    let primary = RoleProvider::new();
    let inherited = fork_for_agent_role(&primary, None, Some("inherit"), None).unwrap();
    assert_eq!(inherited.model(), primary.model());
    assert_eq!(inherited.reasoning_effort(), primary.reasoning_effort());
}
