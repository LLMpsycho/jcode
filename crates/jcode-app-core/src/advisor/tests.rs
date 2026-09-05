use super::*;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn handled_note_immunity_survives_restart_and_prevents_paraphrase_storms() {
    let dir = tempfile::tempdir().expect("directory");
    let manager = Arc::new(AdvisorManager::persistent(dir.path().to_path_buf()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(AdvisorProvider {
        calls: Arc::clone(&calls),
        response: r#"{"severity":"concern","summary":"verify first","evidence":[],"recommended_action":"run checks","blocking":false}"#.into(),
    });
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "immunity".into(),
        provider.clone(),
        queue.clone(),
        AdvisorTurnInput::default(),
        enabled_config()
    ));
    wait_for_status(&manager, "immunity", AdvisorStatus::Ready).await;
    let id = manager.notes("immunity")[0].id.clone();
    manager
        .resolve_note("immunity", &id, AdvisorNoteDisposition::Acknowledged)
        .expect("ack");
    assert!(
        queue.lock().expect("queue").is_empty(),
        "handled queued notes must not reappear"
    );
    drop(manager);
    let manager = Arc::new(AdvisorManager::persistent(dir.path().to_path_buf()));
    manager.resume("immunity");
    // No provider call means paraphrased or alternating findings cannot defeat
    // suppression. A duplicate ack does not extend the window indefinitely.
    for _ in 0..2 {
        assert!(!manager.schedule_turn(
            "immunity".into(),
            provider.clone(),
            queue.clone(),
            AdvisorTurnInput::default(),
            enabled_config()
        ));
        manager
            .resolve_note("immunity", &id, AdvisorNoteDisposition::Acknowledged)
            .expect("duplicate ack");
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(manager.schedule_turn(
        "immunity".into(),
        provider,
        queue,
        AdvisorTurnInput::default(),
        enabled_config()
    ));
    wait_for_status(&manager, "immunity", AdvisorStatus::Ready).await;
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

struct AdvisorProvider {
    calls: Arc<AtomicUsize>,
    response: String,
}

struct ModeCaptureProvider {
    systems: Arc<Mutex<Vec<String>>>,
}

struct PrematureAdvisorProvider {
    emit_error: bool,
}

#[async_trait]
impl Provider for PrematureAdvisorProvider {
    fn name(&self) -> &str {
        "premature-advisor"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            emit_error: self.emit_error,
        })
    }

    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        let mut events = vec![Ok(StreamEvent::TextDelta(
            r#"{"severity":"blocker","summary":"uncommitted","evidence":[],"recommended_action":"stop","blocking":true}"#.into(),
        ))];
        if self.emit_error {
            events.push(Ok(StreamEvent::Error {
                message: "provider failed OPENAI_API_KEY=fixture-private-value".into(),
                retry_after_secs: None,
            }));
        }
        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn advisor_error_events_and_premature_eof_never_publish_partial_notes() {
    for emit_error in [false, true] {
        let manager = Arc::new(AdvisorManager::default());
        let queue = Arc::new(Mutex::new(Vec::new()));
        assert!(manager.schedule_turn(
            "premature".into(),
            Arc::new(PrematureAdvisorProvider { emit_error }),
            queue.clone(),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));
        wait_for_status(&manager, "premature", AdvisorStatus::Failed).await;
        assert!(manager.notes("premature").is_empty());
        assert!(queue.lock().expect("queue").is_empty());
        let error = manager
            .snapshot("premature")
            .expect("state")
            .last_error
            .expect("error");
        assert!(!error.contains("fixture-private-value"));
    }
}

#[async_trait]
impl Provider for AdvisorProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        assert!(tools.is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta(self.response.clone())),
            Ok(StreamEvent::MessageEnd {
                stop_reason: Some("end_turn".to_string()),
            }),
        ])))
    }

    fn name(&self) -> &str {
        "advisor-test"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            calls: Arc::clone(&self.calls),
            response: self.response.clone(),
        })
    }
}

#[async_trait]
impl Provider for ModeCaptureProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        tools: &[crate::message::ToolDefinition],
        system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        assert!(tools.is_empty());
        self.systems
            .lock()
            .expect("systems")
            .push(system.to_string());
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta(
                r#"{"severity":"nit","summary":"ok","evidence":["bounded acceptance"],"recommended_action":"continue","blocking":false}"#.to_string(),
            )),
            Ok(StreamEvent::MessageEnd {
                stop_reason: Some("end_turn".to_string()),
            }),
        ])))
    }

    fn name(&self) -> &str {
        "advisor-mode-capture"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            systems: Arc::clone(&self.systems),
        })
    }
}

