#[test]
fn concurrency_guard_emits_versioned_runtime_events_without_counting_legacy_tui_activity() {
    let _guard = lock_telemetry_test_state();
    TEST_EMITTED_PAYLOADS.lock().unwrap().clear();
    *SESSION_STATE.lock().unwrap() = None;
    let mut root = begin_concurrency_session("root-session", None);
    assert!(root.is_active());
    assert_eq!(root.session_id(), "root-session");
    // TUI/singleton begin calls must not add runtime ownership or inflate peaks.
    begin_session("test", "test");
    begin_session("test", "test");
    record_turn();
    let child = begin_concurrency_session("child-session", Some("root-session"));
    drop(child);
    root.finish();
    root.finish();
    assert!(!root.is_active());
    end_session("test", "test");
    drop(root);

    let payloads = TEST_EMITTED_PAYLOADS.lock().unwrap().clone();
    let events: Vec<_> = payloads
        .iter()
        .filter(|p| p["event"] == "session_concurrency")
        .collect();
    assert_eq!(
        events.len(),
        4,
        "explicit finish followed by Drop emits only one end"
    );
    for event in &events {
        assert_eq!(event["concurrency_tracking_version"], 2);
        assert_eq!(
            event["concurrency_tracking_scope"],
            "runtime_agent_sessions"
        );
        assert_eq!(event["concurrency_tracking_available"], true);
        assert_eq!(event["id"], events[0]["id"]);
        assert!(uuid::Uuid::parse_str(event["concurrency_session_id"].as_str().unwrap()).is_ok());
        let active = event["active_sessions_at_start"].as_u64().unwrap();
        let peak = event["max_concurrent_sessions"].as_u64().unwrap();
        assert!(peak >= active && active >= 1);
        assert_eq!(event["other_active_sessions_at_start"], active - 1);
        assert_eq!(event["multi_sessioned"], peak > 1);
        assert_eq!(
            event["root_sessions_at_start"].as_u64().unwrap()
                + event["child_sessions_at_start"].as_u64().unwrap(),
            active
        );
    }
    assert_eq!(events[0]["phase"], "start");
    assert_eq!(events[0]["active_sessions_at_start"], 1);
    assert_eq!(events[1]["agent_role"], "child");
    assert_eq!(events[1]["active_sessions_at_start"], 2);
    assert_eq!(events[3]["phase"], "end");
    assert_eq!(events[3]["max_concurrent_sessions"], 2);
    assert_eq!(events[3]["max_concurrent_root_sessions"], 1);
    assert_eq!(events[3]["max_concurrent_child_sessions"], 1);
    assert_eq!(
        events[0]["concurrency_session_id"],
        events[3]["concurrency_session_id"]
    );
    assert_ne!(
        events[0]["concurrency_session_id"],
        events[1]["concurrency_session_id"]
    );
    let legacy: Vec<_> = payloads
        .iter()
        .filter(|p| matches!(p["event"].as_str(), Some("session_start" | "session_end")))
        .collect();
    assert_eq!(legacy.len(), 2);
    for event in legacy {
        assert_eq!(event["concurrency_tracking_version"], 2);
        assert_eq!(event["concurrency_tracking_scope"], "legacy_process_global");
        assert_eq!(event["concurrency_tracking_available"], false);
        for key in [
            "active_sessions_at_start",
            "other_active_sessions_at_start",
            "max_concurrent_sessions",
            "multi_sessioned",
        ] {
            assert!(event.get(key).is_none());
        }
    }
    assert!(
        !storage::jcode_dir()
            .unwrap()
            .join("telemetry_active_sessions")
            .exists()
    );
}

