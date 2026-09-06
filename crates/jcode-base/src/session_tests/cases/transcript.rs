#[test]
fn test_redacted_for_export_redacts_tool_result_and_tool_input() -> Result<()> {
    let mut session = Session::create_with_id(
        "session_redact_persist_test".to_string(),
        None,
        Some("redaction test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "OPENROUTER_API_KEY=sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            is_error: None,
        }],
    );

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool_2".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "echo ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123",
                "api_key": "short-secret-value",
                "source": "fn add(a: i32, b: i32) -> i32 { a + b }"
            }),
            thought_signature: None,
        }],
    );

    let persisted = session.redacted_for_export();

    let first_content = &persisted.messages[0].content[0];
    let ContentBlock::ToolResult { content, .. } = first_content else {
        return Err(anyhow!("expected tool result block"));
    };
    assert!(content.contains("OPENROUTER_API_KEY=[REDACTED_SECRET]"));
    assert!(!content.contains("sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789"));

    let second_content = &persisted.messages[1].content[0];
    let ContentBlock::ToolUse { input, .. } = second_content else {
        return Err(anyhow!("expected tool use block"));
    };
    let input_str = input.to_string();
    assert!(input_str.contains("[REDACTED_SECRET]"));
    assert!(!input_str.contains("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"));
    assert!(!input_str.contains("short-secret-value"));
    assert!(input_str.contains("fn add(a: i32, b: i32)"));
    Ok(())
}
#[test]
fn test_redacted_for_export_redacts_replay_events() -> Result<()> {
    let mut session = Session::create_with_id(
        "session_redacted_replay_events_test".to_string(),
        None,
        Some("redacted replay events".to_string()),
    );

    session.record_replay_display_message(
        "swarm",
        Some("DM from fox".to_string()),
        "OPENROUTER_API_KEY=sk-or-v1-secret-value",
    );
    session.record_swarm_status_event(vec![crate::protocol::SwarmMemberStatus {
        session_id: "session_fox".to_string(),
        friendly_name: Some("fox".to_string()),
        status: "running".to_string(),
        detail: Some("ANTHROPIC_API_KEY=sk-ant-secret-value".to_string()),
        role: Some("agent".to_string()),
        is_headless: None,
        live_attachments: None,
        status_age_secs: None,
        output_tail: None,
        report_back_to_session_id: None,
        todo_progress: None,
        todo_items: Vec::new(),
        task_label: None,
        runtime: crate::protocol::SwarmMemberRuntime::default(),
    }]);
    session.record_swarm_plan_event(
        "swarm_test".to_string(),
        1,
        vec![crate::plan::PlanItem {
            content: "OPENROUTER_API_KEY=sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            status: "pending".to_string(),
            priority: "high".to_string(),
            id: "task-1".to_string(),
            subsystem: None,
            file_scope: Vec::new(),
            blocked_by: vec![],
            assigned_to: None,
        }],
        vec![],
        Some("ANTHROPIC_API_KEY=sk-ant-secret-value".to_string()),
    );

    let redacted = session.redacted_for_export();
    assert_eq!(redacted.replay_events.len(), 3);

    let StoredReplayEventKind::DisplayMessage { content, .. } = &redacted.replay_events[0].kind
    else {
        return Err(anyhow!("expected display message replay event"));
    };
    assert!(content.contains("OPENROUTER_API_KEY=[REDACTED_SECRET]"));
    assert!(!content.contains("sk-or-v1-secret-value"));

    let StoredReplayEventKind::SwarmStatus { members } = &redacted.replay_events[1].kind else {
        return Err(anyhow!("expected swarm status replay event"));
    };
    let detail = members[0].detail.as_deref().unwrap_or_default();
    assert!(detail.contains("ANTHROPIC_API_KEY=[REDACTED_SECRET]"));
    assert!(!detail.contains("sk-ant-secret-value"));

    let StoredReplayEventKind::SwarmPlan { items, reason, .. } = &redacted.replay_events[2].kind
    else {
        return Err(anyhow!("expected swarm plan replay event"));
    };
    assert!(
        items[0]
            .content
            .contains("OPENROUTER_API_KEY=[REDACTED_SECRET]")
    );
    assert!(
        !items[0]
            .content
            .contains("sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789")
    );
    let reason = reason.as_deref().unwrap_or_default();
    assert!(reason.contains("ANTHROPIC_API_KEY=[REDACTED_SECRET]"));
    assert!(!reason.contains("sk-ant-secret-value"));
    Ok(())
}
#[test]
fn test_summarize_tool_calls_includes_tool_only_assistant_messages() {
    let mut session = Session::create_with_id(
        "session_tool_summary_test".to_string(),
        None,
        Some("tool summary test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "pwd",
                "intent": "Inspect the current directory"
            }),
            thought_signature: None,
        }],
    );

    let summaries = summarize_tool_calls(&session, 10);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].tool_name, "bash");
    assert_eq!(
        summaries[0].intent.as_deref(),
        Some("Inspect the current directory")
    );
    assert!(summaries[0].brief_output.contains("pwd"));
}
#[test]
fn test_render_messages_honors_system_display_role_override() {
    let mut session = Session::create_with_id(
        "session_display_role_test".to_string(),
        None,
        Some("display role test".to_string()),
    );

    session.add_message_with_display_role(
        Role::User,
        vec![ContentBlock::Text {
            text: "[Background Task Completed]\nTask: abc123 (bash)".to_string(),
            cache_control: None,
        }],
        Some(StoredDisplayRole::System),
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].role, "system");
    assert!(rendered[0].content.contains("Background Task Completed"));
}
#[test]
fn legacy_scheduled_task_message_renders_as_system() {
    let mut session = Session::create(None, None);
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "[Scheduled task]\nA scheduled task for this session is now due.\n\nTask: check progress".to_string(),
            cache_control: None,
        }],
    );

    let rendered = render::render_messages(&session);
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].role, "system");
    assert_eq!(session.visible_conversation_message_count(), 0);
}
#[test]
fn test_render_messages_shows_auto_poke_continuations_as_system_not_user() {
    // Regression: incomplete-todo and private-quality continuations are persisted as
    // Role::User so the model continues the turn, but the live UI hides them.
    // On reload/resume/remote attach the renderer must not resurrect them as
    // the user's last prompt.
    let mut session = Session::create_with_id(
        "session_render_auto_poke_test".to_string(),
        None,
        Some("auto poke render test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "please fix the login bug".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "Working on it.".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: crate::todo::build_auto_poke_message(2),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: crate::todo::TODO_COMPLETION_CONTINUATION_MESSAGE.to_string(),
            cache_control: None,
        }],
    );

    let rendered = render_messages(&session);
    let user_messages: Vec<_> = rendered
        .iter()
        .filter(|message| message.role == "user")
        .collect();
    assert_eq!(
        user_messages.len(),
        1,
        "only the real prompt should render as a user message: {rendered:?}"
    );
    assert_eq!(user_messages[0].content, "please fix the login bug");

    let system_contents: Vec<_> = rendered
        .iter()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .collect();
    assert!(
        system_contents
            .iter()
            .any(|content| content.contains("incomplete todo")),
        "auto-poke continuation should render as system: {rendered:?}"
    );
    assert!(
        system_contents
            .iter()
            .any(|content| content.contains("Double-checking confidence")),
        "quality continuation should render as a short system notice: {rendered:?}"
    );
    // The model-facing instruction text stays out of the transcript.
    assert!(
        !system_contents
            .iter()
            .any(|content| content.contains("Validate the completed result")),
        "quality continuation leaked model-facing instructions: {rendered:?}"
    );
}
#[test]
fn test_render_messages_renders_reasoning_before_answer_in_stored_order() {
    // Regression: providers persist the assistant turn as `[Text, ReasoningTrace,
    // ToolUse]` (see agent/turn_loops.rs push order). On resume/re-render the
    // reasoning must still appear *before* the answer text to match the live
    // streaming order, even though the Text block is stored first.
    use jcode_render_core::REASONING_SENTINEL;

    let _env_lock = lock_env();
    let _mode = EnvVarGuard::set("JCODE_REASONING_DISPLAY", "full");
    crate::config::invalidate_config_cache();

    let mut session = Session::create_with_id(
        "session_render_reasoning_order_test".to_string(),
        None,
        Some("render reasoning order test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::Text {
                text: "Here is the answer.".to_string(),
                cache_control: None,
            },
            ContentBlock::ReasoningTrace {
                text: "step one\nstep two".to_string(),
            },
        ],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    let content = &rendered[0].content;
    assert!(
        content.contains(&format!("*{0}step one{0}*", REASONING_SENTINEL)),
        "expected reasoning markup, got: {content:?}"
    );
    assert!(content.contains("Here is the answer."));
    let reasoning_pos = content.find("step two").unwrap();
    let answer_pos = content.find("Here is the answer.").unwrap();
    assert!(
        reasoning_pos < answer_pos,
        "reasoning should precede the answer text even when stored after it: {content:?}"
    );
}
#[test]
fn test_render_messages_renders_persisted_reasoning() {
    use jcode_render_core::REASONING_SENTINEL;

    let _env_lock = lock_env();
    let _mode = EnvVarGuard::set("JCODE_REASONING_DISPLAY", "full");
    crate::config::invalidate_config_cache();

    let mut session = Session::create_with_id(
        "session_render_reasoning_test".to_string(),
        None,
        Some("render reasoning test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::ReasoningTrace {
                text: "step one\nstep two".to_string(),
            },
            ContentBlock::Text {
                text: "Here is the answer.".to_string(),
                cache_control: None,
            },
        ],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    let content = &rendered[0].content;
    // Reasoning lines are rendered as dim/italic markup with the sentinel.
    assert!(
        content.contains(&format!("*{0}step one{0}*", REASONING_SENTINEL)),
        "expected reasoning markup, got: {content:?}"
    );
    assert!(
        content.contains(&format!("*{0}step two{0}*", REASONING_SENTINEL)),
        "expected reasoning markup, got: {content:?}"
    );
    // Answer text follows the reasoning block.
    assert!(content.contains("Here is the answer."));
    let reasoning_end = content.find("step two").unwrap();
    let answer_start = content.find("Here is the answer.").unwrap();
    assert!(
        reasoning_end < answer_start,
        "reasoning should precede the answer text: {content:?}"
    );
}
#[test]
fn test_render_messages_renders_legacy_reasoning_variant() {
    use jcode_render_core::REASONING_SENTINEL;

    let _env_lock = lock_env();
    let _mode = EnvVarGuard::set("JCODE_REASONING_DISPLAY", "full");
    crate::config::invalidate_config_cache();

    let mut session = Session::create_with_id(
        "session_render_legacy_reasoning_test".to_string(),
        None,
        Some("render legacy reasoning test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Reasoning {
            text: "legacy thought".to_string(),
        }],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    assert!(
        rendered[0]
            .content
            .contains(&format!("*{0}legacy thought{0}*", REASONING_SENTINEL)),
        "expected legacy reasoning markup, got: {:?}",
        rendered[0].content
    );
}
#[test]
fn test_render_messages_hides_persisted_reasoning_in_current_mode() {
    use jcode_render_core::REASONING_SENTINEL;

    let _env_lock = lock_env();
    let _mode = EnvVarGuard::set("JCODE_REASONING_DISPLAY", "current");
    crate::config::invalidate_config_cache();

    let mut session = Session::create_with_id(
        "session_render_reasoning_current_test".to_string(),
        None,
        Some("render reasoning current test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::ReasoningTrace {
                text: "step one\nstep two\nstep three".to_string(),
            },
            ContentBlock::Text {
                text: "Here is the answer.".to_string(),
                cache_control: None,
            },
        ],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    let content = &rendered[0].content;
    // In `current` mode only the *live* reasoning block is ever shown; it streams
    // then is discarded once the model answers. Re-rendered history therefore
    // shows no past reasoning at all (no trace line, no lines, no sentinel).
    assert!(
        !content.contains(REASONING_SENTINEL),
        "no reasoning markup expected in current mode on reload: {content:?}"
    );
    assert!(
        !content.contains("step one")
            && !content.contains("step two")
            && !content.contains("thought"),
        "individual reasoning lines/trace must not be replayed in current mode: {content:?}"
    );
    // The answer text is preserved.
    assert!(content.contains("Here is the answer."));
}
#[test]
fn test_render_messages_hides_persisted_reasoning_in_off_mode() {
    use jcode_render_core::REASONING_SENTINEL;

    let _env_lock = lock_env();
    let _mode = EnvVarGuard::set("JCODE_REASONING_DISPLAY", "off");
    crate::config::invalidate_config_cache();

    let mut session = Session::create_with_id(
        "session_render_reasoning_off_test".to_string(),
        None,
        Some("render reasoning off test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::ReasoningTrace {
                text: "secret thought".to_string(),
            },
            ContentBlock::Text {
                text: "Here is the answer.".to_string(),
                cache_control: None,
            },
        ],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    let content = &rendered[0].content;
    assert!(
        !content.contains(REASONING_SENTINEL) && !content.contains("secret thought"),
        "reasoning must be hidden entirely in off mode: {content:?}"
    );
    assert!(content.contains("Here is the answer."));
}
#[test]
fn test_render_messages_honors_background_task_display_role_override() {
    let mut session = Session::create_with_id(
        "session_background_task_role_test".to_string(),
        None,
        Some("background task role test".to_string()),
    );

    session.add_message_with_display_role(
            Role::User,
            vec![ContentBlock::Text {
                text: "**Background task** `abc123` · `bash` · ✓ completed · 7.1s · exit 0\n\n_No output captured._\n\n_Full output:_ `bg action=\"output\" task_id=\"abc123\"`".to_string(),
                cache_control: None,
            }],
            Some(StoredDisplayRole::BackgroundTask),
        );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].role, "background_task");
    assert!(rendered[0].content.contains("**Background task**"));
}
#[test]
fn test_render_messages_hides_internal_system_reminders() {
    let mut session = Session::create_with_id(
        "session_hidden_system_reminder_test".to_string(),
        None,
        Some("hidden reminder test".to_string()),
    );

    assert!(session.ensure_initial_session_context_message());
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "visible prompt".to_string(),
            cache_control: None,
        }],
    );

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].role, "user");
    assert_eq!(rendered[0].content, "visible prompt");
}
#[test]
fn test_render_messages_shows_recent_compacted_history_by_default() {
    let mut session = Session::create_with_id(
        "session_render_compacted_history_test".to_string(),
        None,
        Some("render compacted history test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "old prompt".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "old response".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "current prompt".to_string(),
            cache_control: None,
        }],
    );
    session.compaction = Some(StoredCompactionState {
        summary_text: "old prompt and response".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 2,
        original_turn_count: 2,
        compacted_count: 2,
    });

    let rendered = render_messages(&session);
    assert_eq!(rendered.len(), 4);
    assert_eq!(rendered[0].role, "system");
    assert!(rendered[0].content.contains("showing all 2"));
    assert_eq!(rendered[1].role, "user");
    assert_eq!(rendered[1].content, "old prompt");
    assert_eq!(rendered[2].role, "assistant");
    assert_eq!(rendered[2].content, "old response");
    assert_eq!(rendered[3].role, "user");
    assert_eq!(rendered[3].content, "current prompt");
}
#[test]
fn test_render_messages_can_expand_compacted_history_window() {
    let mut session = Session::create_with_id(
        "session_render_compacted_history_expand_test".to_string(),
        None,
        Some("render compacted history expand test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "old prompt".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "old response".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "current prompt".to_string(),
            cache_control: None,
        }],
    );
    session.compaction = Some(StoredCompactionState {
        summary_text: "old prompt and response".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 2,
        original_turn_count: 2,
        compacted_count: 2,
    });

    // A small compacted prefix (few renderable messages, a single turn) must
    // never be truncated, even when a tiny visible window is requested. A single
    // long turn in particular must always render in full.
    let (rendered, _images, info) = render_messages_and_images_with_compacted_history(&session, 1);
    let info = info.expect("compacted info");
    assert_eq!(info.total_messages, 2);
    assert_eq!(info.visible_messages, 2);
    assert_eq!(info.remaining_messages, 0);
    assert_eq!(info.hidden_user_prompts, 0);
    assert_eq!(rendered.len(), 4);
    assert!(rendered[0].content.contains("showing all 2"));
    assert_eq!(rendered[1].content, "old prompt");
    assert_eq!(rendered[2].content, "old response");
    assert_eq!(rendered[3].content, "current prompt");

    let (rendered_all, _images, info_all) =
        render_messages_and_images_with_compacted_history(&session, usize::MAX);
    let info_all = info_all.expect("compacted info");
    assert_eq!(info_all.visible_messages, 2);
    assert_eq!(info_all.remaining_messages, 0);
    assert_eq!(info_all.hidden_user_prompts, 0);
    assert_eq!(rendered_all.len(), 4);
    assert!(rendered_all[0].content.contains("showing all 2"));
    assert_eq!(rendered_all[1].content, "old prompt");
    assert_eq!(rendered_all[2].content, "old response");
    assert_eq!(rendered_all[3].content, "current prompt");
}
#[test]
fn test_compacted_history_truncates_only_when_long_and_many_turns() {
    let mut session = Session::create_with_id(
        "session_render_compacted_history_truncate_test".to_string(),
        None,
        Some("render compacted history truncate test".to_string()),
    );

    // Build a large compacted prefix: many turns, each with several visible
    // messages, well past both guardrails (>80 renderable, >5 turns).
    let prefix_turns = 20usize;
    for t in 0..prefix_turns {
        session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: format!("prompt {t}"),
                cache_control: None,
            }],
        );
        // 4 assistant messages per turn -> 5 renderable per turn.
        for r in 0..4 {
            session.add_message(
                Role::Assistant,
                vec![ContentBlock::Text {
                    text: format!("response {t}.{r}"),
                    cache_control: None,
                }],
            );
        }
    }
    // Current (uncompacted) prompt after the compacted prefix.
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "current prompt".to_string(),
            cache_control: None,
        }],
    );

    let compacted_count = prefix_turns * 5; // every prefix message is compacted
    session.compaction = Some(StoredCompactionState {
        summary_text: "older compacted context".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: prefix_turns,
        original_turn_count: prefix_turns,
        compacted_count,
    });

    let total_renderable = prefix_turns * 5; // 100

    // Request a small window: truncation kicks in because the prefix is long
    // and has many turns.
    let (rendered, _images, info) = render_messages_and_images_with_compacted_history(&session, 10);
    let info = info.expect("compacted info");
    assert_eq!(info.total_messages, total_renderable);
    assert!(info.visible_messages < total_renderable);
    assert!(info.remaining_messages > 0);
    assert_eq!(
        info.visible_messages + info.remaining_messages,
        total_renderable
    );
    assert!(info.hidden_user_prompts > 0);
    // The first rendered body message (after the marker) must be a user prompt
    // because we snap the window to a turn boundary.
    assert_eq!(rendered[1].role, "user");

    // Requesting everything shows the whole prefix with no hidden prompts.
    let (_rendered_all, _images, info_all) =
        render_messages_and_images_with_compacted_history(&session, usize::MAX);
    let info_all = info_all.expect("compacted info");
    assert_eq!(info_all.visible_messages, total_renderable);
    assert_eq!(info_all.remaining_messages, 0);
    assert_eq!(info_all.hidden_user_prompts, 0);
}
#[test]
fn test_compacted_history_never_truncates_single_long_turn() {
    let mut session = Session::create_with_id(
        "session_render_compacted_history_single_turn_test".to_string(),
        None,
        Some("render compacted history single turn test".to_string()),
    );

    // A single turn with a huge number of visible messages (well over 80).
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "the one long prompt".to_string(),
            cache_control: None,
        }],
    );
    for r in 0..150 {
        session.add_message(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: format!("long response chunk {r}"),
                cache_control: None,
            }],
        );
    }
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "current prompt".to_string(),
            cache_control: None,
        }],
    );

    let compacted_count = 151; // prompt + 150 responses
    session.compaction = Some(StoredCompactionState {
        summary_text: "older compacted context".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 1,
        original_turn_count: 1,
        compacted_count,
    });

    // Even with a tiny requested window, a single long turn is never truncated.
    let (_rendered, _images, info) = render_messages_and_images_with_compacted_history(&session, 5);
    let info = info.expect("compacted info");
    assert_eq!(info.total_messages, compacted_count);
    assert_eq!(info.visible_messages, compacted_count);
    assert_eq!(info.remaining_messages, 0);
    assert_eq!(info.hidden_user_prompts, 0);
}
#[test]
fn test_compacted_history_window_counts_renderable_messages_not_hidden_reminders() {
    let mut session = Session::create_with_id(
        "session_render_compacted_history_hidden_budget_test".to_string(),
        None,
        Some("render compacted history hidden budget test".to_string()),
    );

    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "older visible prompt".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "<system-reminder>hidden reminder one</system-reminder>".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "previous visible assistant response".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "<system-reminder>hidden reminder two</system-reminder>".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "current prompt".to_string(),
            cache_control: None,
        }],
    );
    session.compaction = Some(StoredCompactionState {
        summary_text: "older compacted context".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 4,
        original_turn_count: 4,
        compacted_count: 4,
    });

    let (rendered, _images, info) = render_messages_and_images_with_compacted_history(&session, 1);
    let info = info.expect("compacted info");

    // Hidden system reminders are never counted as renderable messages, so the
    // small prefix (2 renderable, 1 turn) is shown in full rather than truncated.
    assert_eq!(info.total_messages, 2);
    assert_eq!(info.visible_messages, 2);
    assert_eq!(info.remaining_messages, 0);
    assert_eq!(info.hidden_user_prompts, 0);
    assert_eq!(rendered.len(), 4);
    assert!(rendered[0].content.contains("showing all 2"));
    assert_eq!(rendered[1].role, "user");
    assert_eq!(rendered[1].content, "older visible prompt");
    assert_eq!(rendered[2].role, "assistant");
    assert_eq!(rendered[2].content, "previous visible assistant response");
    assert_eq!(rendered[3].content, "current prompt");
    assert!(
        rendered
            .iter()
            .all(|msg| !msg.content.contains("hidden reminder"))
    );
}