struct FailingAdvisorProvider;

struct BlockingAdvisorProvider {
    release: Arc<tokio::sync::Notify>,
    response: String,
}

#[async_trait]
impl Provider for BlockingAdvisorProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        self.release.notified().await;
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta(self.response.clone())),
            Ok(StreamEvent::MessageEnd {
                stop_reason: Some("end_turn".to_string()),
            }),
        ])))
    }

    fn name(&self) -> &str {
        "advisor-blocking-test"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            release: Arc::clone(&self.release),
            response: self.response.clone(),
        })
    }
}

#[async_trait]
impl Provider for FailingAdvisorProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        anyhow::bail!(
            "request rejected\nAuthorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789\nOPENAI_API_KEY=sk-test-openai-example"
        )
    }

    fn name(&self) -> &str {
        "advisor-failure-test"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self)
    }
}

fn enabled_config() -> AdvisorConfig {
    AdvisorConfig {
        enabled: true,
        ..AdvisorConfig::default()
    }
}

#[tokio::test]
async fn disabled_advisor_has_no_runtime_or_provider_cost() {
    let manager = Arc::new(AdvisorManager::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(Vec::new()));
    let scheduled = manager.schedule_turn(
        "disabled".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::clone(&calls),
            response: String::new(),
        }),
        queue,
        AdvisorTurnInput::default(),
        AdvisorConfig::default(),
    );
    assert!(!scheduled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(manager.snapshot("disabled").is_none());
}

#[tokio::test]
async fn review_advances_once_redacts_and_delivers_structured_note() {
    let manager = Arc::new(AdvisorManager::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(Vec::new()));
    let scheduled = manager.schedule_turn(
        "session".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::clone(&calls),
            response: r#"{"severity":"concern","summary":"Acceptance drift","evidence":["OPENAI_API_KEY=sk-test-openai-example"],"recommended_action":"Run the public flow","blocking":true}"#.to_string(),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput {
            objective: "OPENAI_API_KEY=sk-test-openai-example".to_string(),
            ..AdvisorTurnInput::default()
        },
        enabled_config(),
    );
    assert!(scheduled);

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if manager
                .snapshot("session")
                .is_some_and(|s| s.status == AdvisorStatus::Ready)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("advisor should finish");

    let snapshot = manager.snapshot("session").expect("runtime");
    assert_eq!(snapshot.cursor, 1);
    assert_eq!(snapshot.private_context_len, 1);
    assert_eq!(snapshot.notes_emitted, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let pending = queue.lock().expect("queue");
    assert_eq!(pending.len(), 1);
    assert!(pending[0].content.contains("Acceptance drift"));
    assert!(pending[0].content.contains("[REDACTED_SECRET]"));
    assert!(
        !pending[0].urgent,
        "concern is below the default blocker threshold"
    );
}

async fn wait_for_status(manager: &AdvisorManager, session: &str, expected: AdvisorStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if manager
                .snapshot(session)
                .is_some_and(|snapshot| snapshot.status == expected)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("advisor should reach expected status");
}

