use super::*;
use crate::message::{Message, StreamEvent, ToolDefinition};
use crate::provider::{EventStream, ModelRoute};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct RestoreProvider {
    model: Mutex<String>,
    effort: Mutex<String>,
    pinned: AtomicBool,
    auth_valid: bool,
    calls: Arc<AtomicUsize>,
}

impl RestoreProvider {
    fn new(auth_valid: bool) -> Self {
        Self {
            model: Mutex::new("main-model".into()),
            effort: Mutex::new("high".into()),
            pinned: AtomicBool::new(false),
            auth_valid,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for RestoreProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume: Option<&str>,
    ) -> Result<EventStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.model(), "worker-model");
        assert_eq!(self.reasoning_effort().as_deref(), Some("low"));
        assert!(self.route_pinned());
        Ok(Box::pin(futures::stream::once(async {
            Ok(StreamEvent::TextDelta("Worker complete".into()))
        })))
    }
    fn name(&self) -> &str {
        "restore-test"
    }
    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }
    fn model_routes(&self) -> Vec<ModelRoute> {
        vec![ModelRoute {
            model: "worker-model".into(),
            api_method: "openai-oauth".into(),
            provider: "OpenAI".into(),
            available: true,
            detail: String::new(),
            cheapness: None,
        }]
    }
    fn set_model(&self, model: &str) -> Result<()> {
        anyhow::ensure!(
            model == "main-model",
            "legacy restoration must not configure this role"
        );
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }
    fn set_route_selection(&self, route: &RouteSelection) -> Result<()> {
        anyhow::ensure!(self.auth_valid, "credentials expired");
        *self.model.lock().unwrap() = route.model.clone();
        Ok(())
    }
    fn available_efforts(&self) -> Vec<&'static str> {
        vec!["low", "high"]
    }
    fn reasoning_effort(&self) -> Option<String> {
        Some(self.effort.lock().unwrap().clone())
    }
    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        anyhow::ensure!(
            self.available_efforts().contains(&effort),
            "unsupported effort"
        );
        *self.effort.lock().unwrap() = effort.to_string();
        Ok(())
    }
    fn set_route_pinned(&self, pinned: bool) {
        self.pinned.store(pinned, Ordering::SeqCst);
    }
    fn route_pinned(&self) -> bool {
        self.pinned.load(Ordering::SeqCst)
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: Mutex::new(self.model()),
            effort: Mutex::new(self.reasoning_effort().unwrap()),
            pinned: AtomicBool::new(self.route_pinned()),
            auth_valid: self.auth_valid,
            calls: self.calls.clone(),
        })
    }
}

fn saved_role() -> Session {
    let mut session = Session::create(None, Some("review".into()));
    session.model = Some("worker-model".into());
    session.role_model_selection = Some(ConfigModelRoute {
        model: "worker-model".into(),
        api_method: "openai-oauth".into(),
        provider_label: "OpenAI".into(),
    });
    session.provider_key = Some("openai-oauth".into());
    session.route_api_method = Some("openai-oauth".into());
    session.reasoning_effort = Some("low".into());
    session
}

struct HomeGuard(Option<std::ffi::OsString>);
impl HomeGuard {
    fn set(path: &std::path::Path) -> Self {
        let guard = Self(std::env::var_os("JCODE_HOME"));
        crate::env::set_var("JCODE_HOME", path);
        guard
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

#[tokio::test]
async fn role_model_resume_keeps_previous_session_and_provider_on_invalid_settings() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    for failure in ["auth", "model", "effort"] {
        let provider = Arc::new(RestoreProvider::new(failure != "auth"));
        let provider_dyn: Arc<dyn Provider> = provider.clone();
        let registry = Registry::new(provider_dyn.clone()).await;
        let mut agent = Agent::new(provider_dyn, registry);
        let initial_session = agent.session_id().to_string();
        let initial_status = agent.session.status.clone();
        let mut role = saved_role();
        if failure == "model" {
            role.role_model_selection.as_mut().unwrap().model = "removed-model".into();
        }
        if failure == "effort" {
            role.reasoning_effort = Some("removed-effort".into());
        }
        role.save().unwrap();
        assert!(
            agent.restore_session(&role.id).is_err(),
            "{failure} must fail visibly"
        );
        assert_eq!(agent.session_id(), initial_session);
        assert_eq!(agent.session.status, initial_status);
        assert_eq!(agent.provider_model(), "main-model");
        assert_eq!(provider.reasoning_effort().as_deref(), Some("high"));
        assert!(!provider.route_pinned());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn role_model_constructor_and_resume_dispatch_exact_route_and_effort() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    let provider = Arc::new(RestoreProvider::new(true));
    let provider_dyn: Arc<dyn Provider> = provider.clone();
    let role = saved_role();
    role.save().unwrap();
    let registry = Registry::new(provider_dyn.clone()).await;
    let agent =
        Agent::new_with_role_session(provider_dyn.clone(), registry, role.clone(), None).unwrap();
    assert_eq!(agent.provider_model(), "worker-model");
    assert_eq!(agent.session.reasoning_effort.as_deref(), Some("low"));
    let registry = Registry::new(provider_dyn.clone()).await;
    let mut resumed = Agent::new(provider_dyn, registry);
    resumed.restore_session(&role.id).unwrap();
    assert!(resumed.provider_handle().route_pinned());
    assert_eq!(resumed.provider_model(), "worker-model");
    let _stream = resumed
        .provider_handle()
        .complete(&[], &[], "", None)
        .await
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.model(), "main-model");
    assert!(!provider.route_pinned());
}

#[tokio::test]
async fn role_model_constructor_rejects_expired_auth_and_manual_model_clears_marker() {
    let _lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(temp.path());
    let unavailable: Arc<dyn Provider> = Arc::new(RestoreProvider::new(false));
    let registry = Registry::new(unavailable.clone()).await;
    assert!(Agent::new_with_role_session(unavailable, registry, saved_role(), None).is_err());
    let provider: Arc<dyn Provider> = Arc::new(RestoreProvider::new(true));
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new_with_role_session(provider, registry, saved_role(), None).unwrap();
    agent.set_model("main-model").unwrap();
    assert!(agent.session.role_model_selection.is_none());
    assert!(!agent.provider_handle().route_pinned());
    assert!(
        Session::load(agent.session_id())
            .unwrap()
            .role_model_selection
            .is_none()
    );
}
