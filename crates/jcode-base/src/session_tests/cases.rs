use super::*;
use anyhow::{Result, anyhow};

#[test]
fn test_render_messages_and_images_share_tool_resolution_and_labels() {
    let mut session = Session::create_with_id(
        "session_render_bundle_test".to_string(),
        None,
        Some("render bundle test".to_string()),
    );

    session.add_message(
        Role::Assistant,
        vec![
            ContentBlock::ToolUse {
                id: "tool_img_1".to_string(),
                name: "view_image".to_string(),
                input: serde_json::json!({"file_path": "/tmp/screenshot.png"}),
                thought_signature: None,
            },
            ContentBlock::ToolResult {
                tool_use_id: "tool_img_1".to_string(),
                content: "rendered image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "abcd".to_string(),
            },
            ContentBlock::Text {
                text: "[Attached image associated with the preceding tool result: screenshot.png]"
                    .to_string(),
                cache_control: None,
            },
        ],
    );

    let (rendered, images) = render_messages_and_images(&session);
    // The `[Attached image associated with the preceding tool result: ...]`
    // text block is synthetic image metadata, not a visible message. It must be
    // folded into the image label and never rendered as a (user) message,
    // otherwise it leaks out as a bogus "last prompt".
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].role, "tool");
    assert_eq!(rendered[0].content, "rendered image");
    assert!(
        !rendered
            .iter()
            .any(|m| m.content.contains("Attached image associated")),
        "attached-image label must not render as its own message"
    );
    assert_eq!(
        rendered[0]
            .tool_data
            .as_ref()
            .map(|tool| tool.name.as_str()),
        Some("view_image")
    );

    assert_eq!(images.len(), 1);
    assert_eq!(images[0].label.as_deref(), Some("screenshot.png"));
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(
        images[0].source,
        RenderedImageSource::ToolResult {
            tool_name: "view_image".to_string(),
        }
    );
}

#[test]
fn reasoning_trace_survives_session_save_and_load() -> Result<()> {
    let _env_lock = lock_env();
    let temp_home = tempfile::Builder::new()
        .prefix("jcode-reasoning-persist-test-")
        .tempdir()
        .map_err(|e| anyhow!(e))?;
    let _home = EnvVarGuard::set("JCODE_HOME", temp_home.path().as_os_str());

    let session_id = "session_reasoning_trace_roundtrip";
    let mut session = Session::create_with_id(session_id.to_string(), None, None);
    session.append_stored_message(StoredMessage {
        id: "msg_assistant".to_string(),
        role: Role::Assistant,
        content: vec![
            ContentBlock::ReasoningTrace {
                text: "step 1: consider the run loop ordering".to_string(),
            },
            ContentBlock::Text {
                text: "Here is my answer.".to_string(),
                cache_control: None,
            },
        ],
        display_role: None,
        timestamp: Some(Utc::now()),
        tool_duration_ms: None,
        token_usage: None,
    });
    session.save()?;

    // The reasoning must be persisted to the on-disk transcript, not just held
    // in memory, so it can be recalled/debugged after a restart.
    let raw = std::fs::read_to_string(session_path(session_id)?)?;
    assert!(
        raw.contains("reasoning_trace"),
        "transcript should serialize reasoning_trace block"
    );
    assert!(raw.contains("step 1: consider the run loop ordering"));

    let loaded = Session::load(session_id)?;
    let assistant = loaded
        .messages
        .iter()
        .find(|m| m.role == Role::Assistant)
        .ok_or_else(|| anyhow!("assistant message missing after reload"))?;
    let has_trace = assistant.content.iter().any(|b| {
        matches!(
            b,
            ContentBlock::ReasoningTrace { text }
                if text == "step 1: consider the run loop ordering"
        )
    });
    assert!(has_trace, "ReasoningTrace must survive save/load roundtrip");
    Ok(())
}

