use super::*;
use std::io::Write;

/// Faithful, real-home measurement of the per-frame onboarding cost.
/// Ignored by default (depends on local ~/.codex and ~/.claude contents).
/// Run with:
///   cargo test -p jcode-tui --lib onboarding_suggestion_scan_cost -- --ignored --nocapture
#[test]
#[ignore]
fn onboarding_suggestion_scan_cost() {
    use std::time::Instant;

    // Cold: the uncached scan that reads + JSON-parses the newest external
    // transcripts. This is the work that used to run several times per frame.
    let cold_start = Instant::now();
    let cold = latest_external_cli_continuation_prompt_uncached();
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.0;

    // Warm: the cached front-end the onboarding screen actually calls. Prime
    // the cache once, then measure repeated calls (as a redrawing frame does).
    let _ = latest_external_cli_continuation_prompt();
    let runs = 1000;
    let warm_start = Instant::now();
    let mut warm = None;
    for _ in 0..runs {
        warm = latest_external_cli_continuation_prompt();
    }
    let warm_ms = warm_start.elapsed().as_secs_f64() * 1000.0 / runs as f64;

    eprintln!(
        "external-cli continuation prompt: cold(uncached)={cold_ms:.1} ms, \
             warm(cached, avg of {runs})={warm_ms:.4} ms; cold_some={}, warm_some={}",
        cold.is_some(),
        warm.is_some()
    );
}

#[test]
fn parses_claude_code_jsonl_with_session_path_and_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("session.jsonl");
    std::fs::write(
            &path,
            r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-05-28T02:30:54.188Z","sessionId":"abc","content":"queued prompt"}
{"type":"user","message":{"role":"user","content":"Organize my windows by project"},"cwd":"/home/jeremy","sessionId":"abc"}
{"type":"last-prompt","lastPrompt":"fallback prompt","sessionId":"abc"}
"#,
        )
        .expect("write fixture");

    let candidate = suggestion_candidate_from_jsonl(&path, "Claude Code", SystemTime::UNIX_EPOCH)
        .expect("candidate");
    assert_eq!(candidate.source, "Claude Code");
    assert_eq!(candidate.path, path);
    assert_eq!(candidate.session_id.as_deref(), Some("abc"));
    assert_eq!(candidate.working_dir.as_deref(), Some("/home/jeremy"));
    assert_eq!(
        candidate.context.as_deref(),
        Some("Organize my windows by project")
    );
}

#[test]
fn parses_codex_input_text_blocks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("codex.jsonl");
    std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"sid","cwd":"/home/jeremy/jcode"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"check in on jcode"}]}}
"#,
        )
        .expect("write fixture");

    let candidate =
        suggestion_candidate_from_jsonl(&path, "Codex", SystemTime::UNIX_EPOCH).expect("candidate");
    assert_eq!(candidate.session_id.as_deref(), Some("sid"));
    assert_eq!(candidate.working_dir.as_deref(), Some("/home/jeremy/jcode"));
    assert_eq!(candidate.context.as_deref(), Some("check in on jcode"));
}

#[test]
fn discovery_sorts_after_collecting_nested_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let old_dir = temp.path().join("a");
    let new_dir = temp.path().join("z/deep");
    std::fs::create_dir_all(&old_dir).expect("old dir");
    std::fs::create_dir_all(&new_dir).expect("new dir");
    std::fs::write(
        old_dir.join("old.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"old"},"sessionId":"old"}"#,
    )
    .expect("old fixture");
    std::thread::sleep(std::time::Duration::from_millis(20));

    let new_path = new_dir.join("new.jsonl");
    std::fs::write(
        &new_path,
        r#"{"type":"user","message":{"role":"user","content":"new"},"sessionId":"new"}"#,
    )
    .expect("new fixture");
    // Ensure the newer file has a strictly later mtime even on coarse filesystems.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&new_path)
        .expect("open new");
    writeln!(file).expect("touch new");

    let candidates = latest_jsonl_suggestion_candidates(temp.path(), "Claude Code", 1);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].context.as_deref(), Some("new"));
}

/// Every slash command must be registered exactly once. Duplicate entries
/// mean two different handlers claim the same name, so which one runs
/// depends on dispatch order rather than on the registry the palette and
/// `/help` show the user.
#[test]
fn registered_commands_have_no_duplicate_names() {
    let mut seen = std::collections::HashSet::new();
    let duplicates: Vec<&str> = REGISTERED_COMMANDS
        .iter()
        .filter(|command| !seen.insert(command.name))
        .map(|command| command.name)
        .collect();
    assert!(
        duplicates.is_empty(),
        "duplicate slash command registrations: {:?}",
        duplicates
    );
}

/// Aliases users can actually type must be discoverable through the
/// registry, otherwise autocomplete silently omits working commands.
#[test]
fn known_aliases_are_registered() {
    let names: std::collections::HashSet<&str> =
        REGISTERED_COMMANDS.iter().map(|c| c.name).collect();
    for alias in [
        "/keybindings",
        "/commit-and-push",
        "/resume-all",
        "/hotkeys",
        "/keys",
    ] {
        assert!(names.contains(alias), "{alias} is not registered");
    }
}
