use super::*;
use crate::message::ToolDefinition;
use async_trait::async_trait;
use futures::stream;
use serde_json::json;

#[derive(Clone)]
struct Scripted {
    replies: Arc<Mutex<VecDeque<Vec<StreamEvent>>>>,
    received: Arc<Mutex<Vec<Vec<Message>>>>,
    model: Arc<Mutex<String>>,
    provider_name: &'static str,
}

impl Scripted {
    fn new(replies: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies.into())),
            received: Arc::new(Mutex::new(Vec::new())),
            model: Arc::new(Mutex::new("model-a".into())),
            provider_name: "advisor-script",
        }
    }
}

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        self.provider_name
    }
    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> anyhow::Result<crate::provider::EventStream> {
        assert!(tools.iter().any(|tool| tool.name == "advise"));
        self.received.lock().unwrap().push(messages.to_vec());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("script exhausted");
        Ok(Box::pin(stream::iter(reply.into_iter().map(Ok))))
    }
}

fn text(value: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(value.into()),
        StreamEvent::MessageEnd {
            stop_reason: Some("end_turn".into()),
        },
    ]
}

fn tool(name: &str, input: serde_json::Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolUseStart {
            id: format!("call-{name}"),
            name: name.into(),
        },
        StreamEvent::ToolInputDelta(input.to_string()),
        StreamEvent::ToolUseEnd,
        StreamEvent::MessageEnd {
            stop_reason: Some("tool_use".into()),
        },
    ]
}

fn advice(key: &str, severity: &str, summary: &str) -> Vec<StreamEvent> {
    tool(
        "advise",
        json!({"concern_id":key,"severity":severity,"summary":summary,"evidence":["observed evidence"],"recommended_action":"Verify the implicated behavior"}),
    )
}

async fn run(
    manager: &Arc<AdvisorManager>,
    provider: &Scripted,
    queue: &SoftInterruptQueue,
    objective: &str,
) {
    assert!(manager.schedule_turn(
        "live".into(),
        provider.fork(),
        queue.clone(),
        AdvisorTurnInput {
            objective: objective.into(),
            ..Default::default()
        },
        AdvisorConfig {
            enabled: true,
            ..Default::default()
        }
    ));
    assert!(
        manager
            .wait_for_idle("live", std::time::Duration::from_secs(2))
            .await
    );
    assert_eq!(
        manager.snapshot("live").unwrap().status,
        AdvisorStatus::Ready
    );
}

#[tokio::test]
async fn silence_and_private_commentary_keep_real_history_without_publishing() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        text("The approach is currently sound."),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Preserve tenant authorization").await;
    run(&manager, &provider, &queue, "Continue").await;
    assert!(manager.notes("live").is_empty());
    assert!(queue.lock().unwrap().is_empty());
    let received = provider.received.lock().unwrap();
    let second = serde_json::to_string(&received[1]).unwrap();
    assert!(second.contains("Preserve tenant authorization"));
    assert!(second.contains("The approach is currently sound"));
    assert!(manager.snapshot("live").unwrap().history_messages >= 4);
}

