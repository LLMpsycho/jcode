#[test]
fn load_sessions_includes_claude_code_sessions_from_external_home() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp dir");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

    let project_dir = temp.path().join("external/.claude/projects/demo-project");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let transcript_path = project_dir.join("claude-session-123.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"Investigate the login bug\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"parentUuid\":\"u1\",\"message\":{\"role\":\"assistant\",\"content\":\"I can help with that.\"}}\n"
        ),
    )
    .expect("write transcript");

    std::fs::write(
        project_dir.join("sessions-index.json"),
        format!(
            concat!(
                "{{\"version\":1,\"entries\":[",
                "{{\"sessionId\":\"claude-session-123\",",
                "\"fullPath\":\"{}\",",
                "\"firstPrompt\":\"Investigate the login bug\",",
                "\"summary\":\"Investigate the login bug\",",
                "\"messageCount\":2,",
                "\"created\":\"2026-04-04T12:00:00Z\",",
                "\"modified\":\"2026-04-04T12:05:00Z\",",
                "\"projectPath\":\"/tmp/demo-project\"",
                "}}]}}"
            ),
            transcript_path.display()
        ),
    )
    .expect("write index");

    let sessions = load_sessions().expect("load sessions");
    let session = sessions
        .iter()
        .find(|session| {
            matches!(
                session.resume_target,
                ResumeTarget::ClaudeCodeSession { .. }
            )
        })
        .expect("claude session present");

    assert_eq!(session.source, SessionSource::ClaudeCode);
    assert_eq!(session.id, "claude:claude-session-123");
    assert_eq!(session.short_name, "demo-project");
    assert_eq!(session.title, "Investigate the login bug");
    assert_eq!(session.message_count, 2);
    assert_eq!(session.working_dir.as_deref(), Some("/tmp/demo-project"));
}
/// End-to-end counterpart to the unit tests above (issue #674): with
/// `external_sessions` off, a discoverable Claude Code transcript must not
/// appear in the picker, while jcode's own sessions still do.
#[test]
fn load_sessions_hides_external_sessions_when_opted_out() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp dir");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _opt_out = EnvVarGuard::set_str("JCODE_EXTERNAL_SESSIONS", "0");
    crate::config::Config::invalidate_cache();
    invalidate_session_list_cache();

    let project_dir = temp.path().join("external/.claude/projects/demo-project");
    std::fs::create_dir_all(&project_dir).expect("create project dir");
    let transcript_path = project_dir.join("claude-session-456.jsonl");
    std::fs::write(
        &transcript_path,
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"hidden\"}}\n",
    )
    .expect("write transcript");
    std::fs::write(
        project_dir.join("sessions-index.json"),
        format!(
            concat!(
                "{{\"version\":1,\"entries\":[",
                "{{\"sessionId\":\"claude-session-456\",",
                "\"fullPath\":\"{}\",",
                "\"firstPrompt\":\"hidden\",",
                "\"summary\":\"hidden\",",
                "\"messageCount\":1,",
                "\"created\":\"2026-04-04T12:00:00Z\",",
                "\"modified\":\"2026-04-04T12:05:00Z\",",
                "\"projectPath\":\"/tmp/demo-project\"",
                "}}]}}"
            ),
            transcript_path.display()
        ),
    )
    .expect("write index");

    let sessions = load_sessions().expect("load sessions");
    assert!(
        !sessions
            .iter()
            .any(|session| session.source != SessionSource::Jcode),
        "external sessions must be hidden when display.external_sessions is false"
    );

    // And they come back when the opt-out is lifted.
    drop(_opt_out);
    crate::config::Config::invalidate_cache();
    invalidate_session_list_cache();
    let sessions = load_sessions().expect("load sessions");
    assert!(
        sessions
            .iter()
            .any(|session| session.source == SessionSource::ClaudeCode),
        "external sessions must return when the opt-out is lifted"
    );
}
#[test]
fn load_claude_code_preview_reads_transcript_messages() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp dir");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

    let project_dir = temp.path().join("external/.claude/projects/demo-project");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let transcript_path = project_dir.join("claude-session-456.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Fix the flaky test\"}]}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"parentUuid\":\"u1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"I found the race condition\"}]}}\n"
        ),
    )
    .expect("write transcript");

    std::fs::write(
        project_dir.join("sessions-index.json"),
        format!(
            concat!(
                "{{\"version\":1,\"entries\":[",
                "{{\"sessionId\":\"claude-session-456\",",
                "\"fullPath\":\"{}\",",
                "\"firstPrompt\":\"Fix the flaky test\",",
                "\"messageCount\":2,",
                "\"created\":\"2026-04-04T12:00:00Z\",",
                "\"modified\":\"2026-04-04T12:05:00Z\"",
                "}}]}}"
            ),
            transcript_path.display()
        ),
    )
    .expect("write index");

    let preview = load_claude_code_preview("claude-session-456").expect("preview");
    assert_eq!(preview.len(), 2);
    assert_eq!(preview[0].role, "user");
    assert!(preview[0].content.contains("Fix the flaky test"));
    assert_eq!(preview[1].role, "assistant");
    assert!(preview[1].content.contains("I found the race condition"));
}
#[test]
fn load_sessions_includes_modern_codex_sessions() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp dir");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

    let codex_dir = temp.path().join("external/.codex/sessions/2026/04/05");
    std::fs::create_dir_all(&codex_dir).expect("create codex dir");

    let transcript_path = codex_dir.join("rollout-2026-04-05T19-00-00-test.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"timestamp\":\"2026-04-05T19:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"019d-codex-test\",\"timestamp\":\"2026-04-05T18:59:00Z\",\"cwd\":\"/tmp/codex-demo\",\"source\":\"cli\"}}\n",
            "{\"timestamp\":\"2026-04-05T19:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /tmp/codex-demo\\n\\n<INSTRUCTIONS>ignored</INSTRUCTIONS>\"}]}}\n",
            "{\"timestamp\":\"2026-04-05T19:00:03Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix the OpenAI usage widget\"}]}}\n",
            "{\"timestamp\":\"2026-04-05T19:00:05Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"I found the issue.\"}]}}\n"
        ),
    )
    .expect("write codex transcript");

    let sessions = load_sessions().expect("load sessions");
    let session = sessions
        .iter()
        .find(|session| matches!(session.resume_target, ResumeTarget::CodexSession { .. }))
        .expect("codex session present");

    assert_eq!(session.source, SessionSource::Codex);
    assert_eq!(session.id, "codex:019d-codex-test");
    assert_eq!(session.title, "Codex session 019d-cod");
    assert_eq!(session.message_count, 0);
    assert_eq!(session.user_message_count, 0);
    assert_eq!(session.assistant_message_count, 0);
    assert_eq!(session.working_dir.as_deref(), Some("/tmp/codex-demo"));
}
#[test]
fn load_codex_preview_preserves_blank_line_between_tool_transcript_and_followup_prose() {
    let temp = tempfile::tempdir().expect("temp dir");
    let transcript_path = temp.path().join("codex-preview.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"timestamp\":\"2026-04-10T19:05:54.536Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"019d-preview-test\",\"timestamp\":\"2026-04-10T19:05:54.536Z\"}}\n",
            "{\"timestamp\":\"2026-04-10T19:05:55.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[",
            "{\"type\":\"output_text\",\"text\":\"I’m cleaning up the last leftover warning from the reverted experiment, then I’ll commit the second pass as the debounced large-swarm snapshot optimization.\\n  ✓ batch 3 calls · 174 tok\\n    ✓ apply_patch src/server/swarm.rs (30 lines) · 10 tok\\n    ✓ bash $ cargo fmt --all · 27 tok\\n    ✓ bash $ git add … status broadcasts\"},",
            "{\"type\":\"output_text\",\"text\":\"I landed the second pass as commit 158f6ac, and I’m not stopping there.\"}",
            "]}}\n"
        ),
    )
    .expect("write codex transcript");

    let preview = load_codex_preview_from_path(&transcript_path).expect("preview");
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].role, "assistant");
    assert!(
        preview[0].content.contains(
            "✓ bash $ git add … status broadcasts\n\nI landed the second pass as commit 158f6ac"
        ),
        "preview content should preserve a blank line between tool transcript and followup prose: {:?}",
        preview[0].content
    );
}
#[test]
fn load_codex_preview_reads_only_tail_of_large_transcript() {
    // A transcript far larger than the tail cap should still produce a preview
    // of the most-recent messages, parsed from only the tail slice. This is the
    // regression guard for the picker-navigation lag: previews must not depend
    // on parsing the whole (multi-MB) file.
    let temp = tempfile::tempdir().expect("temp dir");
    let transcript_path = temp.path().join("rollout-big.jsonl");

    let mut contents = String::new();
    // session_meta header line (always skipped).
    contents.push_str(
        "{\"timestamp\":\"2026-04-10T19:05:54.536Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"019d-big\"}}\n",
    );
    // Padding messages near the head that must NOT appear in the preview once
    // the file exceeds the tail cap.
    for i in 0..50_000 {
        contents.push_str(&format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"old padding message {i} aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}]}}}}\n",
        ));
    }
    assert!(
        contents.len() as u64 > EXTERNAL_PREVIEW_TAIL_BYTES,
        "test transcript must exceed the tail cap"
    );
    // Distinctive recent messages at the very end.
    contents.push_str(
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"RECENT_USER_MARKER\"}]}}\n",
    );
    contents.push_str(
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"RECENT_ASSISTANT_MARKER\"}]}}\n",
    );
    std::fs::write(&transcript_path, &contents).expect("write big transcript");

    let preview = load_codex_preview_from_path(&transcript_path).expect("preview");
    // Preview is capped at 20 messages.
    assert!(
        preview.len() <= 20,
        "preview should be capped, got {}",
        preview.len()
    );
    // The most-recent markers must be present.
    let last_two = &preview[preview.len().saturating_sub(2)..];
    assert!(
        last_two
            .iter()
            .any(|m| m.content.contains("RECENT_USER_MARKER"))
    );
    assert!(
        last_two
            .iter()
            .any(|m| m.content.contains("RECENT_ASSISTANT_MARKER"))
    );
    // The head padding must have been skipped (not parsed from the tail slice).
    assert!(
        !preview
            .iter()
            .any(|m| m.content.contains("old padding message 0 ")),
        "head messages should not appear when only the tail is read"
    );
}
#[test]
fn load_claude_code_preview_reads_only_tail_of_large_transcript() {
    let temp = tempfile::tempdir().expect("temp dir");
    let transcript_path = temp.path().join("claude-big.jsonl");

    let mut contents = String::new();
    for i in 0..50_000 {
        contents.push_str(&format!(
            "{{\"type\":\"assistant\",\"uuid\":\"a{i}\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"old padding message {i} bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}}]}}}}\n",
        ));
    }
    assert!(
        contents.len() as u64 > EXTERNAL_PREVIEW_TAIL_BYTES,
        "test transcript must exceed the tail cap"
    );
    contents.push_str(
        "{\"type\":\"user\",\"uuid\":\"u_last\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"RECENT_USER_MARKER\"}]}}\n",
    );
    contents.push_str(
        "{\"type\":\"assistant\",\"uuid\":\"a_last\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"RECENT_ASSISTANT_MARKER\"}]}}\n",
    );
    std::fs::write(&transcript_path, &contents).expect("write big transcript");

    let preview = load_claude_code_preview_from_path(&transcript_path).expect("preview");
    assert!(
        preview.len() <= 20,
        "preview should be capped, got {}",
        preview.len()
    );
    let last_two = &preview[preview.len().saturating_sub(2)..];
    assert!(
        last_two
            .iter()
            .any(|m| m.content.contains("RECENT_USER_MARKER"))
    );
    assert!(
        last_two
            .iter()
            .any(|m| m.content.contains("RECENT_ASSISTANT_MARKER"))
    );
    assert!(
        !preview
            .iter()
            .any(|m| m.content.contains("old padding message 0 ")),
        "head messages should not appear when only the tail is read"
    );
}
