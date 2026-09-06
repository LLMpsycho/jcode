use super::*;
use chrono::{Duration as ChronoDuration, Utc};
use std::io::Write;
use std::time::{Duration as StdDuration, SystemTime};

fn write_session_file_with_mtime(
    path: impl AsRef<std::path::Path>,
    content: &str,
    modified_secs: u64,
) {
    let mut file = std::fs::File::create(path.as_ref()).expect("create session file");
    file.write_all(content.as_bytes())
        .expect("write session file");
    file.set_modified(SystemTime::UNIX_EPOCH + StdDuration::from_secs(modified_secs))
        .expect("set modified time");
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn make_session(id: &str, short_name: &str, is_debug: bool, status: SessionStatus) -> SessionInfo {
    make_session_with_flags(id, short_name, is_debug, false, status)
}

fn make_session_with_flags(
    id: &str,
    short_name: &str,
    is_debug: bool,
    is_canary: bool,
    status: SessionStatus,
) -> SessionInfo {
    let now = Utc::now();
    let title = "Test session".to_string();
    let working_dir = Some("/tmp".to_string());
    let messages_preview = vec![
        PreviewMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        },
        PreviewMessage {
            role: "assistant".to_string(),
            content: "world".to_string(),
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        },
    ];
    let search_index = build_search_index(
        id,
        short_name,
        &title,
        working_dir.as_deref(),
        None,
        &messages_preview,
    );

    SessionInfo {
        id: id.to_string(),
        parent_id: None,
        short_name: short_name.to_string(),
        icon: "🧪".to_string(),
        title,
        message_count: 2,
        user_message_count: 1,
        assistant_message_count: 1,
        created_at: now - ChronoDuration::minutes(5),
        last_message_time: now - ChronoDuration::minutes(1),
        last_active_at: Some(now - ChronoDuration::minutes(1)),
        working_dir,
        model: None,
        provider_key: None,
        is_canary,
        is_debug,
        saved: false,
        save_label: None,
        status,
        needs_catchup: false,
        estimated_tokens: 200,
        first_user_prompt: messages_preview
            .iter()
            .find(|msg| msg.role == "user" && !msg.content.trim().is_empty())
            .map(|msg| msg.content.clone()),
        messages_preview,
        search_index,
        server_name: None,
        server_icon: None,
        source: SessionSource::Jcode,
        resume_target: ResumeTarget::JcodeSession {
            session_id: id.to_string(),
        },
        external_path: None,
    }
}

#[test]
fn test_format_estimated_tokens_uses_compact_units() {
    assert_eq!(SessionPicker::format_estimated_tokens(0), "~0 tok");
    assert_eq!(SessionPicker::format_estimated_tokens(999), "~999 tok");
    assert_eq!(SessionPicker::format_estimated_tokens(1_000), "~1k tok");
    assert_eq!(SessionPicker::format_estimated_tokens(1_234), "~1.2k tok");
    assert_eq!(SessionPicker::format_estimated_tokens(12_345), "~12k tok");
    assert_eq!(SessionPicker::format_estimated_tokens(999_500), "~1M tok");
    assert_eq!(
        SessionPicker::format_estimated_tokens(1_234_567),
        "~1.2M tok"
    );
    assert_eq!(
        SessionPicker::format_estimated_tokens(1_234_567_890),
        "~1.2B tok"
    );
    assert_eq!(
        SessionPicker::format_estimated_tokens(1_234_567_890_123),
        "~1.2T tok"
    );
}

#[test]
fn test_session_item_uses_single_primary_title_line() {
    let mut session = make_session(
        "session_primary_title",
        "rhino",
        false,
        SessionStatus::Closed,
    );
    session.title = "Generated release planning".to_string();
    session.estimated_tokens = 1_234_567;
    let picker = SessionPicker::new(vec![session.clone()]);

    let rows = picker.render_session_item_lines(&session, false);
    let text_rows: Vec<String> = rows.iter().map(line_text).collect();

    // The title must appear on exactly one row (the primary line); other rows
    // (stats, prompt preview, created/dir) must not repeat it.
    assert_eq!(
        text_rows
            .iter()
            .filter(|row| row.contains("Generated release planning"))
            .count(),
        1,
        "title should render on exactly one row: {text_rows:?}"
    );
    assert!(text_rows[0].contains("Generated release planning"));
    assert!(
        text_rows[1..]
            .iter()
            .all(|row| !row.contains("Generated release planning")),
        "title should only be rendered on the primary row: {text_rows:?}"
    );
    assert!(
        text_rows.iter().all(|row| !row.contains("rhino")),
        "memorable short name should remain searchable but not take display space: {text_rows:?}"
    );
    assert!(text_rows[1].contains("~1.2M tok"));
}

