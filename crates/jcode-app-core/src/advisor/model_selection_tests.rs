use super::*;
use crate::message::ToolDefinition;
use crate::provider::{EventStream, ModelRoute, RuntimeKey};
use async_trait::async_trait;
use futures::stream;
use jcode_provider_core::ResolvedCredential;

struct CatalogProvider {
    routes: Vec<ModelRoute>,
    selected: Mutex<RouteSelection>,
    effort: Mutex<String>,
    calls: Arc<Mutex<Vec<(RouteSelection, String)>>>,
    gate: Option<Arc<tokio::sync::Notify>>,
    started: Arc<tokio::sync::Notify>,
    internal_tools: bool,
}

impl CatalogProvider {
    fn new() -> Self {
        let routes = vec![
            route("coder", "openai-oauth", true),
            route("reviewer", "openai-oauth", true),
            route("reviewer", "openai-api", false),
            route("plain", "openai-oauth", true),
        ];
        Self {
            selected: Mutex::new(RouteSelection::from_model_route(&routes[0])),
            routes,
            effort: Mutex::new("low".into()),
            calls: Arc::new(Mutex::new(Vec::new())),
            gate: None,
            started: Arc::new(tokio::sync::Notify::new()),
            internal_tools: false,
        }
    }

    fn reviewer(&self) -> RouteSelection {
        RouteSelection::from_model_route(&self.routes[1])
    }
}

#[async_trait]
impl Provider for CatalogProvider {
    fn name(&self) -> &str {
        "openai"
    }
    fn model(&self) -> String {
        self.selected.lock().expect("model").model.clone()
    }
    fn model_routes(&self) -> Vec<ModelRoute> {
        self.routes.clone()
    }
    fn handles_tools_internally(&self) -> bool {
        self.internal_tools
    }
    fn active_resolved_credential(&self) -> Option<ResolvedCredential> {
        Some(ResolvedCredential::Oauth)
    }
    fn set_route_selection(&self, selection: &RouteSelection) -> Result<()> {
        *self.selected.lock().expect("model") = selection.clone();
        Ok(())
    }
    fn reasoning_effort(&self) -> Option<String> {
        (self.model() != "plain").then(|| self.effort.lock().expect("effort").clone())
    }
    fn available_efforts(&self) -> Vec<&'static str> {
        if self.model() == "plain" {
            vec![]
        } else {
            vec!["low", "high", "swarm", "swarm-deep"]
        }
    }
    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        if !self.available_efforts().contains(&effort) {
            bail!("unsupported effort")
        }
        *self.effort.lock().expect("effort") = effort.into();
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            routes: self.routes.clone(),
            selected: Mutex::new(self.selected.lock().expect("model").clone()),
            effort: Mutex::new(self.effort.lock().expect("effort").clone()),
            calls: Arc::clone(&self.calls),
            gate: self.gate.clone(),
            started: Arc::clone(&self.started),
            internal_tools: self.internal_tools,
        })
    }
    async fn complete(
        &self,
        _: &[Message],
        tools: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<EventStream> {
        assert!(
            tools.iter().all(|tool| tool.name == "advise"),
            "advisor only receives explicitly granted tools"
        );
        assert!(!self.internal_tools, "unsafe provider must not be called");
        self.calls.lock().expect("calls").push((
            self.selected.lock().expect("model").clone(),
            self.effort.lock().expect("effort").clone(),
        ));
        self.started.notify_one();
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta(r#"{"severity":"concern","summary":"verify","evidence":[],"recommended_action":"run tests","blocking":false}"#.into())),
            Ok(StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) }),
        ])))
    }
}

fn route(model: &str, api: &str, available: bool) -> ModelRoute {
    ModelRoute {
        model: model.into(),
        provider: "OpenAI".into(),
        api_method: api.into(),
        available,
        detail: "catalog-only endpoint detail".into(),
        cheapness: None,
    }
}

fn choose(
    manager: &AdvisorManager,
    session: &str,
    provider: &CatalogProvider,
    effort: Option<&str>,
) -> Result<AdvisorModelSettings> {
    manager.select_model(
        session,
        provider,
        &AdvisorConfig::default(),
        provider.reviewer(),
        effort.map(str::to_string),
        manager.begin_model_selection(session),
    )
}