#[test]
fn test_render_images_anchors_tool_and_user_images() {
    let mut session = Session::create_with_id(
        "session_render_image_anchor_test".to_string(),
        None,
        Some("image anchor test".to_string()),
    );

    // Prompt 0 with a pasted image.
    session.add_message(
        Role::User,
        vec![
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "user-image-data".to_string(),
            },
            ContentBlock::Text {
                text: "look at this".to_string(),
                cache_control: None,
            },
        ],
    );
    // Assistant calls a tool.
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool-call-1".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    // Tool result with an attached image.
    session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: "tool-call-1".to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "tool-image-data".to_string(),
            },
        ],
    );

    let (_, images) = render_messages_and_images(&session);
    assert_eq!(images.len(), 2);
    assert_eq!(
        images[0].anchor,
        Some(RenderedImageAnchor::UserPrompt { ordinal: 0 }),
        "pasted user image should anchor to its prompt"
    );
    assert_eq!(
        images[1].anchor,
        Some(RenderedImageAnchor::ToolCall {
            id: "tool-call-1".to_string()
        }),
        "tool image should anchor to its tool call"
    );
}

#[test]
fn test_render_images_attached_label_message_does_not_shift_prompt_ordinals() {
    let mut session = Session::create_with_id(
        "session_render_image_label_ordinal_test".to_string(),
        None,
        Some("image label ordinal test".to_string()),
    );

    // Tool flow that produces a labeled image: the synthetic label text message
    // must not count as a user prompt for anchoring.
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool-call-2".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "shot.png"}),
            thought_signature: None,
        }],
    );
    session.add_message(
        Role::User,
        vec![
            ContentBlock::ToolResult {
                tool_use_id: "tool-call-2".to_string(),
                content: "read image".to_string(),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "tool-image-data".to_string(),
            },
            ContentBlock::Text {
                text: "[Attached image associated with the preceding tool result: shot.png]"
                    .to_string(),
                cache_control: None,
            },
        ],
    );
    // A real follow-up prompt with an image: must be ordinal 0 (first prompt).
    session.add_message(
        Role::User,
        vec![
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "second-user-image".to_string(),
            },
            ContentBlock::Text {
                text: "and this one".to_string(),
                cache_control: None,
            },
        ],
    );

    let (_, images) = render_messages_and_images(&session);
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].label.as_deref(), Some("shot.png"));
    assert_eq!(
        images[1].anchor,
        Some(RenderedImageAnchor::UserPrompt { ordinal: 0 }),
        "label-only messages must not consume prompt ordinals"
    );
}

#[test]
fn fork_notice_is_model_visible_but_hidden_from_transcript() {
    let mut session = Session::create(None, None);
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "original request".to_string(),
            cache_control: None,
        }],
    );

    session.append_fork_notice("session_parent_abc", "otter");

    let notice = session.messages.last().expect("fork notice appended");
    assert_eq!(notice.role, Role::User);
    assert_eq!(notice.display_role, Some(StoredDisplayRole::System));
    let text = notice.content_preview();
    assert!(text.contains("<system-reminder>"));
    assert!(text.contains("forked"));
    assert!(text.contains("session_parent_abc"));
    assert!(text.contains("otter"));

    // Model-visible: included in the provider message list.
    let provider_messages = session.messages_for_provider_uncached();
    assert!(
        provider_messages.iter().any(|message| {
            message.content.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Text { text, .. } if text.contains("forked")
                )
            })
        }),
        "fork notice must reach the model"
    );

    // Transcript-hidden: not rendered as a visible user message.
    let (rendered, _) = render_messages_and_images(&session);
    assert!(
        !rendered
            .iter()
            .any(|message| message.role == "user" && message.content.contains("forked")),
        "fork notice must not render as a visible user message"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn streaming_guard_creates_visible_macos_sleep_assertion() {
    let _lock = lock_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());

    let reason = "Jcode streaming model response";
    {
        let _streaming = StreamingGuard::new("session_power");

        let output = std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .expect("pmset -g assertions should run on macOS");
        assert!(output.status.success(), "pmset should succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(reason),
            "pmset output should show the streaming assertion; output was:\n{stdout}"
        );
    }

    let output = std::process::Command::new("pmset")
        .args(["-g", "assertions"])
        .output()
        .expect("pmset -g assertions should run on macOS");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains(reason),
        "streaming assertion should be released after guard drop; output was:\n{stdout}"
    );
}