#[test]
fn test_status_inference() {
    // Load sessions and ensure status display works
    let sessions = load_sessions().unwrap();
    for session in &sessions {
        let _ = session.status.display();
    }
}

#[test]
fn test_collect_recent_session_stems_skips_empty_recent_sessions() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    write_session_file_with_mtime(
        dir.path().join("session_alpha_1000.json"),
        r#"{"messages":[{"role":"user","content":"hi"}]}"#,
        1000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_beta_2000.json"),
        r#"{"messages":[]}"#,
        2000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_gamma_3000.json"),
        r#"{"messages":[{"role":"user","content":"hello"}]}"#,
        3000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_delta_4000.json"),
        r#"{"messages":[]}"#,
        4000,
    );

    let stems = collect_recent_session_stems(dir.path(), 2).expect("collect stems");
    assert_eq!(stems, vec!["session_gamma_3000", "session_alpha_1000"]);
}

#[test]
fn test_collect_recent_session_stems_skips_system_context_only_sessions() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    write_session_file_with_mtime(
        dir.path().join("session_empty_context_9000.json"),
        r##"{"messages":[{"role":"user","display_role":"system","content":[{"type":"text","text":"<system-reminder>\n# Session Context\n</system-reminder>"}]}]}"##,
        9000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_real_1000.json"),
        r#"{"messages":[{"role":"user","content":"real prompt"}]}"#,
        1000,
    );

    let stems = collect_recent_session_stems(dir.path(), 1).expect("collect stems");
    assert_eq!(stems, vec!["session_real_1000"]);
}

#[test]
fn test_collect_recent_session_stems_keeps_system_context_with_visible_journal_turn() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let stem = "session_context_then_journal_9000";

    write_session_file_with_mtime(
        dir.path().join(format!("{stem}.json")),
        r##"{"messages":[{"role":"user","display_role":"system","content":[{"type":"text","text":"<system-reminder>\n# Session Context\n</system-reminder>"}]}]}"##,
        1000,
    );
    write_session_file_with_mtime(
        dir.path().join(format!("{stem}.journal.jsonl")),
        r#"{"meta":{"updated_at":"2026-05-01T00:00:00Z"},"append_messages":[{"role":"user","content":"real prompt from journal"}]}"#,
        9000,
    );

    let stems = collect_recent_session_stems(dir.path(), 1).expect("collect stems");
    assert_eq!(stems, vec![stem]);
}

#[test]
fn test_collect_recent_session_stems_uses_timestamp_as_mtime_tiebreaker() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    write_session_file_with_mtime(
        dir.path().join("session_old_1111.json"),
        r#"{"messages":[{"role":"user","content":"old"}]}"#,
        1000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_mid_2222.json"),
        r#"{"messages":[{"role":"user","content":"mid"}]}"#,
        1000,
    );
    write_session_file_with_mtime(
        dir.path().join("session_new_3333.json"),
        r#"{"messages":[{"role":"user","content":"new"}]}"#,
        1000,
    );

    let stems = collect_recent_session_stems(dir.path(), 3).expect("collect stems");
    assert_eq!(
        stems,
        vec!["session_new_3333", "session_mid_2222", "session_old_1111"]
    );
}

#[test]
fn test_collect_recent_session_stems_prefers_recently_modified_long_running_session() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    for idx in 0..120 {
        write_session_file_with_mtime(
            dir.path().join(format!(
                "session_newer_created_{:013}.json",
                2_000_000 + idx
            )),
            r#"{"messages":[{"role":"user","content":"short newer-created session"}]}"#,
            1000 + idx,
        );
    }

    let target = "session_long_running_0000000000500";
    write_session_file_with_mtime(
        dir.path().join(format!("{target}.json")),
        r#"{"messages":[{"role":"user","content":"old creation time, recently active"}]}"#,
        10_000,
    );

    let stems = collect_recent_session_stems(dir.path(), 100).expect("collect stems");
    assert_eq!(stems.first().map(String::as_str), Some(target));
    assert!(stems.iter().any(|stem| stem == target));
}

