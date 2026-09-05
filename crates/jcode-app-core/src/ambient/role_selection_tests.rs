use super::*;
use crate::config::{Config, ConfigModelRoute};
use crate::message::{Message, StreamEvent, ToolDefinition};
use crate::provider::{EventStream, ModelRoute, RouteSelection, RuntimeKey};
use jcode_provider_core::ResolvedCredential;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

struct HomeGuard(Option<std::ffi::OsString>);

impl HomeGuard {
    fn set(path: &std::path::Path) -> Self {
        let old = std::env::var_os("JCODE_HOME");
        crate::env::set_var("JCODE_HOME", path);
        Self(old)
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => crate::env::set_var("JCODE_HOME", value),
            None => crate::env::remove_var("JCODE_HOME"),
        }
    }
}

struct RoleProvider {
    model: Mutex<String>,
    effort: Mutex<String>,
    calls: Arc<Mutex<Vec<(String, Option<String>)>>>,
    credential: Mutex<Option<ResolvedCredential>>,
    pin_route_credential: bool,
    route_pinned: AtomicBool,
}

impl RoleProvider {
    fn new() -> Self {
        Self {
            model: Mutex::new("primary-model".to_string()),
            effort: Mutex::new("low".to_string()),
            calls: Arc::new(Mutex::new(Vec::new())),
            credential: Mutex::new(Some(ResolvedCredential::Oauth)),
            pin_route_credential: true,
            route_pinned: AtomicBool::new(false),
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
    ) -> anyhow::Result<EventStream> {
        assert!(
            self.route_pinned(),
            "ambient requests must disable automatic route failover"
        );
        self.calls
            .lock()
            .unwrap()
            .push((self.model(), self.reasoning_effort()));
        Ok(Box::pin(futures::stream::once(async {
            Ok(StreamEvent::TextDelta("Cycle checked.".to_string()))
        })))
    }

    fn name(&self) -> &str {
        "ambient-test"
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: Mutex::new(self.model()),
            effort: Mutex::new(self.reasoning_effort().unwrap()),
            calls: self.calls.clone(),
            credential: Mutex::new(self.active_resolved_credential()),
            pin_route_credential: self.pin_route_credential,
            route_pinned: AtomicBool::new(self.route_pinned()),
        })
    }

    fn model_routes(&self) -> Vec<ModelRoute> {
        ["openai-oauth", "openai-api"]
            .into_iter()
            .map(|api_method| ModelRoute {
                model: "gpt-5.4".to_string(),
                provider: "OpenAI".to_string(),
                api_method: api_method.to_string(),
                available: true,
                detail: String::new(),
                cheapness: None,
            })
            .collect()
    }

    fn set_model(&self, model: &str) -> anyhow::Result<()> {
        anyhow::ensure!(model == "gpt-5.4", "Unavailable ambient model");
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }

    fn set_route_selection(&self, selection: &RouteSelection) -> anyhow::Result<()> {
        if self.pin_route_credential {
            *self.credential.lock().unwrap() = Some(match &selection.runtime_key {
                RuntimeKey::OpenAIOAuth => ResolvedCredential::Oauth,
                _ => ResolvedCredential::ApiKey,
            });
        }
        self.set_model(&selection.model)
    }

    fn active_resolved_credential(&self) -> Option<ResolvedCredential> {
        *self.credential.lock().unwrap()
    }

    fn set_route_pinned(&self, pinned: bool) {
        self.route_pinned.store(pinned, Ordering::SeqCst);
    }

    fn route_pinned(&self) -> bool {
        self.route_pinned.load(Ordering::SeqCst)
    }

    fn reasoning_effort(&self) -> Option<String> {
        Some(self.effort.lock().unwrap().clone())
    }

    fn available_efforts(&self) -> Vec<&'static str> {
        vec!["low", "high"]
    }

    fn set_reasoning_effort(&self, effort: &str) -> anyhow::Result<()> {
        *self.effort.lock().unwrap() = effort.to_string();
        Ok(())
    }
}

fn role_config() -> Config {
    let mut config = Config::default();
    config.ambient.visible = false;
    config.ambient.route = Some(ConfigModelRoute {
        model: "gpt-5.4".to_string(),
        provider_label: "OpenAI".to_string(),
        api_method: "openai-oauth".to_string(),
    });
    config.ambient.effort = Some("high".to_string());
    config.agents.memory_sidecar_enabled = false;
    config
}

