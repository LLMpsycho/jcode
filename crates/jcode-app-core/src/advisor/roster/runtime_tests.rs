use super::*;
use crate::message::{Message, StreamEvent, ToolDefinition};
use crate::provider::{EventStream, ModelRoute, RouteSelection};
use async_trait::async_trait;
use std::sync::Mutex;

struct RecordedCall {
    model: String,
    effort: String,
    history: String,
    system: String,
}

struct RosterProvider {
    model: Mutex<String>,
    effort: Mutex<String>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

#[async_trait]
impl Provider for RosterProvider {
    fn name(&self) -> &str {
        "openai"
    }
    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }
    fn model_routes(&self) -> Vec<ModelRoute> {
        ["primary", "security-model", "verification-model"]
            .into_iter()
            .map(|model| ModelRoute {
                model: model.into(),
                provider: "OpenAI".into(),
                api_method: "openai-oauth".into(),
                available: true,
                detail: String::new(),
                cheapness: None,
            })
            .collect()
    }
    fn set_route_selection(&self, selection: &RouteSelection) -> Result<()> {
        *self.model.lock().unwrap() = selection.model.clone();
        Ok(())
    }
    fn available_efforts(&self) -> Vec<&'static str> {
        vec!["low", "high"]
    }
    fn reasoning_effort(&self) -> Option<String> {
        Some(self.effort.lock().unwrap().clone())
    }
    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        *self.effort.lock().unwrap() = effort.into();
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: Mutex::new(self.model()),
            effort: Mutex::new(self.effort.lock().unwrap().clone()),
            calls: Arc::clone(&self.calls),
        })
    }
    async fn complete(
        &self,
        messages: &[Message],
        _: &[ToolDefinition],
        system: &str,
        _: Option<&str>,
    ) -> Result<EventStream> {
        let model = self.model();
        self.calls.lock().unwrap().push(RecordedCall {
            model: model.clone(),
            effort: self.effort.lock().unwrap().clone(),
            history: serde_json::to_string(messages)?,
            system: system.into(),
        });
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(StreamEvent::TextDelta(format!(
                "{{\"silence\":true,\"private_marker\":\"{model}-private\"}}"
            ))),
            Ok(StreamEvent::MessageEnd {
                stop_reason: Some("end_turn".into()),
            }),
        ])))
    }
}

#[tokio::test]
async fn named_advisors_have_independent_models_efforts_histories_and_controls() {
    let provider = Arc::new(RosterProvider {
        model: Mutex::new("primary".into()),
        effort: Mutex::new("low".into()),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));
    let config = AdvisorConfig {
        enabled: true,
        allowed_runtime_keys: Some(vec!["openai-oauth".into()]),
        roster: ["security", "verification"]
            .into_iter()
            .map(|name| AdvisorRosterEntry {
                name: name.into(),
                model: Some(format!("{name}-model")),
                effort: Some("high".into()),
                instructions: Some(format!("Specialization {name}")),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    for turn in 1..=2 {
        assert!(schedule_updates(
            &manager,
            "roster-test".into(),
            provider.clone(),
            queue.clone(),
            AdvisorTurnInput {
                objective: "Implement requirement".into(),
                latest_primary_turn: format!("visible step {turn}"),
                ..Default::default()
            },
            config.clone(),
            AdvisorUpdateContext {
                primary_turn_id: turn,
                completed_primary_turn: true,
                instructions: "Project bootstrap".into(),
                ..Default::default()
            }
        ));
        assert!(
            manager
                .wait_for_idle("roster-test", std::time::Duration::from_secs(2))
                .await
        );
    }
    assert_eq!(provider.model(), "primary");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("low"));
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    for name in ["security", "verification"] {
        let own = calls
            .iter()
            .filter(|call| call.model == format!("{name}-model"))
            .collect::<Vec<_>>();
        assert_eq!(own.len(), 2);
        assert_eq!(own[1].effort, "high");
        assert!(own[1].system.contains("Project bootstrap"));
        assert!(own[1].system.contains(&format!("Specialization {name}")));
        assert!(own[1].history.contains(&format!("{name}-model-private")));
        let other = if name == "security" {
            "verification"
        } else {
            "security"
        };
        assert!(!own[1].history.contains(&format!("{other}-model-private")));
    }
    drop(calls);
    let key = runtime_session_key("roster-test", "security");
    manager.set_enabled(&key, false).unwrap();
    assert!(is_enabled(&manager, "roster-test", &config, None));
    disable_all(&manager, "roster-test", &config).unwrap();
    assert!(!is_enabled(&manager, "roster-test", &config, None));
    let future = AdvisorConfig {
        roster: vec![AdvisorRosterEntry {
            name: "new-reviewer".into(),
            ..Default::default()
        }],
        ..config
    };
    assert!(
        !is_enabled(&manager, "roster-test", &future, None),
        "global off also covers future entries"
    );
    assert!(
        queue.lock().unwrap().is_empty(),
        "healthy investigations remain silent"
    );
}

#[test]
fn named_model_effort_and_disable_resume_without_enabling_siblings_or_replaying() {
    let dir = tempfile::tempdir().unwrap();
    let provider = RosterProvider {
        model: Mutex::new("primary".into()),
        effort: Mutex::new("low".into()),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let config = AdvisorConfig {
        enabled: false,
        allowed_runtime_keys: Some(vec!["openai-oauth".into()]),
        roster: ["security", "verification"]
            .into_iter()
            .map(|name| AdvisorRosterEntry {
                name: name.into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    let security = runtime_session_key("resume-named", "security");
    let verification = runtime_session_key("resume-named", "verification");
    let selection = RouteSelection::from_model_route(&provider.model_routes()[1]);
    let generation = manager.begin_model_selection(&security);
    manager
        .select_model(
            &security,
            &provider,
            &config,
            selection.clone(),
            Some("high".into()),
            generation,
        )
        .unwrap();
    drop(manager);
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    assert!(is_enabled(&manager, "resume-named", &config, None));
    assert!(!manager.is_enabled(&verification, false));
    let settings = manager.saved_model_settings(&security, &config);
    assert_eq!(settings.selection, Some(selection));
    assert_eq!(settings.reasoning_effort.as_deref(), Some("high"));
    assert!(settings.enabled);
    assert_eq!(manager.snapshot(&security).unwrap().history_messages, 0);
    assert!(
        provider.calls.lock().unwrap().is_empty(),
        "restart never replays a provider request"
    );
    disable_all(&manager, "resume-named", &config).unwrap();
    drop(manager);
    let manager = AdvisorManager::persistent(dir.path().to_path_buf());
    assert!(!is_enabled(&manager, "resume-named", &config, None));
    let future = AdvisorConfig {
        enabled: true,
        roster: vec![AdvisorRosterEntry {
            name: "new".into(),
            ..Default::default()
        }],
        ..config
    };
    assert!(
        !is_enabled(&manager, "resume-named", &future, None),
        "global off survives restart and covers future roster entries"
    );
    assert_eq!(provider.model(), "primary");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("low"));
}