#[test]
fn test_toggle_test_sessions_rebuilds_visibility() {
    let normal = make_session("session_normal", "normal", false, SessionStatus::Closed);
    let debug = make_session("session_debug", "debug", true, SessionStatus::Closed);

    let mut picker = SessionPicker::new(vec![normal.clone(), debug.clone()]);

    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(!picker.show_test_sessions);
    assert_eq!(picker.hidden_test_count, 1);

    picker.toggle_test_sessions();
    assert!(picker.show_test_sessions);
    assert_eq!(picker.visible_sessions.len(), 2);
    assert_eq!(picker.hidden_test_count, 0);

    picker.toggle_test_sessions();
    assert!(!picker.show_test_sessions);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert_eq!(picker.hidden_test_count, 1);
}

#[test]
fn test_new_grouped_hides_debug_by_default() {
    let normal = make_session("session_normal", "normal", false, SessionStatus::Closed);
    let debug = make_session("session_debug", "debug", true, SessionStatus::Closed);
    let canary = make_session_with_flags(
        "session_canary",
        "canary",
        false,
        true,
        SessionStatus::Closed,
    );
    let orphan_normal = make_session(
        "orphan_normal",
        "orphan-normal",
        false,
        SessionStatus::Closed,
    );
    let orphan_debug = make_session("orphan_debug", "orphan-debug", true, SessionStatus::Closed);

    let groups = vec![ServerGroup {
        name: "main".to_string(),
        icon: "🛰".to_string(),
        version: "v0.1.0".to_string(),
        git_hash: "abc1234".to_string(),
        is_running: true,
        sessions: vec![normal.clone(), debug.clone(), canary.clone()],
    }];

    let mut picker = SessionPicker::new_grouped(groups, vec![orphan_normal, orphan_debug]);

    assert!(!picker.show_test_sessions);
    // Canary sessions are now visible by default, only debug sessions are hidden
    assert_eq!(picker.visible_sessions.len(), 3); // normal + canary + orphan_normal
    assert!(picker.visible_session_iter().all(|s| !s.is_debug));
    assert_eq!(picker.hidden_test_count, 2); // debug + orphan_debug

    picker.toggle_test_sessions();
    assert!(picker.show_test_sessions);
    assert_eq!(picker.visible_sessions.len(), 5);
    assert_eq!(picker.hidden_test_count, 0);
    assert!(picker.visible_session_iter().any(|s| s.is_debug));
    assert!(picker.visible_session_iter().any(|s| s.is_canary));
}

#[test]
fn test_new_grouped_without_servers_shows_orphan_sessions() {
    let normal = make_session("session_normal", "normal", false, SessionStatus::Closed);
    let debug = make_session("session_debug", "debug", true, SessionStatus::Closed);

    let mut picker = SessionPicker::new_grouped(Vec::new(), vec![normal, debug]);

    assert!(!picker.show_test_sessions);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(picker.visible_session_iter().all(|s| !s.is_debug));
    assert_eq!(picker.hidden_test_count, 1);
    assert_eq!(picker.items.len(), 1);
    assert_eq!(picker.list_state.selected(), Some(0));

    picker.toggle_test_sessions();
    assert!(picker.show_test_sessions);
    assert_eq!(picker.visible_sessions.len(), 2);
    assert_eq!(picker.hidden_test_count, 0);
    assert_eq!(picker.items.len(), 2);
    assert!(picker.visible_session_iter().any(|s| s.is_debug));
}

#[test]
fn test_crash_reason_line_for_crashed_sessions() {
    let crashed = make_session(
        "session_crash",
        "crash",
        false,
        SessionStatus::Crashed {
            message: Some("Terminal or window closed (SIGHUP)".to_string()),
        },
    );
    let line = SessionPicker::crash_reason_line(&crashed).expect("crash reason should render");
    let text: String = line
        .spans
        .into_iter()
        .map(|s| s.content.to_string())
        .collect();
    assert!(text.contains("reason:"));
    assert!(text.contains("SIGHUP"));
}

#[test]
fn test_batch_restore_detection_excludes_already_recovered_parent_sessions() {
    let crashed = make_session(
        "session_crash_source",
        "crash-source",
        false,
        SessionStatus::Crashed {
            message: Some("boom".to_string()),
        },
    );

    let mut recovered = make_session(
        "session_recovery_rec123",
        "recovered",
        false,
        SessionStatus::Closed,
    );
    recovered.parent_id = Some(crashed.id.clone());

    let picker = SessionPicker::new(vec![crashed, recovered]);

    assert!(picker.crashed_sessions.is_none());
    assert!(picker.crashed_session_ids.is_empty());
}