#[tokio::test]
async fn duplicate_notes_are_delivered_only_once_across_turns() {
    let manager = Arc::new(AdvisorManager::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn Provider> = Arc::new(AdvisorProvider {
        calls: Arc::clone(&calls),
        response: r#"{"severity":"nit","summary":"Same note","evidence":[],"recommended_action":"Keep going","blocking":false}"#.to_string(),
    });

    for expected_cursor in 1..=2 {
        assert!(manager.schedule_turn(
            "dedupe".to_string(),
            Arc::clone(&provider),
            Arc::clone(&queue),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));
        wait_for_status(&manager, "dedupe", AdvisorStatus::Ready).await;
        assert_eq!(
            manager.snapshot("dedupe").expect("runtime").cursor,
            expected_cursor
        );
    }

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(queue.lock().expect("queue").len(), 1);
}

#[tokio::test]
async fn review_finishing_while_another_turn_completes_runs_latest_pending_review() {
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    assert!(manager.schedule_turn(
        "coalesce".to_string(),
        Arc::new(BlockingAdvisorProvider {
            release: Arc::clone(&release),
            response: r#"{"severity":"nit","summary":"first","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput {
            objective: "first".to_string(),
            ..AdvisorTurnInput::default()
        },
        enabled_config(),
    ));
    assert!(manager.schedule_turn(
        "coalesce".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            response: r#"{"severity":"concern","summary":"latest","evidence":[],"recommended_action":"review latest","blocking":false}"#.to_string(),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput {
            objective: "latest".to_string(),
            ..AdvisorTurnInput::default()
        },
        enabled_config(),
    ));

    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if manager.snapshot("coalesce").is_some_and(|snapshot| {
                snapshot.cursor == 2 && snapshot.status == AdvisorStatus::Ready
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pending review should run after the active review");

    let pending = queue.lock().expect("queue");
    assert_eq!(pending.len(), 2);
    assert!(pending[1].content.contains("latest"));
}

#[tokio::test]
async fn malformed_advisor_response_fails_without_interrupting_primary_session() {
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "failure".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            response: "not-json".to_string(),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput::default(),
        enabled_config(),
    ));

    wait_for_status(&manager, "failure", AdvisorStatus::Failed).await;
    let snapshot = manager.snapshot("failure").expect("runtime");
    assert_eq!(snapshot.cursor, 1);
    assert!(snapshot.last_error.is_some());
    assert!(queue.lock().expect("queue").is_empty());
}

#[tokio::test]
async fn provider_failure_is_redacted_before_runtime_storage() {
    let manager = Arc::new(AdvisorManager::default());
    assert!(manager.schedule_turn(
        "redacted-failure".to_string(),
        Arc::new(FailingAdvisorProvider),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        enabled_config(),
    ));

    wait_for_status(&manager, "redacted-failure", AdvisorStatus::Failed).await;
    let error = manager
        .snapshot("redacted-failure")
        .and_then(|snapshot| snapshot.last_error)
        .expect("failure should be stored");
    assert!(error.contains("[REDACTED_SECRET]"));
    assert!(!error.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
    assert!(!error.contains("sk-test-openai-example"));
    assert!(!error.contains('\n'));
}

#[tokio::test]
async fn stale_review_completion_cannot_publish_into_recreated_session() {
    let manager = AdvisorManager::default();
    manager.sessions.lock().expect("sessions").insert(
        "recreated".to_string(),
        AdvisorRuntime {
            cursor: 1,
            status: AdvisorStatus::Reviewing,
            active_review_id: 2,
            ..AdvisorRuntime::default()
        },
    );
    let queue = Arc::new(Mutex::new(Vec::new()));

    manager
        .run_review(
            "recreated".to_string(),
            1,
            PendingReview {
            provider: Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: r#"{"severity":"blocker","summary":"Stale","evidence":[],"recommended_action":"Do not publish","blocking":true}"#.to_string(),
            }),
            queue: Arc::clone(&queue),
            input: AdvisorTurnInput::default(),
            config: enabled_config(),
            model_override: None,
            },
        )
        .await;

    let snapshot = manager.snapshot("recreated").expect("runtime");
    assert_eq!(snapshot.status, AdvisorStatus::Reviewing);
    assert_eq!(snapshot.notes_emitted, 0);
    assert!(queue.lock().expect("queue").is_empty());
}

#[tokio::test]
async fn oversized_response_fails_with_bounded_error_and_no_note() {
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "oversized".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            response: "x".repeat(MAX_INPUT_BYTES + 1),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput::default(),
        enabled_config(),
    ));

    wait_for_status(&manager, "oversized", AdvisorStatus::Failed).await;
    let snapshot = manager.snapshot("oversized").expect("runtime");
    assert!(snapshot.last_error.expect("bounded error").len() <= 1000);
    assert!(queue.lock().expect("queue").is_empty());
}

#[tokio::test]
async fn private_context_retains_only_the_bounded_recent_window() {
    let manager = Arc::new(AdvisorManager::default());
    let queue = Arc::new(Mutex::new(Vec::new()));

    for index in 0..MAX_PRIVATE_CONTEXT + 2 {
        assert!(manager.schedule_turn(
            "context".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: format!(
                    r#"{{"severity":"nit","summary":"note-{index}","evidence":[],"recommended_action":"continue","blocking":false}}"#
                ),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput {
                objective: format!("turn-{index}"),
                ..AdvisorTurnInput::default()
            },
            enabled_config(),
        ));
        wait_for_status(&manager, "context", AdvisorStatus::Ready).await;
    }

    let snapshot = manager.snapshot("context").expect("runtime");
    assert_eq!(snapshot.cursor as usize, MAX_PRIVATE_CONTEXT + 2);
    assert_eq!(snapshot.private_context_len, MAX_PRIVATE_CONTEXT);
}