#[tokio::test]
async fn oauth_advisor_selection_needs_no_api_key_and_preserves_primary() {
    let provider = CatalogProvider::new();
    let manager = Arc::new(AdvisorManager::default());
    let config = AdvisorConfig::default();
    let options = manager
        .model_options("oauth", &provider, &config, None)
        .expect("catalog");
    assert!(options.selection.is_none());
    assert_eq!(options.available_routes.len(), 3);
    assert_eq!(options.available_selections.len(), 3);
    for (route, selection) in options
        .available_routes
        .iter()
        .zip(&options.available_selections)
    {
        assert_eq!(selection.model, route.model);
        assert_eq!(selection.runtime_key, RuntimeKey::OpenAIOAuth);
        assert!(selection.detail.is_empty());
        manager
            .model_options("oauth", &provider, &config, Some(selection))
            .expect("canonical catalog selection can be previewed unchanged");
    }
    assert!(
        options
            .available_routes
            .iter()
            .all(|route| route.api_method == "openai-oauth")
    );
    let options = manager
        .model_options("oauth", &provider, &config, Some(&provider.reviewer()))
        .expect("efforts");
    assert_eq!(options.available_efforts, ["low", "high"]);
    assert_eq!(options.reasoning_effort.as_deref(), Some("low"));
    let settings = choose(&manager, "oauth", &provider, Some("high")).expect("select OAuth");
    assert!(settings.enabled);
    assert!(!settings.follows_primary);
    assert_eq!(
        settings.selection.expect("selection").runtime_key,
        RuntimeKey::OpenAIOAuth
    );
    assert_eq!(
        manager
            .model_options("oauth", &provider, &config, Some(&provider.reviewer()))
            .expect("saved effort")
            .reasoning_effort
            .as_deref(),
        Some("high")
    );
    assert!(manager.schedule_turn(
        "oauth".into(),
        provider.fork(),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while manager.snapshot("oauth").expect("snapshot").status == AdvisorStatus::Reviewing {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("review completes");
    let calls = provider.calls.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.model, "reviewer");
    assert_eq!(calls[0].0.runtime_key, RuntimeKey::OpenAIOAuth);
    assert_eq!(calls[0].1, "high");
    assert_eq!(provider.model(), "coder");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("low"));
}

#[test]
fn model_and_effort_validation_fail_closed_without_changing_saved_choice() {
    let provider = CatalogProvider::new();
    let manager = AdvisorManager::default();
    let original = choose(&manager, "validation", &provider, Some("high")).expect("initial");
    for effort in ["swarm", "swarm-deep", "invalid"] {
        assert!(choose(&manager, "validation", &provider, Some(effort)).is_err());
        assert_eq!(
            manager.model_settings("validation", &provider, &AdvisorConfig::default()),
            original
        );
    }
    let forbidden = AdvisorConfig {
        allowed_runtime_keys: Some(vec!["openai-api-key".into()]),
        ..AdvisorConfig::default()
    };
    assert!(
        manager
            .select_model(
                "validation",
                &provider,
                &forbidden,
                provider.reviewer(),
                Some("high".into()),
                manager.begin_model_selection("validation")
            )
            .is_err()
    );
    let mut wrong_identity = provider.reviewer();
    wrong_identity.runtime_key = RuntimeKey::OpenAIApiKey;
    assert!(
        manager
            .select_model(
                "validation",
                &provider,
                &AdvisorConfig::default(),
                wrong_identity,
                None,
                manager.begin_model_selection("validation")
            )
            .is_err()
    );
    let plain = RouteSelection::from_model_route(&provider.routes[3]);
    assert!(
        manager
            .model_options(
                "validation",
                &provider,
                &AdvisorConfig::default(),
                Some(&plain)
            )
            .expect("non-reasoning model")
            .available_efforts
            .is_empty()
    );
    assert!(
        manager
            .select_model(
                "validation",
                &provider,
                &AdvisorConfig::default(),
                plain,
                Some("high".into()),
                manager.begin_model_selection("validation")
            )
            .is_err()
    );
    assert_eq!(
        manager.model_settings("validation", &provider, &AdvisorConfig::default()),
        original
    );
    assert!(provider.calls.lock().expect("calls").is_empty());
}

#[test]
fn selected_route_effort_and_primary_follow_survive_restart_and_history_changes() {
    let dir = tempfile::tempdir().expect("state directory");
    let provider = CatalogProvider::new();
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    let saved = choose(&manager, "resume", &provider, Some("high")).expect("select");
    let checkpoint = std::fs::read_to_string(
        std::fs::read_dir(dir.path())
            .expect("state files")
            .next()
            .expect("checkpoint")
            .expect("entry")
            .path(),
    )
    .expect("read checkpoint");
    assert!(!checkpoint.contains("catalog-only endpoint detail"));
    drop(manager);
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    manager.resume("resume");
    assert_eq!(
        manager.model_settings("resume", &provider, &AdvisorConfig::default()),
        saved
    );
    manager.reset_history("resume");
    assert_eq!(
        manager.model_settings("resume", &provider, &AdvisorConfig::default()),
        saved
    );
    let configured = AdvisorConfig {
        model: Some("reviewer".into()),
        ..AdvisorConfig::default()
    };
    let primary = manager
        .use_primary_model(
            "resume",
            &provider,
            &configured,
            manager.begin_model_selection("resume"),
        )
        .expect("follow primary");
    assert!(primary.follows_primary);
    assert_eq!(
        primary.selection.as_ref().expect("selection").model,
        "coder"
    );
    assert_eq!(primary.reasoning_effort.as_deref(), Some("low"));
    drop(manager);
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    manager.resume("resume");
    assert_eq!(
        manager.model_settings("resume", &provider, &configured),
        primary
    );
    assert!(
        provider.calls.lock().expect("calls").is_empty(),
        "restoring controls cannot replay reviews"
    );
}

#[tokio::test]
async fn choosing_a_model_cancels_active_and_pending_reviews_without_losing_notes() {
    let mut provider = CatalogProvider::new();
    let gate = Arc::new(tokio::sync::Notify::new());
    provider.gate = Some(Arc::clone(&gate));
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));
    manager.sessions.lock().expect("sessions").insert(
        "stale".into(),
        AdvisorRuntime {
            active_review_id: 42,
            status: AdvisorStatus::Reviewing,
            notes: VecDeque::from([AdvisorNoteMetadata {
                id: "adv-existing".into(),
                severity: AdvisorSeverity::Blocker,
                summary: "retained".into(),
                evidence: vec![],
                recommended_action: "verify".into(),
                blocking: true,
                disposition: AdvisorNoteDisposition::Unresolved,
            }]),
            pending: Some(PendingReview {
                provider: provider.fork(),
                queue: Arc::clone(&queue),
                input: AdvisorTurnInput::default(),
                config: AdvisorConfig::default(),
                model_override: None,
                context: AdvisorUpdateContext::default(),
                cancellation: tokio_util::sync::CancellationToken::new(),
            }),
            ..AdvisorRuntime::default()
        },
    );
    let running = Arc::clone(&manager);
    let pending = PendingReview {
        provider: provider.fork(),
        queue: Arc::clone(&queue),
        input: AdvisorTurnInput::default(),
        config: AdvisorConfig::default(),
        model_override: None,
        context: AdvisorUpdateContext::default(),
        cancellation: tokio_util::sync::CancellationToken::new(),
    };
    let task = tokio::spawn(async move { running.run_review("stale".into(), 42, pending).await });
    provider.started.notified().await;
    choose(&manager, "stale", &provider, Some("high")).expect("new selection");
    gate.notify_one();
    task.await.expect("old review ends");
    assert!(
        manager
            .sessions
            .lock()
            .expect("sessions")
            .get("stale")
            .expect("state")
            .pending
            .is_none()
    );
    assert_eq!(manager.notes("stale").len(), 1);
    assert_eq!(manager.notes("stale")[0].id, "adv-existing");
    assert!(queue.lock().expect("queue").is_empty());
}