#[test]
fn test_grouped_batch_restore_uses_last_active_at_and_includes_debug_sessions() {
    let now = Utc::now();

    let mut recent_normal = make_session(
        "session_recent_normal",
        "recent-normal",
        false,
        SessionStatus::Crashed {
            message: Some("recent crash".to_string()),
        },
    );
    recent_normal.last_message_time = now - ChronoDuration::minutes(10);
    recent_normal.last_active_at = Some(now - ChronoDuration::seconds(10));

    let mut recent_debug = make_session(
        "session_recent_debug",
        "recent-debug",
        true,
        SessionStatus::Crashed {
            message: Some("debug crash".to_string()),
        },
    );
    recent_debug.last_message_time = now - ChronoDuration::minutes(9);
    recent_debug.last_active_at = Some(now - ChronoDuration::seconds(20));

    let mut stale_crash = make_session(
        "session_stale_crash",
        "stale-crash",
        false,
        SessionStatus::Crashed {
            message: Some("old crash".to_string()),
        },
    );
    stale_crash.last_message_time = now - ChronoDuration::seconds(30);
    stale_crash.last_active_at = Some(now - ChronoDuration::minutes(3));

    let picker = SessionPicker::new_grouped(
        vec![ServerGroup {
            name: "main".to_string(),
            icon: "🛰".to_string(),
            version: "v0.1.0".to_string(),
            git_hash: "abc1234".to_string(),
            is_running: true,
            sessions: vec![recent_normal.clone(), recent_debug.clone(), stale_crash],
        }],
        Vec::new(),
    );

    let crashed = picker
        .crashed_sessions
        .as_ref()
        .expect("expected eligible crashed sessions");

    assert_eq!(crashed.session_ids.len(), 2);
    assert_eq!(crashed.omitted_crashed_count, 1);
    assert!(crashed.session_ids.contains(&recent_normal.id));
    assert!(crashed.session_ids.contains(&recent_debug.id));
    assert!(
        !crashed
            .session_ids
            .iter()
            .any(|id| id == "session_stale_crash")
    );

    let mut picker = picker;
    let action = picker
        .handle_overlay_key(KeyCode::Char('R'), KeyModifiers::empty())
        .expect("restore group key should be handled");
    let OverlayAction::Selected(PickerResult::RestoreCrashedGroup(ids)) = action else {
        panic!("expected restore group action");
    };
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&recent_normal.id));
    assert!(ids.contains(&recent_debug.id));
    assert!(!ids.iter().any(|id| id == "session_stale_crash"));
}

#[test]
fn test_filter_mode_cycles_through_requested_session_sources() {
    let mut saved = make_session("session_saved", "saved", false, SessionStatus::Closed);
    saved.saved = true;
    saved.needs_catchup = true;

    let mut claude_code = make_session("claude:demo", "claude-code", false, SessionStatus::Closed);
    claude_code.source = SessionSource::ClaudeCode;
    claude_code.resume_target = ResumeTarget::ClaudeCodeSession {
        session_id: "claude-session-demo".to_string(),
        session_path: "/tmp/claude-session-demo.jsonl".to_string(),
    };

    let mut codex = make_session("session_codex", "codex", false, SessionStatus::Closed);
    codex.model = Some("gpt-5.3-codex".to_string());
    codex.source = SessionSource::Codex;

    let mut pi = make_session("session_pi", "pi", false, SessionStatus::Closed);
    pi.provider_key = Some("pi".to_string());
    pi.source = SessionSource::Pi;

    let mut opencode = make_session("session_opencode", "opencode", false, SessionStatus::Closed);
    opencode.provider_key = Some("opencode".to_string());
    opencode.source = SessionSource::OpenCode;

    let mut cursor = make_session("session_cursor", "cursor", false, SessionStatus::Closed);
    cursor.provider_key = Some("cursor".to_string());
    cursor.source = SessionSource::Cursor;

    let mut picker = SessionPicker::new(vec![saved, claude_code, codex, pi, opencode, cursor]);
    picker.all_sessions[0].working_dir = Some("/work/project".to_string());
    picker.set_current_dir(Some("/work/project/".to_string()));
    picker.rebuild_items();

    assert_eq!(picker.filter_mode, SessionFilterMode::All);
    assert_eq!(picker.visible_sessions.len(), 6);

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::CurrentDir);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(|session| picker.session_in_current_dir(session))
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::CatchUp);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(|session| session.needs_catchup)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::Saved);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(picker.visible_session_iter().all(|session| session.saved));
    assert_eq!(picker.items.len(), picker.visible_sessions.len());

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::Active);
    // No live processes own these synthetic sessions, so the Active view is
    // empty in tests.
    assert_eq!(picker.visible_sessions.len(), 0);

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::ClaudeCode);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(SessionPicker::session_is_claude_code)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::Codex);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(SessionPicker::session_is_codex)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::Pi);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(SessionPicker::session_is_pi)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::OpenCode);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(SessionPicker::session_is_open_code)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::Cursor);
    assert_eq!(picker.visible_sessions.len(), 1);
    assert!(
        picker
            .visible_session_iter()
            .all(SessionPicker::session_is_cursor)
    );

    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::All);
    assert_eq!(picker.visible_sessions.len(), 6);
}

