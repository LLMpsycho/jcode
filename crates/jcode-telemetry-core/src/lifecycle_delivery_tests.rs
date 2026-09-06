#[test]
fn superseded_session_queues_complete_lifecycle_without_blocking() {
    let _guard = lock_telemetry_test_state();
    *SESSION_STATE.lock().unwrap() = None;
    begin_session("test", "first");
    record_turn();
    let first_id = SESSION_STATE
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .session_id
        .clone();
    TEST_EMITTED_PAYLOADS.lock().unwrap().clear();
    TEST_DELIVERY_MODES.lock().unwrap().clear();

    begin_session("test", "second");

    let payloads = TEST_EMITTED_PAYLOADS.lock().unwrap().clone();
    let events: Vec<_> = payloads
        .iter()
        .map(|p| p["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["turn_end", "session_end", "todo_session"]);
    assert_eq!(payloads[0]["session_id"], first_id);
    assert_eq!(payloads[0]["turn_end_reason"], "superseded");
    assert_eq!(payloads[1]["session_id"], first_id);
    assert_eq!(payloads[1]["end_reason"], "superseded");
    let modes = TEST_DELIVERY_MODES.lock().unwrap();
    assert_eq!(modes.len(), payloads.len());
    assert!(modes.iter().all(|m| matches!(m, DeliveryMode::Background)));
    let state = SESSION_STATE.lock().unwrap();
    assert_eq!(state.as_ref().unwrap().model_start, "second");
    assert!(!state.as_ref().unwrap().start_event_sent);
}

#[test]
fn process_shutdown_keeps_bounded_blocking_lifecycle_delivery() {
    let _guard = lock_telemetry_test_state();
    *SESSION_STATE.lock().unwrap() = None;
    begin_session("test", "test");
    record_turn();
    TEST_DELIVERY_MODES.lock().unwrap().clear();
    end_session("test", "test");
    let modes = TEST_DELIVERY_MODES.lock().unwrap();
    assert_eq!(modes.len(), 3);
    assert!(modes.iter().all(
        |m| matches!(m, DeliveryMode::Blocking(timeout) if *timeout == BLOCKING_LIFECYCLE_TIMEOUT)
    ));
    assert!(SESSION_STATE.lock().unwrap().is_none());
}