#[tokio::test]
async fn ambient_cycle_uses_configured_route_effort_and_records_actual_model() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    role_config().save().unwrap();
    let primary = Arc::new(RoleProvider::new());
    let provider: Arc<dyn Provider> = primary.clone();
    let runner = AmbientRunnerHandle::new(Arc::new(SafetySystem::new()));

    let (result, provider_name, model) = runner.run_cycle(&provider).await.unwrap();

    assert!(result.conversation.is_some());
    assert_eq!(provider_name, "ambient-test");
    assert_eq!(model, "gpt-5.4");
    let calls = primary.calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(
        calls
            .iter()
            .all(|(model, effort)| model == "gpt-5.4" && effort.as_deref() == Some("high"))
    );
    assert_eq!(primary.model(), "primary-model");
    assert_eq!(primary.reasoning_effort().as_deref(), Some("low"));
    assert!(!primary.route_pinned());
}

#[tokio::test]
async fn ambient_legacy_model_override_is_applied() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    let mut config = role_config();
    config.ambient.route = None;
    config.ambient.model = Some("gpt-5.4".to_string());
    config.save().unwrap();
    let primary = Arc::new(RoleProvider::new());
    let provider: Arc<dyn Provider> = primary.clone();
    let runner = AmbientRunnerHandle::new(Arc::new(SafetySystem::new()));

    let (_, _, model) = runner.run_cycle(&provider).await.unwrap();

    assert_eq!(model, "gpt-5.4");
    assert_eq!(primary.model(), "primary-model");
}

#[tokio::test]
async fn unavailable_ambient_route_fails_before_launching_or_sending_a_request() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    let mut config = role_config();
    config.ambient.route.as_mut().unwrap().model = "unavailable-model".to_string();
    config.save().unwrap();
    let primary = Arc::new(RoleProvider::new());
    let provider: Arc<dyn Provider> = primary.clone();
    let runner = AmbientRunnerHandle::new(Arc::new(SafetySystem::new()));

    let error = runner.run_cycle(&provider).await.unwrap_err();

    assert!(error.to_string().contains("route is unavailable"));
    assert!(primary.calls.lock().unwrap().is_empty());
    assert_eq!(primary.model(), "primary-model");
}

#[tokio::test]
async fn ambient_api_route_requires_explicit_api_key_permission() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    let mut config = role_config();
    config.ambient.route.as_mut().unwrap().api_method = "openai-api".to_string();
    config.save().unwrap();
    let primary = Arc::new(RoleProvider::new());
    let provider: Arc<dyn Provider> = primary.clone();
    let runner = AmbientRunnerHandle::new(Arc::new(SafetySystem::new()));

    let error = runner.run_cycle(&provider).await.unwrap_err();
    assert!(error.to_string().contains("ambient.allow_api_keys=false"));
    assert!(primary.calls.lock().unwrap().is_empty());
    assert_eq!(
        primary.active_resolved_credential(),
        Some(ResolvedCredential::Oauth)
    );

    config.ambient.allow_api_keys = true;
    config.save().unwrap();
    runner.run_cycle(&provider).await.unwrap();
    assert!(!primary.calls.lock().unwrap().is_empty());
    assert_eq!(
        primary.active_resolved_credential(),
        Some(ResolvedCredential::Oauth)
    );
}

#[test]
fn inherited_ambient_auth_is_checked_even_without_an_explicit_model() {
    let provider = RoleProvider::new();
    let mut config = crate::config::AmbientConfig::default();
    let selected = fork_ambient_provider(&provider, &config).unwrap();
    assert!(selected.route_pinned());
    assert!(!provider.route_pinned());

    *provider.credential.lock().unwrap() = Some(ResolvedCredential::ApiKey);
    assert!(fork_ambient_provider(&provider, &config).is_err());

    *provider.credential.lock().unwrap() = None;
    assert!(fork_ambient_provider(&provider, &config).is_err());

    config.allow_api_keys = true;
    assert!(fork_ambient_provider(&provider, &config).is_ok());
}

#[test]
fn ambient_rejects_api_credentials_even_when_catalog_route_claims_oauth() {
    let mut provider = RoleProvider::new();
    provider.pin_route_credential = false;
    *provider.credential.lock().unwrap() = Some(ResolvedCredential::ApiKey);
    let config = role_config();

    assert!(fork_ambient_provider(&provider, &config.ambient).is_err());
    assert!(provider.calls.lock().unwrap().is_empty());
}