#[test]
fn test_filter_mode_keyboard_shortcuts_cycle_both_directions() {
    let mut picker = SessionPicker::new(vec![make_session(
        "session_saved",
        "saved",
        false,
        SessionStatus::Closed,
    )]);
    picker
        .handle_overlay_key(KeyCode::Char('s'), KeyModifiers::empty())
        .unwrap();
    assert_eq!(picker.filter_mode, SessionFilterMode::CurrentDir);

    picker
        .handle_overlay_key(KeyCode::Char('S'), KeyModifiers::empty())
        .unwrap();
    assert_eq!(picker.filter_mode, SessionFilterMode::All);
}

fn live_presence(session_id: &str, streaming: bool) -> crate::session::SessionPresence {
    crate::session::SessionPresence {
        session_id: session_id.to_string(),
        pid: std::process::id(),
        streaming,
        streaming_since: streaming
            .then(|| std::time::SystemTime::now() - std::time::Duration::from_secs(90)),
        internal: false,
    }
}

#[test]
fn test_active_filter_shows_only_live_sessions_ready_before_working() {
    let live_working = make_session("session_working", "alpha", false, SessionStatus::Active);
    let live_ready = make_session("session_ready", "beta", false, SessionStatus::Active);
    let dead = make_session("session_dead", "dead", false, SessionStatus::Closed);

    let mut picker = SessionPicker::new(vec![live_working, live_ready, dead]);
    picker.activate_active_filter();
    picker.set_live_presence_for_test(vec![
        live_presence("session_working", true),
        live_presence("session_ready", false),
    ]);

    let visible: Vec<&str> = picker
        .visible_session_iter()
        .map(|session| session.id.as_str())
        .collect();
    // Only live sessions appear; the ready one is triaged above the working one.
    assert_eq!(visible, vec!["session_ready", "session_working"]);

    let ready = picker
        .visible_session_iter()
        .find(|session| session.id == "session_ready")
        .expect("ready visible");
    let working = picker
        .visible_session_iter()
        .find(|session| session.id == "session_working")
        .expect("working visible");
    assert!(!picker.session_is_streaming(ready));
    assert!(picker.session_is_streaming(working));
    assert!(picker.session_streaming_duration(working).is_some());
}

#[test]
fn test_active_rows_render_working_and_ready_badges() {
    let live_working = make_session("session_working", "alpha", false, SessionStatus::Active);
    let live_ready = make_session("session_ready", "beta", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![live_working, live_ready]);
    picker.activate_active_filter();
    picker.set_live_presence_for_test(vec![
        live_presence("session_working", true),
        live_presence("session_ready", false),
    ]);

    let working = picker
        .visible_session_iter()
        .find(|session| session.id == "session_working")
        .cloned()
        .expect("working visible");
    let ready = picker
        .visible_session_iter()
        .find(|session| session.id == "session_ready")
        .cloned()
        .expect("ready visible");

    let working_lines = picker.render_session_item_lines(&working, false);
    let working_text = working_lines
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        working_text.contains("working 1m"),
        "expected working badge with duration, got: {working_text}"
    );

    let first_frame = picker
        .render_session_item_lines_at_frame(&working, false, 0)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    let second_frame = picker
        .render_session_item_lines_at_frame(&working, false, 1)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(first_frame.contains('⠋'));
    assert!(second_frame.contains('⠙'));
    assert_ne!(first_frame, second_frame, "running glyph should animate");
    assert!(picker.has_visible_running_sessions());

    let ready_lines = picker.render_session_item_lines(&ready, false);
    let ready_text = ready_lines
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        ready_text.contains("ready"),
        "expected ready badge, got: {ready_text}"
    );
    // Live presence overrides the stale persisted "closed" status.
    assert!(
        !ready_text.contains("closed"),
        "live session must not render as closed: {ready_text}"
    );
}