#[test]
fn concurrency_guard_opt_out_is_inert_and_opt_out_after_start_still_releases_ownership() {
    let _guard = lock_telemetry_test_state();
    TEST_EMITTED_PAYLOADS.lock().unwrap().clear();
    jcode_core::env::set_var("JCODE_NO_TELEMETRY", "1");
    let mut disabled = begin_concurrency_session("disabled-session", None);
    assert!(!disabled.is_active());
    assert_eq!(disabled.session_id(), "disabled-session");
    disabled.finish();
    assert!(
        !storage::jcode_dir()
            .unwrap()
            .join("telemetry_concurrency_v2")
            .exists()
    );
    assert!(TEST_EMITTED_PAYLOADS.lock().unwrap().is_empty());
    jcode_core::env::remove_var("JCODE_NO_TELEMETRY");
    let mut active = begin_concurrency_session("active", None);
    jcode_core::env::set_var("DO_NOT_TRACK", "1");
    active.finish();
    jcode_core::env::remove_var("DO_NOT_TRACK");
    let mut fresh = begin_concurrency_session("fresh", None);
    fresh.finish();
    let payloads = TEST_EMITTED_PAYLOADS.lock().unwrap();
    assert_eq!(payloads.len(), 3, "opted-out end must not emit");
    assert_eq!(payloads[1]["active_sessions_at_start"], 1);
    assert_eq!(payloads[2]["max_concurrent_sessions"], 1);
}

#[test]
fn concurrency_guard_paths_never_use_untrusted_session_ids() {
    let _guard = lock_telemetry_test_state();
    let home = storage::jcode_dir().unwrap();
    let escape = home.join("escaped");
    let mut first = begin_concurrency_session("../../escaped", None);
    let mut second = begin_concurrency_session(escape.to_str().unwrap(), Some(".."));
    assert!(!escape.exists());
    for entry in std::fs::read_dir(home.join("telemetry_concurrency_v2")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) == Some("lease") {
            assert!(uuid::Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).is_ok());
        }
    }
    first.finish();
    second.finish();
}

#[test]
fn concurrency_guard_io_failure_omits_numbers_instead_of_fabricating_zero() {
    let _guard = lock_telemetry_test_state();
    TEST_EMITTED_PAYLOADS.lock().unwrap().clear();
    std::fs::write(
        storage::jcode_dir()
            .unwrap()
            .join("telemetry_concurrency_v2"),
        "not a directory",
    )
    .unwrap();
    let mut session = begin_concurrency_session("unavailable", None);
    assert!(session.is_active());
    session.finish();
    let payloads = TEST_EMITTED_PAYLOADS.lock().unwrap();
    assert_eq!(payloads.len(), 2);
    for event in payloads.iter() {
        assert_eq!(event["concurrency_tracking_version"], 2);
        assert_eq!(event["concurrency_tracking_available"], false);
        for key in [
            "active_sessions_at_start",
            "other_active_sessions_at_start",
            "max_concurrent_sessions",
            "multi_sessioned",
            "root_sessions_at_start",
            "child_sessions_at_start",
            "max_concurrent_root_sessions",
            "max_concurrent_child_sessions",
        ] {
            assert!(event.get(key).is_none());
        }
    }
}

#[test]
fn concurrent_first_sessions_share_one_persisted_install_id() {
    let _guard = lock_telemetry_test_state();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                get_or_create_id().unwrap()
            })
        })
        .collect();
    let ids: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(read_existing_id().as_ref(), Some(&ids[0]));
}

#[test]
fn concurrency_crash_lifecycle_does_not_emit_untrusted_legacy_counts() {
    let _guard = lock_telemetry_test_state();
    TEST_EMITTED_PAYLOADS.lock().unwrap().clear();
    *SESSION_STATE.lock().unwrap() = None;
    begin_session("test", "test");
    record_turn();
    record_crash("test", "test", SessionEndReason::Panic);
    let payloads = TEST_EMITTED_PAYLOADS.lock().unwrap();
    let crash = payloads
        .iter()
        .find(|p| p["event"] == "session_crash")
        .unwrap();
    assert_eq!(crash["concurrency_tracking_version"], 2);
    assert_eq!(crash["concurrency_tracking_scope"], "legacy_process_global");
    assert_eq!(crash["concurrency_tracking_available"], false);
    assert!(crash.get("max_concurrent_sessions").is_none());
    assert!(crash.get("active_sessions_at_start").is_none());
    assert!(crash.get("multi_sessioned").is_none());
}
