#[test]
#[ignore = "developer benchmark: times real /resume loading phases"]
fn benchmark_real_resume_loading_phases() {
    invalidate_session_list_cache();

    let sessions_dir = storage::jcode_dir().expect("jcode dir").join("sessions");
    let scan_limit = session_scan_limit();
    let candidate_limit = session_candidate_window(scan_limit);

    let phase_start = std::time::Instant::now();
    let candidates = if sessions_dir.exists() {
        collect_recent_session_candidates(&sessions_dir, candidate_limit)
            .expect("collect recent session candidates")
    } else {
        Vec::new()
    };
    let collect_candidates_elapsed = phase_start.elapsed();

    let mut sessions = Vec::new();
    let mut skipped_empty = 0usize;
    let mut skipped_imported = 0usize;
    let mut summary_errors = 0usize;
    let phase_start = std::time::Instant::now();
    for stem in &candidates {
        if sessions.len() >= scan_limit {
            let saved = sessions_dir.join(format!("{stem}.json"));
            if !session_snapshot_or_journal_has_saved_metadata(&saved) {
                continue;
            }
        }
        if stem.starts_with("imported_cc_")
            || stem.starts_with("imported_codex_")
            || stem.starts_with("imported_pi_")
            || stem.starts_with("imported_opencode_")
        {
            skipped_imported += 1;
            continue;
        }

        let path = sessions_dir.join(format!("{stem}.json"));
        match load_session_summary(&path) {
            Ok(summary) if summary.messages.visible_message_count > 0 => {
                sessions.push((stem.clone(), summary));
            }
            Ok(_) => skipped_empty += 1,
            Err(_) => summary_errors += 1,
        }
    }
    let jcode_summary_elapsed = phase_start.elapsed();

    let phase_start = std::time::Instant::now();
    let claude = load_external_claude_code_sessions(scan_limit);
    let claude_elapsed = phase_start.elapsed();

    let phase_start = std::time::Instant::now();
    let codex = load_external_codex_sessions(scan_limit);
    let codex_elapsed = phase_start.elapsed();

    let phase_start = std::time::Instant::now();
    let pi = load_external_pi_sessions(scan_limit);
    let pi_elapsed = phase_start.elapsed();

    let phase_start = std::time::Instant::now();
    let opencode = load_external_opencode_sessions(scan_limit);
    let opencode_elapsed = phase_start.elapsed();

    let phase_start = std::time::Instant::now();
    let all_sessions = load_sessions().expect("load sessions");
    let load_sessions_elapsed = phase_start.elapsed();

    invalidate_session_list_cache();
    let phase_start = std::time::Instant::now();
    let (groups, orphans) = load_sessions_grouped().expect("load grouped sessions");
    let grouped_elapsed = phase_start.elapsed();

    let snapshot_count = std::fs::read_dir(&sessions_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.file_name().to_str().is_some_and(|name| {
                        name.ends_with(".json") && !name.ends_with(".journal.json")
                    })
                })
                .count()
        })
        .unwrap_or_default();

    eprintln!(
        concat!(
            "real resume phases: scan_limit={} candidate_limit={} snapshot_count={} ",
            "candidate_count={} collect_candidates={}ms ",
            "jcode_summary={}ms jcode_loaded={} skipped_empty={} skipped_imported={} summary_errors={} ",
            "external_claude={}ms/{} external_codex={}ms/{} external_pi={}ms/{} external_opencode={}ms/{} ",
            "load_sessions={}ms/{} load_sessions_grouped={}ms groups={} orphans={}"
        ),
        scan_limit,
        candidate_limit,
        snapshot_count,
        candidates.len(),
        collect_candidates_elapsed.as_millis(),
        jcode_summary_elapsed.as_millis(),
        sessions.len(),
        skipped_empty,
        skipped_imported,
        summary_errors,
        claude_elapsed.as_millis(),
        claude.len(),
        codex_elapsed.as_millis(),
        codex.len(),
        pi_elapsed.as_millis(),
        pi.len(),
        opencode_elapsed.as_millis(),
        opencode.len(),
        load_sessions_elapsed.as_millis(),
        all_sessions.len(),
        grouped_elapsed.as_millis(),
        groups.len(),
        orphans.len(),
    );
}
#[test]
#[ignore = "developer benchmark: scans the real JCODE_HOME session directory"]
fn benchmark_real_resume_loading_reports_timings() {
    invalidate_session_list_cache();

    let load_start = std::time::Instant::now();
    let sessions = load_sessions().expect("load real sessions");
    let load_elapsed = load_start.elapsed();

    invalidate_session_list_cache();
    let grouped_start = std::time::Instant::now();
    let grouped = load_sessions_grouped().expect("load real grouped sessions");
    let grouped_elapsed = grouped_start.elapsed();
    let grouped_count = grouped
        .0
        .iter()
        .map(|group| group.sessions.len())
        .sum::<usize>()
        + grouped.1.len();

    eprintln!(
        "real resume bench: load_sessions={}ms count={} load_sessions_grouped={}ms grouped_count={} server_groups={} orphan_sessions={}",
        load_elapsed.as_millis(),
        sessions.len(),
        grouped_elapsed.as_millis(),
        grouped_count,
        grouped.0.len(),
        grouped.1.len()
    );
}
#[test]
fn benchmark_resume_loading_reports_timings() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp dir");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

    let sessions_dir = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    for idx in 0..120 {
        let mut session = Session::create_with_id(
            format!("session_resume_bench_{idx:03}"),
            Some(format!("/tmp/resume-bench-{idx:03}")),
            Some(format!("Resume Bench {idx:03}")),
        );
        session.append_stored_message(crate::session::StoredMessage {
            id: format!("msg-{idx}-1"),
            role: crate::message::Role::User,
            content: vec![crate::message::ContentBlock::Text {
                text: format!("session {idx:03} says benchmark transcript token zebra-{idx:03}"),
                cache_control: None,
            }],
            display_role: None,
            timestamp: None,
            tool_duration_ms: None,
            token_usage: None,
        });
        session.append_stored_message(crate::session::StoredMessage {
            id: format!("msg-{idx}-2"),
            role: crate::message::Role::Assistant,
            content: vec![crate::message::ContentBlock::Text {
                text: "assistant reply for benchmark coverage".to_string(),
                cache_control: None,
            }],
            display_role: None,
            timestamp: None,
            tool_duration_ms: None,
            token_usage: None,
        });
        session.save().expect("save benchmark session");
    }

    let load_start = std::time::Instant::now();
    let sessions = load_sessions().expect("load sessions");
    let load_elapsed = load_start.elapsed();

    let group_start = std::time::Instant::now();
    let grouped = load_sessions_grouped().expect("load grouped sessions");
    let group_elapsed = group_start.elapsed();

    assert!(sessions.len() >= 100);
    assert!(!grouped.0.is_empty() || !grouped.1.is_empty());

    eprintln!(
        "resume bench: load_sessions={}ms load_sessions_grouped={}ms count={}",
        load_elapsed.as_millis(),
        group_elapsed.as_millis(),
        sessions.len()
    );
}