#[tokio::test]
async fn review_cadence_and_session_budget_bound_provider_calls() {
    let manager = Arc::new(AdvisorManager::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(AdvisorProvider {
        calls: Arc::clone(&calls),
        response: r#"{"severity":"nit","summary":"ok","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
    });
    let config = AdvisorConfig {
        enabled: true,
        review_every_n_turns: 2,
        max_reviews_per_session: 2,
        ..AdvisorConfig::default()
    };

    assert!(manager.schedule_turn(
        "budgeted".to_string(),
        Arc::clone(&provider),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config.clone(),
    ));
    wait_for_status(&manager, "budgeted", AdvisorStatus::Ready).await;
    assert!(!manager.schedule_turn(
        "budgeted".to_string(),
        Arc::clone(&provider),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config.clone(),
    ));
    assert!(manager.schedule_turn(
        "budgeted".to_string(),
        Arc::clone(&provider),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config.clone(),
    ));
    wait_for_status(&manager, "budgeted", AdvisorStatus::Ready).await;
    assert!(!manager.schedule_turn(
        "budgeted".to_string(),
        Arc::clone(&provider),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config.clone(),
    ));
    assert!(!manager.schedule_turn(
        "budgeted".to_string(),
        provider,
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config,
    ));

    let snapshot = manager.snapshot("budgeted").expect("runtime");
    assert_eq!(snapshot.turns_observed, 5);
    assert_eq!(snapshot.cursor, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn zero_session_budget_has_no_runtime_or_provider_cost() {
    let manager = Arc::new(AdvisorManager::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let config = AdvisorConfig {
        enabled: true,
        max_reviews_per_session: 0,
        ..AdvisorConfig::default()
    };
    assert!(!manager.schedule_turn(
        "zero-budget".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::clone(&calls),
            response: String::new(),
        }),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        config,
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(manager.snapshot("zero-budget").is_none());
}

#[test]
fn advisor_modes_have_distinct_toolless_evidence_contracts() {
    let interactive = advisor_system_prompt(AdvisorMode::Interactive);
    let guardian = advisor_system_prompt(AdvisorMode::SelfdevGuardian);
    let final_review = advisor_system_prompt(AdvisorMode::FinalReview);

    for prompt in [&interactive, &guardian, &final_review] {
        assert!(prompt.contains("You have no tools"));
        assert!(prompt.contains("Return exactly one JSON object"));
        assert!(prompt.contains("Do not include markdown or hidden reasoning"));
    }
    assert!(interactive.contains("materially helps the user"));
    assert!(guardian.contains("strictly read-only"));
    assert!(guardian.contains("benchmark validity"));
    assert!(final_review.contains("independent evidence-referencing verdict"));
    assert!(final_review.contains("do not infer success from implementation alone"));
    assert_ne!(interactive, guardian);
    assert_ne!(guardian, final_review);
}

#[tokio::test]
async fn configured_mode_contract_reaches_the_forked_provider() {
    let manager = Arc::new(AdvisorManager::default());
    let systems = Arc::new(Mutex::new(Vec::new()));
    for (index, mode) in [
        AdvisorMode::Interactive,
        AdvisorMode::SelfdevGuardian,
        AdvisorMode::FinalReview,
    ]
    .into_iter()
    .enumerate()
    {
        let session_id = format!("mode-{index}");
        assert!(manager.schedule_turn(
            session_id.clone(),
            Arc::new(ModeCaptureProvider {
                systems: Arc::clone(&systems),
            }),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput {
                objective: "bounded acceptance".into(),
                ..AdvisorTurnInput::default()
            },
            AdvisorConfig {
                enabled: true,
                mode,
                ..AdvisorConfig::default()
            },
        ));
        wait_for_status(&manager, &session_id, AdvisorStatus::Ready).await;
    }

    let systems = systems.lock().expect("systems");
    assert_eq!(systems.len(), 3);
    assert!(systems[0].contains("Interactive mode"));
    assert!(systems[1].contains("Self-development guardian mode"));
    assert!(systems[2].contains("Final-review mode"));
}

#[test]
fn turn_input_is_bounded_and_redacted() {
    let input = AdvisorTurnInput {
        objective: format!(
            "OPENAI_API_KEY=sk-test-openai-example {}",
            "x".repeat(MAX_FIELD_BYTES * 2)
        ),
        tools: (0..MAX_TOOLS + 5)
            .map(|index| AdvisorToolInput {
                name: format!("tool-{index}"),
                intent: Some(format!(
                    "OPENAI_API_KEY=sk-test-openai-example {}",
                    "i".repeat(MAX_FIELD_BYTES * 2)
                )),
                result: "y".repeat(MAX_FIELD_BYTES * 2),
            })
            .collect(),
        ..AdvisorTurnInput::default()
    }
    .bounded(true);
    let encoded = serde_json::to_vec(&input).expect("serialize");
    assert!(encoded.len() <= MAX_INPUT_BYTES);
    assert!(input.objective.contains("[REDACTED_SECRET]"));
    assert!(input.tools.len() <= MAX_TOOLS);
    assert!(
        input.tools[0]
            .intent
            .as_deref()
            .is_some_and(|intent| intent.contains("[REDACTED_SECRET]"))
    );
}

#[test]
fn escaped_fields_cannot_exceed_total_input_budget() {
    let escaped = "\0".repeat(MAX_FIELD_BYTES);
    let input = AdvisorTurnInput {
        objective: escaped.clone(),
        latest_primary_turn: escaped.clone(),
        diff_summary: escaped.clone(),
        diagnostics: escaped.clone(),
        verification_status: escaped.clone(),
        outstanding_todos: escaped.clone(),
        acceptance_criteria: escaped,
        ..AdvisorTurnInput::default()
    }
    .bounded(false);

    assert!(serde_json::to_vec(&input).expect("serialize").len() <= MAX_INPUT_BYTES);
}

#[tokio::test]
async fn unresolved_blocker_gates_only_future_risky_tools_until_handled_or_disabled() {
    let manager = Arc::new(AdvisorManager::default());
    manager
        .set_enabled("gating", true)
        .expect("save advisor control");
    assert!(manager.schedule_turn(
        "gating".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            response: r#"{"severity":"blocker","summary":"unsafe publication","evidence":["bounded"],"recommended_action":"verify first","blocking":false}"#.to_string(),
        }),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        enabled_config(),
    ));
    wait_for_status(&manager, "gating", AdvisorStatus::Ready).await;

    let notes = manager.notes("gating");
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].blocking,
        "severity, not model boolean, controls gating"
    );
    use crate::tool::ToolCapability;
    assert!(
        manager
            .blocks_tool_call("gating", "read", ToolCapability::ReadOnly)
            .is_none()
    );
    assert!(
        manager
            .blocks_tool_call("gating", "bash", ToolCapability::Execute)
            .is_some()
    );

    assert!(
        manager
            .resolve_note("gating", &notes[0].id, AdvisorNoteDisposition::Acknowledged,)
            .expect("ack")
    );
    assert!(
        manager
            .blocks_tool_call("gating", "bash", ToolCapability::Execute)
            .is_none()
    );

    manager
        .resolve_note("gating", &notes[0].id, AdvisorNoteDisposition::Unresolved)
        .expect("unresolve");
    manager
        .set_enabled("gating", false)
        .expect("save advisor control");
    assert!(
        manager
            .blocks_tool_call("gating", "write", ToolCapability::WriteFiles)
            .is_none()
    );
}