fn make_claude_session(session_id: &str) -> SessionInfo {
    let mut session = make_session(
        &format!("claude:{session_id}"),
        "claude live",
        false,
        SessionStatus::Closed,
    );
    session.source = SessionSource::ClaudeCode;
    session.provider_key = Some("claude-code".to_string());
    session.resume_target = ResumeTarget::ClaudeCodeSession {
        session_id: session_id.to_string(),
        session_path: format!("/tmp/{session_id}.jsonl"),
    };
    session
}

#[test]
fn live_claude_takeover_requires_explicit_key_and_confirmation() {
    let session = make_claude_session("claude-live-id");
    let target = session.resume_target.clone();
    let mut picker = SessionPicker::new(vec![session]);
    picker.set_live_presence_for_test(vec![live_presence("claude:claude-live-id", false)]);

    // Ordinary Enter remains an ordinary resume and never implies takeover.
    let ordinary = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    assert!(matches!(
        ordinary,
        OverlayAction::Selected(PickerResult::SelectedInCurrentTerminal(_))
            | OverlayAction::Selected(PickerResult::SelectedInNewTerminal(_))
    ));
    assert!(!picker.claude_takeover_confirmation_active_for_test());

    assert!(matches!(
        picker
            .handle_overlay_key(KeyCode::Char('T'), KeyModifiers::empty())
            .unwrap(),
        OverlayAction::Continue
    ));
    assert!(picker.claude_takeover_confirmation_active_for_test());

    // Cancellation does not emit an action.
    assert!(matches!(
        picker
            .handle_overlay_key(KeyCode::Esc, KeyModifiers::empty())
            .unwrap(),
        OverlayAction::Continue
    ));
    assert!(!picker.claude_takeover_confirmation_active_for_test());

    picker
        .handle_overlay_key(KeyCode::Char('T'), KeyModifiers::empty())
        .unwrap();
    let confirmed = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    assert!(matches!(
        confirmed,
        OverlayAction::Selected(PickerResult::TakeOverClaude(actual)) if actual == target
    ));
}

#[test]
fn live_claude_row_is_labeled_and_closed_claude_cannot_take_over() {
    let mut live = make_claude_session("live-row");
    let mut closed = make_claude_session("closed-row");
    live.last_message_time = Utc::now();
    closed.last_message_time = Utc::now() - ChronoDuration::minutes(1);
    let mut picker = SessionPicker::new(vec![live.clone(), closed]);
    picker.set_live_presence_for_test(vec![live_presence("claude:live-row", false)]);

    let rendered = picker
        .render_session_item_lines(&live, false)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("live Claude"), "rendered row: {rendered}");

    picker
        .handle_overlay_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    picker
        .handle_overlay_key(KeyCode::Char('T'), KeyModifiers::empty())
        .unwrap();
    assert!(!picker.claude_takeover_confirmation_active_for_test());
}

#[test]
fn test_current_session_row_is_labeled() {
    let session = make_session("session_self", "self", false, SessionStatus::Active);
    let mut picker = SessionPicker::new(vec![session.clone()]);
    picker.set_current_session_id(Some("session_self".to_string()));

    let lines = picker.render_session_item_lines(&session, false);
    let text = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
    assert!(
        text.contains("current"),
        "expected current label, got: {text}"
    );
}

#[test]
fn test_maybe_refresh_live_presence_throttles_and_detects_changes() {
    let session = make_session("session_live", "live", false, SessionStatus::Active);
    let mut picker = SessionPicker::new(vec![session]);
    picker.activate_active_filter();
    // Freshly injected snapshot: refresh is throttled, nothing changes.
    picker.set_live_presence_for_test(vec![live_presence("session_live", true)]);
    assert!(!picker.maybe_refresh_live_presence());
}

