use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn stored(role: Role, content: Vec<ContentBlock>) -> StoredMessage {
    StoredMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content,
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }
}

#[test]
fn visible_observation_excludes_private_reasoning_and_marks_elision() {
    let messages = vec![stored(
        Role::Assistant,
        vec![
            ContentBlock::Reasoning {
                text: "PRIVATE_REASONING".into(),
            },
            ContentBlock::ReasoningTrace {
                text: "PRIVATE_TRACE".into(),
            },
            ContentBlock::AnthropicThinking {
                thinking: "PRIVATE_THINKING".into(),
                signature: "PRIVATE_SIGNATURE".into(),
            },
            ContentBlock::OpenAIReasoning {
                id: "PRIVATE_ID".into(),
                summary: vec!["PRIVATE_SUMMARY".into()],
                encrypted_content: Some("PRIVATE_ENCRYPTED".into()),
                status: None,
            },
            ContentBlock::Text {
                text: "visible progress".into(),
                cache_control: None,
            },
            ContentBlock::ToolUse {
                id: "read-1".into(),
                name: "read".into(),
                input: json!({"file_path":"src/bounds.py"}),
                thought_signature: Some("PRIVATE_TOOL_SIGNATURE".into()),
            },
            ContentBlock::ToolResult {
                tool_use_id: "read-1".into(),
                content: "é".repeat(BLOCK_BYTES),
                is_error: None,
            },
        ],
    )];
    let visible = visible_delta(&messages);
    assert!(!visible.contains("PRIVATE_"));
    assert!(visible.contains("visible progress"));
    assert!(visible.contains("src/bounds.py"));
    assert!(visible.contains("[excerpt truncated]"));
}

#[test]
fn bounded_visible_text_and_tool_input_never_expose_partial_private_keys() {
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        "SECRET_KEY_MATERIAL".repeat(BLOCK_BYTES)
    );
    let messages = vec![stored(
        Role::Assistant,
        vec![
            ContentBlock::Text {
                text: pem.clone(),
                cache_control: None,
            },
            ContentBlock::ToolUse {
                id: "tool-1".into(),
                name: "example".into(),
                input: json!({"content":pem}),
                thought_signature: None,
            },
        ],
    )];
    let visible = visible_delta(&messages);
    assert!(!visible.contains("SECRET_KEY_MATERIAL"));
    assert!(visible.contains("[redacted private key]"));
}

#[test]
fn continue_retains_earlier_user_requirements_without_synthetic_context() {
    let messages = vec![
        stored(
            Role::User,
            vec![ContentBlock::Text {
                text: "<system-reminder>\n# Session Context".into(),
                cache_control: None,
            }],
        ),
        stored(
            Role::User,
            vec![ContentBlock::Text {
                text: "Accept the inclusive boundary and verify the negative case".into(),
                cache_control: None,
            }],
        ),
        stored(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "Working on it".into(),
                cache_control: None,
            }],
        ),
        stored(
            Role::User,
            vec![ContentBlock::Text {
                text: "continue".into(),
                cache_control: None,
            }],
        ),
    ];
    let context = task_context(&messages, "continue");
    assert!(context.contains("Accept the inclusive boundary"));
    assert!(context.contains("Current user request:\ncontinue"));
    assert!(!context.contains("Session Context"));
}

#[derive(Default)]
struct FeedbackState {
    primary_calls: AtomicUsize,
    advisor_read: AtomicBool,
    emitted: AtomicBool,
    initial_answer: AtomicBool,
    saw_project_instructions: AtomicBool,
}

struct FeedbackProvider {
    root: PathBuf,
    state: Arc<FeedbackState>,
}

fn text_stream(text: &str) -> crate::provider::EventStream {
    Box::pin(futures::stream::iter(vec![
        Ok(StreamEvent::TextDelta(text.to_string())),
        Ok(StreamEvent::MessageEnd {
            stop_reason: Some("end_turn".into()),
        }),
    ]))
}

fn tool_stream(name: &str, input: serde_json::Value) -> crate::provider::EventStream {
    Box::pin(futures::stream::iter(vec![
        Ok(StreamEvent::ToolUseStart {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
        }),
        Ok(StreamEvent::ToolInputDelta(input.to_string())),
        Ok(StreamEvent::ToolUseEnd),
        Ok(StreamEvent::MessageEnd {
            stop_reason: Some("tool_use".into()),
        }),
    ]))
}