#[tokio::test]
async fn enable_override_activates_globally_disabled_advisor() {
    let manager = Arc::new(AdvisorManager::default());
    manager
        .set_enabled("enabled-override", true)
        .expect("save advisor control");
    assert!(manager.is_enabled("enabled-override", false));

    assert!(manager.schedule_turn(
        "enabled-override".to_string(),
        Arc::new(AdvisorProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            response: r#"{"severity":"nit","summary":"ok","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
        }),
        Arc::new(Mutex::new(Vec::new())),
        AdvisorTurnInput::default(),
        AdvisorConfig::default(),
    ));
    wait_for_status(&manager, "enabled-override", AdvisorStatus::Ready).await;
}

#[tokio::test]
async fn disabling_fences_an_in_flight_review_and_discards_its_note() {
    let manager = Arc::new(AdvisorManager::default());
    let release = Arc::new(tokio::sync::Notify::new());
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "disable-in-flight".to_string(),
        Arc::new(BlockingAdvisorProvider {
            release: Arc::clone(&release),
            response: r#"{"severity":"blocker","summary":"late","evidence":[],"recommended_action":"stop","blocking":true}"#.to_string(),
        }),
        Arc::clone(&queue),
        AdvisorTurnInput::default(),
        enabled_config(),
    ));
    wait_for_status(&manager, "disable-in-flight", AdvisorStatus::Reviewing).await;

    manager
        .set_enabled("disable-in-flight", false)
        .expect("save advisor control");
    release.notify_waiters();
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let snapshot = manager.snapshot("disable-in-flight").expect("snapshot");
    assert_eq!(snapshot.status, AdvisorStatus::Idle);
    assert!(!manager.is_enabled("disable-in-flight", true));
    assert!(manager.notes("disable-in-flight").is_empty());
    assert!(queue.lock().expect("queue").is_empty());
}