#[tokio::test]
async fn inherited_model_and_provider_changes_drop_incompatible_native_reasoning_history() {
    let manager = Arc::new(AdvisorManager::default());
    let mut initial = text(r#"{"silence":true}"#);
    initial.insert(
        0,
        StreamEvent::OpenAIReasoning {
            id: "opaque-reasoning".into(),
            summary: vec![],
            encrypted_content: Some("PRIVATE-OPAQUE-CONTEXT".into()),
            status: None,
        },
    );
    let provider = Scripted::new(vec![
        initial,
        text(r#"{"silence":true}"#),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Keep requirement A").await;
    *provider.model.lock().unwrap() = "model-b".into();
    run(&manager, &provider, &queue, "Keep requirement A; continue").await;
    let other = Scripted {
        provider_name: "different-provider",
        ..provider.clone()
    };
    run(&manager, &other, &queue, "Keep requirement A; verify").await;
    let received = provider.received.lock().unwrap();
    assert!(
        !serde_json::to_string(&received[1])
            .unwrap()
            .contains("PRIVATE-OPAQUE-CONTEXT")
    );
    assert!(
        !serde_json::to_string(&received[2])
            .unwrap()
            .contains("Keep requirement A; continue")
    );
    assert!(
        serde_json::to_string(&received[2])
            .unwrap()
            .contains("Keep requirement A; verify")
    );
}

#[tokio::test]
async fn specialization_change_discards_previous_advisor_conversation() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        text("Old specialization private observation"),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    for (turn, instructions) in [(1, "Inspect security"), (2, "Inspect performance")] {
        assert!(manager.schedule_update(
            "live".into(),
            provider.fork(),
            queue.clone(),
            AdvisorTurnInput {
                objective: "User task".into(),
                ..Default::default()
            },
            AdvisorConfig {
                enabled: true,
                ..Default::default()
            },
            AdvisorUpdateContext {
                owner_session_id: "live".into(),
                primary_turn_id: turn,
                completed_primary_turn: true,
                instructions: instructions.into(),
                ..Default::default()
            }
        ));
        assert!(
            manager
                .wait_for_idle("live", std::time::Duration::from_secs(1))
                .await
        );
    }
    let received = provider.received.lock().unwrap();
    assert!(
        !serde_json::to_string(&received[1])
            .unwrap()
            .contains("Old specialization private observation")
    );
}

#[tokio::test]
async fn advisor_tool_loop_rejects_ungranted_action_and_replays_paired_result() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        tool("write", json!({"path":"must-not-exist","content":"unsafe"})),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Inspect only").await;
    let received = provider.received.lock().unwrap();
    assert_eq!(received.len(), 2);
    let second = serde_json::to_string(&received[1]).unwrap();
    assert!(second.contains("Tool is not granted"));
    assert!(second.contains("call-write"));
    assert!(manager.notes("live").is_empty());
}

#[tokio::test]
async fn handled_concern_suppresses_paraphrase_but_still_finds_an_unrelated_blocker() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        advice("tenant-scope", "concern", "Tenant scope is missing"),
        advice(
            "tenant-scope",
            "concern",
            "Another phrasing of that tenant issue",
        ),
        advice("data-loss", "blocker", "New independent destructive change"),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Check authorization").await;
    let id = manager.notes("live")[0].id.clone();
    manager
        .resolve_note("live", &id, AdvisorNoteDisposition::Acknowledged)
        .unwrap();
    run(&manager, &provider, &queue, "Scope was repaired").await;
    assert_eq!(manager.notes("live").len(), 1);
    run(&manager, &provider, &queue, "Continue checking").await;
    assert_eq!(manager.notes("live").len(), 2);
    assert!(queue.lock().unwrap().iter().any(|message| message.urgent));
    assert!(
        manager
            .blocks_tool_call("live", "bash", crate::tool::ToolCapability::Execute)
            .is_none(),
        "interactive feedback must permit fixing the issue"
    );
    assert_eq!(manager.snapshot("live").unwrap().suppressed_notes, 1);
}

#[tokio::test]
async fn terminal_transition_preserves_concern_card_without_starting_a_correction() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![advice(
        "minor-gap",
        "concern",
        "Review the optional check",
    )]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Finish the work").await;
    assert_eq!(queue.lock().unwrap().len(), 1);
    manager.prepare_terminal_delivery("live");
    assert!(queue.lock().unwrap().is_empty());
    assert_eq!(manager.take_asides("live").len(), 1);
    assert!(manager.take_asides("live").is_empty());
    assert_eq!(manager.notes("live").len(), 1);
}

#[tokio::test]
async fn terminal_blocker_remains_eligible_for_same_invocation_correction() {
    let manager = Arc::new(AdvisorManager::default());
    manager.set_enabled("live", true).unwrap();
    manager.prepare_terminal_delivery("live");
    let provider = Scripted::new(vec![advice(
        "required-gap",
        "blocker",
        "Hard acceptance is unmet",
    )]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Finish").await;
    assert_eq!(queue.lock().unwrap().len(), 1);
    assert!(queue.lock().unwrap()[0].urgent);
    assert!(manager.take_asides("live").is_empty());
}

#[tokio::test]
async fn final_review_requires_a_verdict_only_at_completed_update_and_failure_stays_visible() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        text(r#"{"silence":true}"#),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    for completed in [false, true] {
        assert!(manager.schedule_update(
            "live".into(),
            provider.fork(),
            queue.clone(),
            AdvisorTurnInput::default(),
            AdvisorConfig {
                enabled: true,
                mode: AdvisorMode::FinalReview,
                ..Default::default()
            },
            AdvisorUpdateContext {
                owner_session_id: "live".into(),
                primary_turn_id: 1,
                completed_primary_turn: completed,
                ..Default::default()
            }
        ));
        assert!(
            manager
                .wait_for_idle("live", std::time::Duration::from_secs(1))
                .await
        );
        assert_eq!(
            manager.snapshot("live").unwrap().status,
            if completed {
                AdvisorStatus::Failed
            } else {
                AdvisorStatus::Ready
            }
        );
    }
    manager.cancel_turn("live");
    assert_eq!(
        manager.snapshot("live").unwrap().status,
        AdvisorStatus::Failed
    );
    assert!(manager.notes("live").is_empty());
}