#[test]
fn later_disable_supersedes_a_deferred_model_selection() {
    let provider = CatalogProvider::new();
    let manager = AdvisorManager::default();
    let request_id = manager.begin_model_selection("deferred");
    manager.set_enabled("deferred", false).expect("disable");
    let error = manager
        .select_model(
            "deferred",
            &provider,
            &AdvisorConfig::default(),
            provider.reviewer(),
            Some("high".into()),
            request_id,
        )
        .expect_err("stale selection");
    assert!(error.to_string().contains("superseded"));
    assert!(!manager.is_enabled("deferred", true));
}

#[test]
fn following_primary_rejects_inherited_swarm_efforts() {
    let provider = CatalogProvider::new();
    *provider.effort.lock().expect("effort") = "swarm".into();
    let manager = AdvisorManager::default();
    assert!(
        manager
            .use_primary_model(
                "swarm",
                &provider,
                &AdvisorConfig::default(),
                manager.begin_model_selection("swarm")
            )
            .is_err()
    );
    assert!(!manager.is_enabled("swarm", false));
    choose(&manager, "swarm", &provider, Some("high")).expect("explicit single-model effort");
}

#[test]
fn jcode_subscription_selection_retains_structured_runtime_identity() {
    let mut provider = CatalogProvider::new();
    provider.routes.push(ModelRoute {
        model: "reviewer".into(),
        provider: "Jcode subscription".into(),
        api_method: "jcode-subscription".into(),
        available: true,
        detail: String::new(),
        cheapness: None,
    });
    let selection =
        RouteSelection::from_model_route(provider.routes.last().expect("subscription route"));
    assert_eq!(selection.runtime_key, RuntimeKey::JcodeSubscription);
    let manager = AdvisorManager::default();
    let saved = manager
        .select_model(
            "subscription",
            &provider,
            &AdvisorConfig::default(),
            selection,
            Some("high".into()),
            manager.begin_model_selection("subscription"),
        )
        .expect("subscription selection");
    assert_eq!(
        saved.selection.expect("saved route").runtime_key,
        RuntimeKey::JcodeSubscription
    );
    assert_eq!(
        provider.selected.lock().expect("primary").runtime_key,
        RuntimeKey::OpenAIOAuth
    );
}