#[async_trait]
impl Provider for FeedbackProvider {
    fn name(&self) -> &str {
        "advisor-feedback-fixture"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            root: self.root.clone(),
            state: Arc::clone(&self.state),
        })
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        _: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        let transcript = serde_json::to_string(messages)?;
        if tools.iter().any(|tool| tool.name == "advise") {
            if system.contains("BOUNDARY_PROJECT_RULE")
                || transcript.contains("BOUNDARY_PROJECT_RULE")
            {
                self.state
                    .saw_project_instructions
                    .store(true, Ordering::SeqCst);
            }
            if self.state.emitted.load(Ordering::SeqCst) {
                return Ok(text_stream(r#"{"silence":true}"#));
            }
            let investigated = messages.iter().flat_map(|message| &message.content).any(|block| matches!(block, ContentBlock::ToolResult { content, .. } if content.contains("DEFECT_41")));
            if !investigated {
                return Ok(tool_stream(
                    "read",
                    json!({"file_path":"src/bounds.py","intent":"Inspect boundary implementation"}),
                ));
            }
            self.state.advisor_read.store(true, Ordering::SeqCst);
            // This fixture makes the finding late: the primary has already
            // supplied its provisional terminal answer before advice appears.
            tokio::time::timeout(Duration::from_secs(5), async {
                while !self.state.initial_answer.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            self.state.emitted.store(true, Ordering::SeqCst);
            return Ok(tool_stream(
                "advise",
                json!({
                    "concern_id":"inclusive-boundary", "severity":"blocker",
                    "summary":"The inclusive boundary is still excluded",
                    "evidence":["src/bounds.py: return value > 10 # DEFECT_41"],
                    "recommended_action":"Read src/bounds.py and change the comparison to >= 10",
                    "blocking":true,
                }),
            ));
        }

        let call = self.state.primary_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            return Ok(tool_stream(
                "read",
                json!({"file_path":"README.md","intent":"Inspect project overview"}),
            ));
        }
        if !transcript.contains("[ADVISOR ") {
            self.state.initial_answer.store(true, Ordering::SeqCst);
            return Ok(text_stream("The initial implementation is complete."));
        }
        if std::fs::read_to_string(self.root.join("src/bounds.py"))?.contains(">= 10") {
            return Ok(text_stream(
                "Corrected the inclusive boundary after the advisor identified it.",
            ));
        }
        let primary_read = messages.iter().flat_map(|message| &message.content).any(|block| matches!(block, ContentBlock::ToolResult { content, .. } if content.contains("DEFECT_41")));
        if !primary_read {
            return Ok(tool_stream(
                "read",
                json!({"file_path":"src/bounds.py","intent":"Check advisor finding before editing"}),
            ));
        }
        Ok(tool_stream(
            "write",
            json!({
                "file_path":"src/bounds.py", "content":"def accepts(value):\n    return value >= 10\n",
                "intent":"Fix the inclusive boundary identified by the advisor",
            }),
        ))
    }
}

async fn feedback_agent(root: &std::path::Path) -> (Agent, Arc<FeedbackState>) {
    std::fs::create_dir(root.join("src")).expect("source directory");
    std::fs::write(root.join("README.md"), "A boundary check project.").expect("readme");
    std::fs::write(
        root.join("src/bounds.py"),
        "def accepts(value):\n    return value > 10 # DEFECT_41\n",
    )
    .expect("source");
    std::fs::write(
        root.join("AGENTS.md"),
        "BOUNDARY_PROJECT_RULE: Equality must be accepted.",
    )
    .expect("instructions");
    let state = Arc::new(FeedbackState::default());
    let provider: Arc<dyn Provider> = Arc::new(FeedbackProvider {
        root: root.to_path_buf(),
        state: Arc::clone(&state),
    });
    let registry = Registry::new(Arc::clone(&provider)).await;
    let mut agent = Agent::new_with_initial_working_dir(provider, registry, root.to_str());
    agent.memory_enabled = false;
    crate::advisor::advisor_manager()
        .set_enabled(&agent.session.id, true)
        .expect("enable advisor");
    (agent, state)
}

#[tokio::test]
async fn late_investigative_blocker_corrects_headless_work_without_another_user_turn() {
    let _guard = crate::storage::lock_test_env();
    let root = tempfile::tempdir().expect("directory");
    let (mut agent, state) = feedback_agent(root.path()).await;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        agent.run_once_capture("Implement an inclusive boundary check."),
    )
    .await
    .expect("bounded turn")
    .expect("turn");
    assert!(
        output.contains("Corrected the inclusive boundary"),
        "{output}"
    );
    assert!(state.advisor_read.load(Ordering::SeqCst));
    assert!(state.saw_project_instructions.load(Ordering::SeqCst));
    assert!(
        std::fs::read_to_string(root.path().join("src/bounds.py"))
            .expect("source")
            .contains(">= 10")
    );
    assert!(!crate::advisor::advisor_manager().has_pending_review(&agent.session.id));
    crate::advisor::advisor_manager().remove(&agent.session.id);
}

#[tokio::test]
async fn streaming_terminal_drain_delivers_late_blocker_and_completes_correction() {
    let _guard = crate::storage::lock_test_env();
    let root = tempfile::tempdir().expect("directory");
    let (mut agent, state) = feedback_agent(root.path()).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::time::timeout(
        Duration::from_secs(15),
        agent.run_once_streaming_mpsc(
            "Implement an inclusive boundary check.",
            Vec::new(),
            None,
            tx,
        ),
    )
    .await
    .expect("bounded turn")
    .expect("turn");
    let mut delivered = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, ServerEvent::SoftInterruptInjected { content, .. } if content.contains("inclusive boundary"))
        {
            delivered = true;
        }
    }
    assert!(delivered);
    assert!(state.advisor_read.load(Ordering::SeqCst));
    assert!(
        std::fs::read_to_string(root.path().join("src/bounds.py"))
            .expect("source")
            .contains(">= 10")
    );
    crate::advisor::advisor_manager().remove(&agent.session.id);
}