#[test]
fn coalesced_updates_retain_intermediate_edits_and_final_state() {
    let previous = AdvisorTurnInput {
        objective: "Complete task".into(),
        latest_primary_turn: "Edited authorization".into(),
        ..Default::default()
    };
    let latest = AdvisorTurnInput {
        latest_primary_turn: "Ran authorization tests".into(),
        verification_status: "passed".into(),
        ..Default::default()
    };
    let input = runtime::coalesce(previous, latest, true);
    assert_eq!(input.objective, "Complete task");
    assert!(input.latest_primary_turn.contains("Edited authorization"));
    assert!(
        input
            .latest_primary_turn
            .contains("Ran authorization tests")
    );
    assert_eq!(input.verification_status, "passed");
}

#[test]
fn oversized_pending_updates_preserve_newest_evidence_with_explicit_elision() {
    let previous = AdvisorTurnInput {
        objective: "Keep the original requirement".into(),
        latest_primary_turn: "older progress ".repeat(3000),
        diagnostics: "old diagnostics ".repeat(400),
        ..Default::default()
    };
    let latest = AdvisorTurnInput {
        latest_primary_turn: "NEWEST: acceptance failed after the final edit".into(),
        diagnostics: "NEWEST diagnostic: authorization was removed".into(),
        ..Default::default()
    };
    let input = runtime::coalesce(previous, latest, true);
    assert!(
        input
            .latest_primary_turn
            .contains("older advisor evidence elided")
    );
    assert!(
        input
            .latest_primary_turn
            .ends_with("NEWEST: acceptance failed after the final edit")
    );
    assert!(
        input
            .diagnostics
            .ends_with("NEWEST diagnostic: authorization was removed")
    );
    assert_eq!(input.objective, "Keep the original requirement");
    assert!(serde_json::to_vec(&input).unwrap().len() <= MAX_INPUT_BYTES);
}

