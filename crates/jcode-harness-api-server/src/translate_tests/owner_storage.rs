/// A session id becomes a filesystem path, so it must be treated as untrusted.
///
/// The id arrives straight off the wire and is interpolated into
/// `<home>/sessions/<id>.json`. Without validation, a traversal id is a
/// readable path, and `peek_session` returns whatever it finds there.
#[test]
fn a_session_id_cannot_escape_the_sessions_directory() {
    for hostile in [
        "../../../etc/passwd",
        "../.ssh/id_rsa",
        "a/b",
        "a\\b",
        "..",
        "",
        "with space",
        "semi;colon",
    ] {
        assert!(
            BridgeState::session_record_path(hostile).is_none(),
            "`{hostile}` must not resolve to a session record path"
        );
    }
}
#[test]
fn a_plain_session_id_still_resolves() {
    let path = BridgeState::session_record_path("session_otter_1785728596263_80eb5ad6012a1864")
        .expect("a normal session id must resolve");
    assert!(path.ends_with("session_otter_1785728596263_80eb5ad6012a1864.json"));
    assert!(
        path.parent().is_some_and(|dir| dir.ends_with("sessions")),
        "records live in the sessions directory: {}",
        path.display()
    );
}
/// Session records must be read from the *instance's* home, not the user's.
///
/// `launch()` gives an embedded instance its own `JCODE_HOME` precisely so it
/// cannot see the user's work. Reading the user's home directly made
/// `peek_session` return the real transcripts of the jcode the user runs
/// interactively, from a client that was supposed to be sandboxed.
#[test]
fn session_records_are_read_from_the_instance_home() {
    let home = ScopedJcodeHome::new("instance-home");
    let path = BridgeState::session_record_path("session_x_1_a");
    let path = path.expect("a normal session id must resolve");
    assert!(
        path.starts_with(&home.path),
        "JCODE_HOME must scope session records, got {}",
        path.display()
    );
}
#[test]
fn unattached_list_sessions_discovers_all_persisted_records() {
    let home = ScopedJcodeHome::new("persisted-discovery");
    let first_root = home.path.join("first-project");
    let second_root = home.path.join("second-project");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    write_session_record_with_titles(
        &home.path,
        "persisted_one",
        &first_root,
        Some("  Generated first title  "),
        None,
    );
    write_session_record_with_titles(
        &home.path,
        "persisted_two",
        &second_root,
        Some("Generated second title"),
        Some("  Custom second title  "),
    );
    std::fs::write(home.path.join("sessions/not-a-session.txt"), "ignored").unwrap();

    let event = only_reply_event(
        BridgeState::default().api_request_to_legacy(&json!({"req": "list_sessions", "id": 1})),
    );
    let ApiEvent::Sessions { sessions } = event else {
        panic!("expected sessions reply, got {event:?}");
    };
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        ["persisted_one", "persisted_two"]
    );
    assert_eq!(sessions[0].working_dir.as_deref(), first_root.to_str());
    assert_eq!(sessions[1].working_dir.as_deref(), second_root.to_str());
    assert_eq!(sessions[0].title.as_deref(), Some("Generated first title"));
    assert_eq!(sessions[1].title.as_deref(), Some("Custom second title"));
}
#[test]
fn limited_session_list_reads_compact_index_without_transcript_records() {
    let home = ScopedJcodeHome::new("metadata-index");
    assert!(BridgeState::recent_session_index_entries().is_empty());
    let mut connection = Connection::open(home.path.join("session-metadata-v1.sqlite3")).unwrap();
    let transaction = connection.transaction().unwrap();
    for index in 0..100 {
        transaction
            .execute(
                "INSERT INTO recent_sessions (
                     session_id, working_dir, todo_title, saved, updated_at_ms, last_active_at_ms
                 ) VALUES (?1, '/indexed/project', ?2, ?4, ?3, ?3)",
                params![
                    format!("indexed_{index:03}"),
                    format!("Indexed goal {index}"),
                    index,
                    index == 99,
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();

    let event = only_reply_event(
        BridgeState::default()
            .api_request_to_legacy(&json!({"req": "list_sessions", "id": 1, "limit": 100})),
    );
    let ApiEvent::Sessions { sessions } = event else {
        panic!("expected sessions reply, got {event:?}");
    };
    assert_eq!(sessions.len(), 100);
    let newest = sessions
        .iter()
        .find(|session| session.session_id == "indexed_099")
        .expect("indexed newest session");
    assert!(newest.saved);
    assert_eq!(newest.updated_at_ms, Some(99));
    assert_eq!(newest.last_active_at_ms, Some(99));
    assert!(sessions.iter().all(|session| {
        session
            .title
            .as_deref()
            .is_some_and(|title| title.starts_with("Indexed goal "))
    }));
}
#[test]
fn archive_restore_and_retention_are_reversible_and_owner_only() {
    let home = ScopedJcodeHome::new("archive");
    let root = home.path.join("project");
    std::fs::create_dir_all(&root).unwrap();
    write_session_record(&home.path, "recent_session", &root);
    let old_record = write_session_record(&home.path, "old_session", &root);
    let old_time = SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&old_record)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();

    let mut state = BridgeState::default();
    assert!(matches!(
        only_reply_event(state.api_request_to_legacy(&json!({
            "req": "archive_session",
            "id": 1,
            "session_id": "recent_session"
        }))),
        ApiEvent::Ok
    ));
    let ApiEvent::Sessions { sessions } =
        only_reply_event(state.api_request_to_legacy(&json!({"req": "list_sessions", "id": 2})))
    else {
        panic!("expected sessions");
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "old_session");

    assert!(matches!(
        only_reply_event(state.api_request_to_legacy(&json!({
            "req": "restore_session",
            "id": 3,
            "session_id": "recent_session"
        }))),
        ApiEvent::Ok
    ));
    assert!(matches!(
        only_reply_event(state.api_request_to_legacy(&json!({
            "req": "set_retention_policy",
            "id": 4,
            "archive_after_days": 1
        }))),
        ApiEvent::Ok
    ));

    let ApiEvent::Sessions { sessions } = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "list_sessions",
        "id": 5,
        "include_archived": true
    }))) else {
        panic!("expected sessions");
    };
    let old = sessions
        .iter()
        .find(|session| session.session_id == "old_session")
        .expect("old session remains restorable");
    assert_eq!(old.archived, true);
    assert!(old.archived_at_ms.is_some());
    let recent = sessions
        .iter()
        .find(|session| session.session_id == "recent_session")
        .expect("restored session is listed");
    assert!(!recent.archived);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.path.join("sdk-archive.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let home_mode = std::fs::metadata(&home.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(home_mode, 0o700);
    }
}
#[test]
fn credential_provisioning_normalizes_gemini_and_supports_jcode() {
    let home = ScopedJcodeHome::new("credentials");
    let config = home.path.join("config/jcode");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("gemini.env"),
        "GOOGLE_API_KEY=stale\nKEEP_ME=yes\n",
    )
    .unwrap();
    let mut state = BridgeState::default();

    let outbound = state.api_request_to_legacy(&json!({
        "req": "set_api_key",
        "id": 7,
        "provider": "google-gemini",
        "api_key": "gemini-secret"
    }));
    let [Outbound::Legacy(notify)] = outbound.as_slice() else {
        panic!("credential change should notify the daemon: {outbound:?}");
    };
    assert_eq!(notify["provider"], "gemini");
    let legacy_id = notify["id"].as_u64().unwrap();
    let frames = state.legacy_event_to_api(&json!({"type": "ack", "id": legacy_id}));
    assert!(matches!(
        &frames[0].event,
        ApiEvent::CredentialUpdated { provider, configured }
            if provider == "gemini" && *configured
    ));
    let gemini = std::fs::read_to_string(config.join("gemini.env")).unwrap();
    assert!(gemini.contains("GEMINI_API_KEY=gemini-secret\n"));
    assert!(gemini.contains("KEEP_ME=yes\n"));
    assert!(!gemini.contains("GOOGLE_API_KEY"));

    let outbound = state.api_request_to_legacy(&json!({
        "req": "set_api_key",
        "id": 8,
        "provider": "subscription",
        "api_key": "jcode-secret"
    }));
    let [Outbound::Legacy(notify)] = outbound.as_slice() else {
        panic!("jcode credential should notify the daemon: {outbound:?}");
    };
    assert_eq!(notify["provider"], "jcode");
    assert_eq!(
        std::fs::read_to_string(config.join("jcode-subscription.env")).unwrap(),
        "JCODE_API_KEY=jcode-secret\n"
    );

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "set_api_key",
        "id": 9,
        "provider": "gemini",
        "api_key": "line one\nline two"
    })));
    assert!(matches!(
        event,
        ApiEvent::Error {
            code: ErrorCode::InvalidRequest,
            ..
        }
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(config.join("gemini.env"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[cfg(unix)]
#[test]
fn owner_only_writes_refuse_symlink_targets_and_directories() {
    use std::os::unix::fs::symlink;

    let home = ScopedJcodeHome::new("credential-symlinks");
    let outside_file = home.path.join("outside.env");
    std::fs::write(&outside_file, "unchanged\n").unwrap();
    let config = home.path.join("config/jcode");
    std::fs::create_dir_all(&config).unwrap();
    symlink(&outside_file, config.join("gemini.env")).unwrap();
    let mut state = BridgeState::default();
    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "set_api_key",
        "id": 1,
        "provider": "gemini",
        "api_key": "must-not-land"
    })));
    assert!(matches!(
        event,
        ApiEvent::Error {
            code: ErrorCode::Internal,
            ..
        }
    ));
    assert_eq!(
        std::fs::read_to_string(&outside_file).unwrap(),
        "unchanged\n"
    );

    std::fs::remove_file(config.join("gemini.env")).unwrap();
    std::fs::remove_dir(&config).unwrap();
    let outside_dir = home.path.join("outside-config");
    std::fs::create_dir_all(&outside_dir).unwrap();
    symlink(&outside_dir, &config).unwrap();
    let event = only_reply_event(BridgeState::default().api_request_to_legacy(&json!({
        "req": "set_api_key",
        "id": 2,
        "provider": "jcode",
        "api_key": "must-not-land"
    })));
    assert!(matches!(
        event,
        ApiEvent::Error {
            code: ErrorCode::Internal,
            ..
        }
    ));
    assert!(!outside_dir.join("jcode-subscription.env").exists());
}
#[cfg(unix)]
#[test]
fn rooted_file_operations_reject_traversal_and_symlink_escapes_and_bound_results() {
    use std::os::unix::fs::symlink;

    let home = ScopedJcodeHome::new("rooted-files");
    let root = home.path.join("project");
    let outside = home.path.join("outside");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("src/unicode.txt"), "éx secret\n").unwrap();
    for index in 0..8 {
        std::fs::write(root.join(format!("src/match-{index}.txt")), "needle\n").unwrap();
    }
    std::fs::write(outside.join("outside-secret.txt"), "outside needle\n").unwrap();
    symlink(&outside, root.join("escape")).unwrap();
    write_session_record(&home.path, "s1", &root);
    let mut state = state_with_session();

    for hostile in ["../outside/outside-secret.txt", "escape/outside-secret.txt"] {
        let event = only_reply_event(state.api_request_to_legacy(&json!({
            "req": "read_file",
            "id": 1,
            "session_id": "s1",
            "path": hostile
        })));
        assert!(matches!(
            event,
            ApiEvent::Error {
                code: ErrorCode::InvalidRequest,
                ..
            }
        ));
    }

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "read_file",
        "id": 2,
        "session_id": "s1",
        "path": "src/unicode.txt",
        "max_bytes": 2
    })));
    assert!(matches!(
        event,
        ApiEvent::FileContent {
            content,
            truncated: true,
            ..
        } if content == "é"
    ));

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "find_files",
        "id": 3,
        "session_id": "s1",
        "query": "outside-secret",
        "limit": 1000000
    })));
    assert!(matches!(event, ApiEvent::Files { paths, .. } if paths.is_empty()));

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "search_text",
        "id": 4,
        "session_id": "s1",
        "query": "needle",
        "limit": 3
    })));
    let ApiEvent::TextMatches { matches, .. } = event else {
        panic!("expected bounded text matches, got {event:?}");
    };
    assert_eq!(matches.len(), 3);
    assert!(
        matches
            .iter()
            .all(|found| !found.path.starts_with("escape/"))
    );

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "file_status",
        "id": 5,
        "session_id": "s1",
        "path": "src/missing.txt"
    })));
    assert!(matches!(
        event,
        ApiEvent::FileStatus {
            exists: false,
            ref kind,
            ..
        } if kind == "missing"
    ));
}

#[test]
fn empty_session_directory_lists_without_panicking() {
    let home = ScopedJcodeHome::new("empty-session-directory");
    std::fs::create_dir_all(home.path.join("sessions")).unwrap();
    let mut state = BridgeState::default();
    let actions = state.api_request_to_legacy(&json!({"id": 1, "req": "list_sessions"}));
    let ApiEvent::Sessions { sessions } = only_reply_event(actions) else {
        panic!("expected sessions reply");
    };
    assert!(sessions.is_empty());
}