#[test]
fn advisor_catalog_skips_unrepresentable_runtime_keys_without_losing_valid_routes() {
    let mut provider = CatalogProvider::new();
    let legacy = route("legacy-reviewer", "grok-acp", true);
    let selection = RouteSelection::from_model_route(&legacy);
    assert!(matches!(&selection.runtime_key, RuntimeKey::Other(_)));
    assert!(serde_json::to_string(&selection).is_err());
    provider.routes.push(legacy);
    *provider.selected.lock().expect("primary route") = selection.clone();
    let manager = AdvisorManager::default();
    let config = AdvisorConfig::default();
    let options = manager
        .model_options("legacy", &provider, &config, None)
        .expect("valid catalog remains available");
    assert_eq!(options.available_routes.len(), 3);
    assert_eq!(options.available_selections.len(), 3);
    assert!(
        options
            .available_routes
            .iter()
            .all(|route| route.api_method != "grok-acp")
    );
    let settings = manager.model_settings("legacy", &provider, &config);
    assert!(settings.selection.is_none());
    serde_json::to_string(&crate::protocol::AdvisorControlResult {
        model_options: Some(options),
        model_settings: Some(settings),
        ..crate::protocol::AdvisorControlResult::default()
    })
    .expect("whole response is serializable");
    assert!(
        manager
            .model_options("legacy", &provider, &config, Some(&selection))
            .is_err()
    );
    assert!(provider.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn advisor_rejects_providers_that_cannot_disable_internal_tools() {
    let mut provider = CatalogProvider::new();
    let manager = Arc::new(AdvisorManager::default());
    let config = AdvisorConfig::default();
    choose(&manager, "unsafe", &provider, Some("high")).expect("initial safe selection");
    provider.internal_tools = true;

    let preview = manager
        .model_options("unsafe", &provider, &config, Some(&provider.reviewer()))
        .expect_err("unsafe preview must fail");
    assert!(
        preview
            .to_string()
            .contains("cannot disable its built-in tools")
    );
    assert!(choose(&manager, "unsafe", &provider, Some("high")).is_err());
    assert!(
        manager
            .use_primary_model(
                "unsafe",
                &provider,
                &config,
                manager.begin_model_selection("unsafe")
            )
            .is_err()
    );

    // Re-check the live runtime before each review even if a previously saved
    // route was safe when it was selected.
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "unsafe".into(),
        provider.fork(),
        Arc::clone(&queue),
        AdvisorTurnInput::default(),
        config
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while manager.snapshot("unsafe").expect("snapshot").status == AdvisorStatus::Reviewing {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("unsafe review rejected");
    let snapshot = manager.snapshot("unsafe").expect("failed state");
    assert_eq!(snapshot.status, AdvisorStatus::Failed);
    assert!(
        snapshot
            .last_error
            .expect("capability error")
            .contains("cannot disable its built-in tools")
    );
    assert!(provider.calls.lock().expect("calls").is_empty());
    assert!(manager.notes("unsafe").is_empty());
    assert!(queue.lock().expect("queue").is_empty());
}