struct WaitingAdvisorProvider {
    started: Arc<tokio::sync::Semaphore>,
    requests: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for WaitingAdvisorProvider {
    fn name(&self) -> &str {
        "waiting-advisor-fixture"
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            started: Arc::clone(&self.started),
            requests: Arc::clone(&self.requests),
        })
    }
    async fn complete(
        &self,
        _: &[Message],
        tools: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        if tools.iter().any(|tool| tool.name == "advise") {
            self.started.add_permits(1);
            return std::future::pending().await;
        }
        self.requests.fetch_add(1, Ordering::SeqCst);
        Ok(text_stream("Primary answer complete."))
    }
}

fn waiting_agent() -> (Agent, Arc<tokio::sync::Semaphore>, Arc<AtomicUsize>) {
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(WaitingAdvisorProvider {
        started: Arc::clone(&started),
        requests: Arc::clone(&requests),
    });
    let mut agent = Agent::new(provider, Registry::empty());
    agent.memory_enabled = false;
    crate::advisor::advisor_manager()
        .set_enabled(&agent.session.id, true)
        .expect("enable");
    (agent, started, requests)
}

#[tokio::test]
async fn explicit_cancel_stops_terminal_drain_and_never_resumes_primary() {
    let _guard = crate::storage::lock_test_env();
    let (mut agent, started, requests) = waiting_agent();
    let cancel = agent.graceful_shutdown_signal();
    let canceller = async {
        let _permit = started.acquire().await.expect("advisor starts");
        cancel.fire();
    };
    let runner = agent.run_once_capture("Review this change.");
    let (_, result) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(canceller, runner)
    })
    .await
    .expect("cancellation is prompt");
    result.expect("cancelled turn exits cleanly");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(!crate::advisor::advisor_manager().has_pending_review(&agent.session.id));
    assert!(!agent.has_soft_interrupts());
    crate::advisor::advisor_manager().remove(&agent.session.id);
}

#[tokio::test]
async fn dropping_primary_future_fences_background_advisor_completion() {
    let _guard = crate::storage::lock_test_env();
    let (mut agent, started, _) = waiting_agent();
    let mut run = Box::pin(agent.run_once_capture("Review this change."));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = started.acquire() => {},
            result = &mut run => panic!("turn finished before pending advisor: {result:?}"),
        }
    })
    .await
    .expect("advisor starts");
    drop(run);
    assert!(!crate::advisor::advisor_manager().has_pending_review(&agent.session.id));
    assert!(!agent.has_soft_interrupts());
    crate::advisor::advisor_manager().remove(&agent.session.id);
}

struct NativeToolPrimary {
    native: bool,
    first_response: Arc<AtomicBool>,
    advised: Arc<AtomicBool>,
}

#[async_trait]
impl Provider for NativeToolPrimary {
    fn name(&self) -> &str {
        "native-tools-fixture"
    }
    fn handles_tools_internally(&self) -> bool {
        self.native
    }
    fn fork(&self) -> Arc<dyn Provider> {
        // Simulate selecting a separate advisor route with explicit tools.
        Arc::new(Self {
            native: false,
            first_response: Arc::clone(&self.first_response),
            advised: Arc::clone(&self.advised),
        })
    }
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        if tools.iter().any(|tool| tool.name == "advise") {
            while !self.first_response.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            if self.advised.swap(true, Ordering::SeqCst) {
                return Ok(text_stream(r#"{"silence":true}"#));
            }
            return Ok(tool_stream(
                "advise",
                json!({
                    "concern_id":"native-verification", "severity":"blocker",
                    "summary":"Native execution has not verified the result", "evidence":["read call only"],
                    "recommended_action":"Verify before claiming completion", "blocking":true,
                }),
            ));
        }
        if !self.first_response.swap(true, Ordering::SeqCst) {
            return Ok(tool_stream(
                "read",
                json!({"file_path":"README.md","intent":"Provider native read"}),
            ));
        }
        assert!(serde_json::to_string(messages)?.contains("[ADVISOR "));
        Ok(text_stream(
            "Native execution processed the advisor correction.",
        ))
    }
}

#[tokio::test]
async fn provider_native_tool_completion_also_drains_late_advisor_feedback() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeToolPrimary {
        native: true,
        first_response: Arc::new(AtomicBool::new(false)),
        advised: Arc::new(AtomicBool::new(false)),
    });
    let mut agent = Agent::new(provider, Registry::empty());
    agent.memory_enabled = false;
    crate::advisor::advisor_manager()
        .set_enabled(&agent.session.id, true)
        .expect("enable");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        agent.run_once_capture("Check the native execution result."),
    )
    .await
    .expect("bounded native turn")
    .expect("turn");
    assert_eq!(result, "Native execution processed the advisor correction.");
    crate::advisor::advisor_manager().remove(&agent.session.id);
}