#[test]
fn test_space_selects_multiple_sessions_and_enter_returns_them() {
    let mut newer = make_session("session_newer", "newer", false, SessionStatus::Closed);
    let mut older = make_session("session_older", "older", false, SessionStatus::Closed);
    newer.last_message_time = Utc::now();
    older.last_message_time = Utc::now() - ChronoDuration::minutes(1);

    let mut picker = SessionPicker::new(vec![older, newer]);

    picker
        .handle_overlay_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    picker
        .handle_overlay_key(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    picker
        .handle_overlay_key(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();

    let action = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    match action {
        OverlayAction::Selected(PickerResult::SelectedInCurrentTerminal(ids)) => {
            assert_eq!(
                ids,
                vec![
                    ResumeTarget::JcodeSession {
                        session_id: "session_newer".to_string(),
                    },
                    ResumeTarget::JcodeSession {
                        session_id: "session_older".to_string(),
                    }
                ]
            );
        }
        other => panic!("expected selected sessions, got {other:?}"),
    }

    let alternate_action = picker
        .handle_overlay_key(KeyCode::Enter, KeyModifiers::CONTROL)
        .unwrap();

    match alternate_action {
        OverlayAction::Selected(PickerResult::SelectedInNewTerminal(ids)) => {
            assert_eq!(
                ids,
                vec![
                    ResumeTarget::JcodeSession {
                        session_id: "session_newer".to_string(),
                    },
                    ResumeTarget::JcodeSession {
                        session_id: "session_older".to_string(),
                    }
                ]
            );
        }
        other => panic!("expected alternate selected sessions, got {other:?}"),
    }
}

#[test]
fn test_rebuild_items_prunes_selected_sessions_hidden_by_filter() {
    let mut saved = make_session("session_saved", "saved", false, SessionStatus::Closed);
    saved.saved = true;
    let normal = make_session("session_normal", "normal", false, SessionStatus::Closed);

    let mut picker = SessionPicker::new(vec![saved, normal]);
    picker
        .selected_session_ids
        .insert("session_saved".to_string());
    picker
        .selected_session_ids
        .insert("session_normal".to_string());

    picker.filter_mode = SessionFilterMode::Saved;
    picker.rebuild_items();

    assert_eq!(picker.selected_session_ids.len(), 1);
    assert!(picker.selected_session_ids.contains("session_saved"));
}

#[test]
fn test_mouse_scroll_only_affects_hovered_pane_without_changing_focus() {
    let s1 = make_session("session_1", "one", false, SessionStatus::Closed);
    let s2 = make_session("session_2", "two", false, SessionStatus::Closed);
    let s3 = make_session("session_3", "three", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![s1, s2, s3]);

    picker.focus = PaneFocus::Preview;
    picker.scroll_offset = 7;
    picker.last_list_area = Some(Rect::new(0, 0, 20, 10));
    picker.last_preview_area = Some(Rect::new(20, 0, 20, 10));

    picker.handle_overlay_mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 5,
        row: 5,
        modifiers: KeyModifiers::empty(),
    });

    assert_eq!(picker.focus, PaneFocus::Preview);
    assert_eq!(picker.scroll_offset, 0);
    assert_eq!(
        picker.selected_session().map(|s| s.id.as_str()),
        Some("session_2")
    );
}

#[test]
fn test_keyboard_scroll_uses_sessions_focus_for_paging() {
    let s1 = make_session("session_1", "one", false, SessionStatus::Closed);
    let s2 = make_session("session_2", "two", false, SessionStatus::Closed);
    let s3 = make_session("session_3", "three", false, SessionStatus::Closed);
    let s4 = make_session("session_4", "four", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![s1, s2, s3, s4]);

    picker.focus = PaneFocus::Sessions;
    picker.scroll_offset = 6;

    let result = picker.handle_overlay_key(KeyCode::PageDown, KeyModifiers::empty());

    assert!(matches!(result, Ok(OverlayAction::Continue)));
    assert_eq!(picker.focus, PaneFocus::Sessions);
    assert_eq!(picker.scroll_offset, 0);
    assert_eq!(
        picker.selected_session().map(|s| s.id.as_str()),
        Some("session_1")
    );
}

#[test]
fn test_keyboard_scroll_uses_preview_focus_for_paging() {
    let s1 = make_session("session_1", "one", false, SessionStatus::Closed);
    let s2 = make_session("session_2", "two", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![s1, s2]);

    picker.focus = PaneFocus::Preview;

    let result = picker.handle_overlay_key(KeyCode::PageDown, KeyModifiers::empty());

    assert!(matches!(result, Ok(OverlayAction::Continue)));
    assert_eq!(picker.focus, PaneFocus::Preview);
    assert_eq!(picker.scroll_offset, PREVIEW_PAGE_SCROLL);
    assert_eq!(
        picker.selected_session().map(|s| s.id.as_str()),
        Some("session_2")
    );
}

/// Build a session with many short user/assistant turns so the preview overflows
/// a small viewport (used to exercise the preview scrollbar + sticky header).
fn make_session_with_many_turns(id: &str, turns: usize) -> SessionInfo {
    let mut session = make_session(id, id, false, SessionStatus::Closed);
    let mut preview = Vec::new();
    for i in 0..turns {
        preview.push(PreviewMessage {
            role: "user".to_string(),
            content: format!("user prompt number {i}"),
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        });
        preview.push(PreviewMessage {
            role: "assistant".to_string(),
            content: format!("assistant reply number {i}"),
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        });
    }
    session.first_user_prompt = preview.first().map(|m| m.content.clone());
    session.messages_preview = preview;
    session
}