/// Issue #432: `/rewind N` must interpret N against the same numbered list the
/// TUI shows, even in tool-heavy sessions where stored user-role tool-result
/// messages vastly outnumber real prompts.
#[test]
fn test_rewind_targets_match_rendered_transcript_numbering() {
    let mut session = Session::create_with_id(
        "session_rewind_numbering_test".to_string(),
        None,
        Some("rewind numbering".to_string()),
    );

    // Turn 1: prompt, assistant tool call, tool result, assistant answer.
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "prompt-1".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
            thought_signature: None,
        }],
    );
    // Tool results are stored as user-role messages; the old index mapping
    // counted them as rewind targets even though the UI never numbers them.
    session.add_message(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "file-a file-b".to_string(),
            is_error: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "answer-1".to_string(),
            cache_control: None,
        }],
    );

    // Turn 2: prompt + answer.
    session.add_message(
        Role::User,
        vec![ContentBlock::Text {
            text: "prompt-2".to_string(),
            cache_control: None,
        }],
    );
    session.add_message(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "answer-2".to_string(),
            cache_control: None,
        }],
    );

    // The numbered /rewind list shows user/assistant transcript entries only:
    // 1 prompt-1, 2 answer-1, 3 prompt-2, 4 answer-2.
    let rendered_targets: Vec<String> = render_messages(&session)
        .into_iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .map(|m| m.content)
        .collect();
    assert_eq!(
        rendered_targets,
        ["prompt-1", "answer-1", "prompt-2", "answer-2"]
    );

    let targets = session.rewind_target_stored_indices();
    assert_eq!(session.rewind_target_count(), 4);
    assert_eq!(targets.len(), 4);

    // Rewinding to entry 3 ("prompt-2") must keep everything through the
    // stored prompt-2 message (stored index 4 → len 5) and drop answer-2.
    assert_eq!(targets[2], 4);
    let mut rewound = session.clone();
    rewound.truncate_messages(targets[2] + 1);
    let remaining: Vec<String> = render_messages(&rewound)
        .into_iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .map(|m| m.content)
        .collect();
    assert_eq!(remaining, ["prompt-1", "answer-1", "prompt-2"]);

    // The old stored-message mapping counted the tool result as target 3,
    // which would have chopped the transcript mid-turn (the #432 bug).
    assert_eq!(
        session.stored_len_for_visible_conversation_message(3),
        Some(3),
        "sanity: raw stored counting diverges, which is why rewind must not use it"
    );
}

/// Issue #688: `/rewind N` after an undo must honour the new N, not repeat the
/// previous rewind's target.
///
/// The reporter described the second `/rewind N` in a session ignoring its
/// argument and landing wherever the first one did. #432 covered the *first*
/// rewind's numbering; nothing covered rewind -> undo -> rewind, which is the
/// sequence that loses transcript if the target list is computed once and
/// reused. Drive the real agent-side operations so the whole cycle is pinned.
#[test]
fn test_rewind_after_undo_uses_the_new_target_not_the_previous_one() {
    let mut session = Session::create_with_id(
        "session_rewind_repeat_test".to_string(),
        None,
        Some("rewind repeat".to_string()),
    );
    for turn in 1..=6 {
        session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: format!("prompt-{turn}"),
                cache_control: None,
            }],
        );
        session.add_message(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: format!("answer-{turn}"),
                cache_control: None,
            }],
        );
    }

    let numbered = |session: &Session| -> Vec<String> {
        render_messages(session)
            .into_iter()
            .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
            .map(|m| m.content)
            .collect()
    };

    let full = numbered(&session);
    assert_eq!(full.len(), 12);
    let before_rewind = session.messages.clone();

    // First rewind: to entry 4 ("answer-2").
    let targets = session.rewind_target_stored_indices();
    session.truncate_messages(targets[4 - 1] + 1);
    assert_eq!(numbered(&session).len(), 4);
    assert_eq!(numbered(&session).last().unwrap(), "answer-2");

    // Undo restores the full transcript, exactly as `Agent::undo_rewind` does.
    session.replace_messages(before_rewind);
    assert_eq!(
        numbered(&session),
        full,
        "undo must restore the original transcript"
    );

    // Second rewind to a *different, larger* N. The bug report says this lands
    // back on the first rewind's target; it must honour 11.
    let targets = session.rewind_target_stored_indices();
    assert_eq!(
        targets.len(),
        12,
        "targets must be recomputed against the restored transcript"
    );
    session.truncate_messages(targets[11 - 1] + 1);

    let after = numbered(&session);
    assert_eq!(
        after.len(),
        11,
        "rewind 11 must keep 11 entries, not fall back to the earlier target of 4"
    );
    assert_eq!(after.last().unwrap(), "prompt-6");
    assert_eq!(session.rewind_target_count(), 11);
}

include!("cases/persistence.rs");
include!("cases/transcript.rs");