#[tokio::test]
async fn delivered_interrupt_cooldown_keeps_reviewing_and_blockers_bypass_it() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Arc::new(AdvisorManager::persistent(directory.path().to_path_buf()));
    let provider = Scripted::new(vec![
        advice("first", "concern", "First concrete issue"),
        advice("second", "concern", "Another distinct issue"),
        advice("urgent", "blocker", "Material new blocker"),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    run(&manager, &provider, &queue, "Inspect the change").await;
    let content = queue.lock().unwrap().remove(0).content;
    manager.record_delivery("live", &content);
    assert_eq!(
        manager
            .snapshot("live")
            .unwrap()
            .interruption_cooldown_remaining,
        3
    );
    drop(manager);
    let manager = Arc::new(AdvisorManager::persistent(directory.path().to_path_buf()));
    manager.resume("live");
    run(&manager, &provider, &queue, "Fixing the first issue").await;
    assert!(queue.lock().unwrap().is_empty());
    assert_eq!(manager.take_asides("live").len(), 1);
    run(&manager, &provider, &queue, "Continue").await;
    assert_eq!(queue.lock().unwrap().len(), 1);
    assert!(queue.lock().unwrap()[0].urgent);
    assert_eq!(provider.received.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn live_cadence_selects_or_skips_a_whole_primary_turn() {
    let manager = Arc::new(AdvisorManager::default());
    let provider = Scripted::new(vec![
        text(r#"{"silence":true}"#),
        text(r#"{"silence":true}"#),
        text(r#"{"silence":true}"#),
    ]);
    let queue = Arc::new(Mutex::new(Vec::new()));
    let config = AdvisorConfig {
        enabled: true,
        review_every_n_turns: 2,
        ..Default::default()
    };
    for (id, complete, expected) in [
        (1, false, true),
        (1, true, true),
        (2, false, false),
        (2, true, false),
        (3, false, true),
    ] {
        assert_eq!(
            manager.schedule_update(
                "live".into(),
                provider.fork(),
                queue.clone(),
                AdvisorTurnInput::default(),
                config.clone(),
                AdvisorUpdateContext {
                    owner_session_id: "live".into(),
                    completed_primary_turn: complete,
                    primary_turn_id: id,
                    ..Default::default()
                }
            ),
            expected
        );
        assert!(
            manager
                .wait_for_idle("live", std::time::Duration::from_secs(1))
                .await
        );
    }
    assert_eq!(provider.received.lock().unwrap().len(), 3);
    assert_eq!(manager.snapshot("live").unwrap().turns_observed, 2);
}

#[test]
fn context_maintenance_keeps_complete_tool_exchanges_and_initial_task() {
    use crate::message::{ContentBlock, Role};
    let mut history = history::AdvisorHistory::default();
    for index in 0..20 {
        let id = format!("read-{index}");
        history.retain(
            "Preserve owner-only access",
            vec![
                Message::user(&format!("Visible update {index}")),
                Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::ToolUse {
                        id: id.clone(),
                        name: "read".into(),
                        input: json!({"file_path":"auth.rs"}),
                        thought_signature: Some("provider signature".into()),
                    }],
                    timestamp: None,
                    tool_duration_ms: None,
                },
                Message::tool_result(&id, "verified source", false),
            ],
        );
    }
    let messages = history.messages("Continue", 2500);
    let encoded = serde_json::to_string(&messages).unwrap();
    assert!(encoded.contains("Preserve owner-only access"));
    assert!(encoded.contains("Visible update 0"));
    let mut open_calls = std::collections::HashSet::new();
    for message in messages {
        for block in message.content {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    open_calls.insert(id);
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    assert!(open_calls.remove(&tool_use_id));
                }
                _ => {}
            }
        }
    }
    assert!(open_calls.is_empty());
}

struct CancelProvider {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, AtomicOrdering::SeqCst);
    }
}
#[async_trait]
impl Provider for CancelProvider {
    fn name(&self) -> &str {
        "cancel-fixture"
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            started: self.started.clone(),
            dropped: self.dropped.clone(),
        })
    }
    async fn complete(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> anyhow::Result<crate::provider::EventStream> {
        let _drop = DropFlag(self.dropped.clone());
        self.started.notify_one();
        futures::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_drops_provider_future_and_fences_late_delivery() {
    let manager = Arc::new(AdvisorManager::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let queue = Arc::new(Mutex::new(Vec::new()));
    assert!(manager.schedule_turn(
        "cancel".into(),
        Arc::new(CancelProvider {
            started: started.clone(),
            dropped: dropped.clone()
        }),
        queue.clone(),
        AdvisorTurnInput::default(),
        AdvisorConfig {
            enabled: true,
            ..Default::default()
        }
    ));
    started.notified().await;
    manager.cancel_turn("cancel");
    for _ in 0..100 {
        if dropped.load(AtomicOrdering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(dropped.load(AtomicOrdering::SeqCst));
    assert!(queue.lock().unwrap().is_empty());
    assert!(!manager.has_pending_review("cancel"));
}

#[tokio::test]
async fn specialization_reload_cancels_old_review_and_pending_update() {
    let manager = Arc::new(AdvisorManager::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let queue = Arc::new(Mutex::new(Vec::new()));
    let original = Arc::new(CancelProvider {
        started: started.clone(),
        dropped: dropped.clone(),
    });
    let config = AdvisorConfig {
        enabled: true,
        ..Default::default()
    };
    let context = AdvisorUpdateContext {
        owner_session_id: "reload".into(),
        primary_turn_id: 1,
        instructions: "Original specialization".into(),
        ..Default::default()
    };
    assert!(manager.schedule_update(
        "reload".into(),
        original.clone(),
        queue.clone(),
        AdvisorTurnInput::default(),
        config.clone(),
        context.clone()
    ));
    started.notified().await;
    assert!(manager.schedule_update(
        "reload".into(),
        original,
        queue.clone(),
        AdvisorTurnInput {
            latest_primary_turn: "STALE-PENDING-UPDATE".into(),
            ..Default::default()
        },
        config.clone(),
        context.clone()
    ));
    let replacement = Scripted::new(vec![text(r#"{"silence":true}"#)]);
    let reconfigured = AdvisorUpdateContext {
        instructions: "New specialization".into(),
        ..context
    };
    assert!(manager.schedule_update(
        "reload".into(),
        replacement.fork(),
        queue.clone(),
        AdvisorTurnInput {
            latest_primary_turn: "NEW-UPDATE".into(),
            ..Default::default()
        },
        config,
        reconfigured
    ));
    assert!(
        manager
            .wait_for_idle("reload", std::time::Duration::from_secs(1))
            .await
    );
    assert!(dropped.load(AtomicOrdering::SeqCst));
    assert!(queue.lock().unwrap().is_empty());
    let received = replacement.received.lock().unwrap();
    assert_eq!(received.len(), 1);
    let request = serde_json::to_string(&received[0]).unwrap();
    assert!(request.contains("NEW-UPDATE"));
    assert!(!request.contains("STALE-PENDING-UPDATE"));
}