fn buffer_text(picker: &mut SessionPicker, w: u16, h: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render picker");
    let buffer = terminal.backend().buffer().clone();
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

// ---------------------------------------------------------------------------
// Developer benchmarks: profile the operations exercised by the `/resume`
// overlay. These are `#[ignore]`d so they never run in CI; run them with:
//
//   cargo test -p jcode-tui --lib --release -- --ignored --nocapture benchmark_resume_op
//
// They print human-readable timing lines to stderr. They use synthetic
// sessions so they are deterministic and independent of the user's session
// store.
// ---------------------------------------------------------------------------

/// Build a synthetic preview message list that mimics a realistic conversation:
/// alternating user prompts and multi-paragraph markdown assistant replies. The
/// assistant content includes markdown (headers, lists, code) so it exercises
/// the same markdown render + wrap path as the real preview.
fn bench_preview_messages(turns: usize, assistant_paragraphs: usize) -> Vec<PreviewMessage> {
    let mut preview = Vec::with_capacity(turns * 2);
    for turn in 0..turns {
        preview.push(PreviewMessage {
            role: "user".to_string(),
            content: format!(
                "Prompt {turn}: can you refactor the session picker so that the preview \
                 pane does not rebuild and re-wrap every line on every single frame?"
            ),
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        });

        let mut body = String::new();
        body.push_str(&format!("## Response {turn}\n\n"));
        for para in 0..assistant_paragraphs {
            body.push_str(&format!(
                "Here is paragraph {para} of a longer answer that wraps across several \
                 terminal columns and therefore costs real work to lay out. It mentions \
                 `render_preview`, `wrap_lines`, and the scroll offset so the markdown \
                 renderer has inline code spans to style.\n\n"
            ));
            body.push_str("- a bullet point that also needs wrapping and styling\n");
            body.push_str("- another bullet with `inline_code` to style\n\n");
        }
        body.push_str("```rust\nlet scroll = self.scroll_offset as usize; // cached?\n```\n");
        preview.push(PreviewMessage {
            role: "assistant".to_string(),
            content: body,
            tool_calls: Vec::new(),
            tool_data: None,
            timestamp: None,
        });
    }
    preview
}

/// A session whose preview is large enough to overflow the viewport and require
/// scrolling (the case the user reported as slow).
fn bench_large_session(id: &str, turns: usize, assistant_paragraphs: usize) -> SessionInfo {
    let mut session = make_session(id, id, false, SessionStatus::Closed);
    let preview = bench_preview_messages(turns, assistant_paragraphs);
    session.first_user_prompt = preview.first().map(|m| m.content.clone());
    session.estimated_tokens = 4_000 * turns;
    session.message_count = turns * 2;
    session.user_message_count = turns;
    session.assistant_message_count = turns;
    session.messages_preview = preview;
    session
}

fn bench_render_full(picker: &mut SessionPicker, w: u16, h: u16) -> std::time::Duration {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    let start = std::time::Instant::now();
    terminal
        .draw(|frame| picker.render(frame))
        .expect("render picker");
    start.elapsed()
}

fn bench_render_preview_only(picker: &mut SessionPicker, area: Rect) -> std::time::Duration {
    // Render into a backend sized exactly to the area, placing it at the origin
    // (the preview/list rendering only depends on width/height, not x/y).
    let area = Rect::new(0, 0, area.width, area.height);
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    let start = std::time::Instant::now();
    terminal
        .draw(|frame| picker.render_preview(frame, area))
        .expect("render preview");
    start.elapsed()
}

fn bench_render_list_only(picker: &mut SessionPicker, area: Rect) -> std::time::Duration {
    let area = Rect::new(0, 0, area.width, area.height);
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    let start = std::time::Instant::now();
    terminal
        .draw(|frame| picker.render_session_list(frame, area))
        .expect("render list");
    start.elapsed()
}

fn bench_median(mut samples: Vec<std::time::Duration>) -> std::time::Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Any of the native scrollbar thumb glyphs (see `render_native_scrollbar`).
fn contains_scrollbar_glyph(text: &str) -> bool {
    text.contains('•') || text.contains('╷') || text.contains('╵') || text.contains('│')
}

include!("session_picker_tests/benchmarks.rs");
include!("session_picker_tests/onboarding.rs");
include!("session_picker_tests/preview_scroll.rs");
include!("session_picker_tests/search.rs");
