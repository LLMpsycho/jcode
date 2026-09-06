/// Attaching must volunteer the model identity: a client that has to know to
/// ask would show "unknown model" forever, which is what this fixes.
#[test]
fn attaching_probes_and_reports_the_model() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "create_session", "id": 7}));
    let Outbound::Legacy(catalog) = &out[2] else {
        panic!("expected a legacy catalog probe");
    };
    assert_eq!(catalog["type"], "get_model_catalog");
    let catalog_id = catalog["id"].as_u64().unwrap();

    // The daemon answers the probe with a `history`-shaped reply carrying no
    // messages. That must become an unsolicited model_info event, not a reply
    // to some client request that never asked for history.
    let frames = state.legacy_event_to_api(&json!({
        "type": "history", "id": catalog_id, "messages": [],
        "provider_name": "anthropic", "provider_model": "claude-sonnet-4-5",
    }));
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].reply_to, None,
        "the probe was not client-initiated"
    );
    match &frames[0].event {
        ApiEvent::ModelInfo {
            provider, model, ..
        } => {
            assert_eq!(provider.as_deref(), Some("anthropic"));
            assert_eq!(model.as_deref(), Some("claude-sonnet-4-5"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}
/// A real `get_history` reply must still be a history reply after the probe has
/// been consumed, or the probe would swallow the client's own request.
#[test]
fn a_client_history_request_is_untouched_by_the_probe() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "create_session", "id": 1}));
    let Outbound::Legacy(catalog) = &out[2] else {
        panic!("expected a catalog probe");
    };
    let catalog_id = catalog["id"].as_u64().unwrap();
    state.legacy_event_to_api(&json!({"type": "history", "id": catalog_id, "messages": []}));

    let out = state.api_request_to_legacy(&json!({"req": "get_history", "id": 9}));
    let Outbound::Legacy(request) = &out[0] else {
        panic!("expected a legacy history request");
    };
    let history_id = request["id"].as_u64().unwrap();
    let frames = state.legacy_event_to_api(&json!({
        "type": "history", "id": history_id,
        "messages": [{"role": "user", "content": "hi"}],
    }));
    assert_eq!(frames[0].reply_to, Some(9));
    assert!(matches!(frames[0].event, ApiEvent::History { .. }));
}
/// Switching model mid-session must reach the client, or the caption goes stale
/// and confidently lies about which model answered.
#[test]
fn a_model_change_is_forwarded() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "model_changed", "id": 3,
        "model": "gpt-5.6", "provider_name": "openai",
    }));
    match &frames[0].event {
        ApiEvent::ModelInfo {
            provider, model, ..
        } => {
            assert_eq!(provider.as_deref(), Some("openai"));
            assert_eq!(model.as_deref(), Some("gpt-5.6"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}
/// A failed model change must not be reported as the active model.
#[test]
fn a_failed_model_change_is_not_reported() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "model_changed", "id": 3, "model": "nope", "error": "no such model",
    }));
    assert!(frames.is_empty());
}
/// An auth change re-resolves the route, so the push must update the caption.
#[test]
fn an_available_models_push_updates_the_model() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "available_models_updated",
        "provider_name": "anthropic", "provider_model": "claude-opus-4-5",
        "available_models": ["claude-opus-4-5"],
    }));
    match &frames[0].event {
        ApiEvent::ModelInfo {
            session_id, model, ..
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(model.as_deref(), Some("claude-opus-4-5"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}
#[test]
fn create_session_in_a_jcode_checkout_requests_selfdev() {
    // Regression: external client opens its own crate, and without the `selfdev`
    // flag the daemon hands back an agent with no self-dev tools or prompt.
    let mut state = BridgeState::default();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("crates/jcode-tui");
    let out = state.api_request_to_legacy(&json!({
        "req": "create_session",
        "id": 1,
        "working_dir": repo.display().to_string(),
    }));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(value["selfdev"], json!(true));
}
#[test]
fn create_session_outside_a_checkout_leaves_selfdev_unset() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({
        "req": "create_session",
        "id": 1,
        "working_dir": "/",
    }));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert!(value.get("selfdev").is_none(), "got {value}");
}
/// A turn that fails ends with `error` instead of `done`. The bridge must let
/// go of the pending message, or a later unrelated `done` reusing that legacy
/// id would be reported to the client as this turn finally finishing, and a
/// client that trusts `turn_done` would unblock on a turn that never ran.
#[test]
fn a_failed_turn_clears_the_pending_message() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "req": "send_message", "id": 11, "content": "hi",
    }));
    let Outbound::Legacy(message) = &out[0] else {
        panic!("expected a legacy message");
    };
    let legacy_id = message["id"].as_u64().expect("a legacy id");

    let frames = state.legacy_event_to_api(&json!({
        "type": "error", "id": legacy_id, "message": "dns error",
    }));
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame.event, ApiEvent::Error { .. })),
        "the failure was not forwarded"
    );

    // The same id arriving as `done` afterwards is no longer this turn.
    let frames = state.legacy_event_to_api(&json!({"type": "done", "id": legacy_id}));
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame.event, ApiEvent::TurnDone { .. })),
        "a failed turn reported a second, phantom completion"
    );
}
/// The catalog arrives on attach, so a picker must open without a round trip.
#[test]
fn list_models_is_answered_from_the_cached_catalog() {
    let mut state = state_with_session();
    state.legacy_event_to_api(&json!({
        "type": "available_models_updated",
        "provider_model": "claude-opus-5",
        "available_models": ["claude-opus-5", "claude-fable-5"],
    }));

    let out = state.api_request_to_legacy(&json!({"id": 9, "req": "list_models"}));
    match &out[..] {
        [Outbound::Reply(frame)] => match &frame.event {
            ApiEvent::Models {
                models, current, ..
            } => {
                assert_eq!(models, &["claude-opus-5", "claude-fable-5"]);
                assert_eq!(current.as_deref(), Some("claude-opus-5"));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("expected one local reply, got {other:?}"),
    }
}
/// A client can ask before the catalog lands. Answering "no models" then would
/// be a lie that empties its picker, so the request waits for the real answer.
#[test]
fn list_models_before_the_catalog_asks_the_daemon() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"id": 9, "req": "list_models"}));
    match &out[..] {
        [Outbound::Legacy(value)] => assert_eq!(value["type"], "get_model_catalog"),
        other => panic!("expected a daemon round trip, got {other:?}"),
    }

    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => value["id"].as_u64().unwrap(),
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "history", "id": legacy_id, "session_id": "s1",
        "available_models": ["a", "b"], "provider_model": "a",
    }));
    match &frames[0].event {
        ApiEvent::Models { models, .. } => assert_eq!(models, &["a", "b"]),
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(frames[0].reply_to, Some(9));
}
/// A switch must resolve the caller's request *and* tell every other client
/// watching the session that the model moved under them.
#[test]
fn a_requested_model_change_replies_and_broadcasts() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "id": 4, "req": "set_model", "model": "claude-fable-5",
    }));
    let legacy_id = match &out[..] {
        [Outbound::Legacy(value)] => {
            assert_eq!(value["type"], "set_model");
            assert_eq!(value["model"], "claude-fable-5");
            value["id"].as_u64().unwrap()
        }
        other => panic!("expected a daemon request, got {other:?}"),
    };

    let frames = state.legacy_event_to_api(&json!({
        "type": "model_changed", "id": legacy_id,
        "model": "claude-fable-5", "provider_name": "anthropic",
    }));
    assert_eq!(frames.len(), 2, "expected a reply and a broadcast");
    assert_eq!(frames[0].reply_to, Some(4));
    assert!(matches!(frames[0].event, ApiEvent::Ok));
    assert_eq!(frames[1].reply_to, None);
    assert!(matches!(frames[1].event, ApiEvent::ModelInfo { .. }));
    // The cache must follow, or a picker reopened after the switch is wrong.
    assert_eq!(state.current_model.as_deref(), Some("claude-fable-5"));
}
/// The daemon reports a rejected switch in-band, on a success-shaped event.
/// Reporting success there would leave the client's picker showing a model
/// the session is not using.
#[test]
fn a_rejected_model_change_fails_the_request() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "id": 4, "req": "set_model", "model": "nope",
    }));
    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => value["id"].as_u64().unwrap(),
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "model_changed", "id": legacy_id,
        "model": "nope", "error": "unknown model",
    }));
    match &frames[..] {
        [frame] => {
            assert_eq!(frame.reply_to, Some(4));
            match &frame.event {
                ApiEvent::Error { code, message } => {
                    assert_eq!(*code, ErrorCode::InvalidRequest);
                    assert_eq!(message, "unknown model");
                }
                other => panic!("unexpected: {other:?}"),
            }
        }
        other => panic!("expected one error reply, got {other:?}"),
    }
    assert_eq!(
        state.current_model, None,
        "a failed switch must not be cached"
    );
}
#[test]
fn an_empty_model_is_refused_locally() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"id": 4, "req": "set_model", "model": ""}));
    match &out[..] {
        [Outbound::Reply(frame)] => {
            assert!(matches!(frame.event, ApiEvent::Error { .. }));
        }
        other => panic!("expected a local rejection, got {other:?}"),
    }
}
#[test]
fn reasoning_effort_reports_provider_refusal() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "id": 5, "req": "set_reasoning_effort", "effort": "max",
    }));
    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => {
            assert_eq!(value["effort"], "max");
            value["id"].as_u64().unwrap()
        }
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "reasoning_effort_changed", "id": legacy_id,
        "error": "provider does not support reasoning effort",
    }));
    assert_eq!(frames[0].reply_to, Some(5));
    assert!(matches!(frames[0].event, ApiEvent::Error { .. }));
}
/// An effort change is identity, like a model change: every attached client
/// needs to hear it, not only the requester. A change made by another client
/// (no pending request here) must still arrive as a `model_info` broadcast,
/// and the requester's own change gets the broadcast after its `Ok`.
#[test]
fn reasoning_effort_changes_are_broadcast_as_model_info() {
    let mut state = state_with_session();

    // Unsolicited change (another client's request id): broadcast only.
    let frames = state.legacy_event_to_api(&json!({
        "type": "reasoning_effort_changed", "id": 999, "effort": "high",
    }));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].reply_to, None);
    match &frames[0].event {
        ApiEvent::ModelInfo {
            reasoning_effort, ..
        } => assert_eq!(reasoning_effort.as_deref(), Some("high")),
        other => panic!("expected model_info, got {other:?}"),
    }

    // The same effort again is not news: no broadcast.
    let frames = state.legacy_event_to_api(&json!({
        "type": "reasoning_effort_changed", "id": 999, "effort": "high",
    }));
    assert!(frames.is_empty(), "unchanged effort must not re-broadcast");

    // This client's own change: Ok reply first, then the broadcast.
    let out = state.api_request_to_legacy(&json!({
        "id": 7, "req": "set_reasoning_effort", "effort": "low",
    }));
    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => value["id"].as_u64().unwrap(),
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "reasoning_effort_changed", "id": legacy_id, "effort": "low",
    }));
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].reply_to, Some(7));
    assert!(matches!(frames[0].event, ApiEvent::Ok));
    assert!(matches!(
        &frames[1].event,
        ApiEvent::ModelInfo { reasoning_effort, .. }
            if reasoning_effort.as_deref() == Some("low")
    ));
}
/// Compaction can be refused (nothing to compact, a turn in flight) and the
/// daemon says so with `success: false`, not an error frame. Telling the
/// client "done" would claim work that never happened.
#[test]
fn a_refused_compaction_is_an_error_not_a_success() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"id": 6, "req": "compact"}));
    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => value["id"].as_u64().unwrap(),
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "compact_result", "id": legacy_id,
        "message": "nothing to compact", "success": false,
    }));
    match &frames[0].event {
        ApiEvent::Error { message, .. } => assert_eq!(message, "nothing to compact"),
        other => panic!("unexpected: {other:?}"),
    }
}
#[test]
fn a_scheduled_compaction_reports_its_status() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"id": 6, "req": "compact"}));
    let legacy_id = match &out[0] {
        Outbound::Legacy(value) => value["id"].as_u64().unwrap(),
        _ => unreachable!(),
    };
    let frames = state.legacy_event_to_api(&json!({
        "type": "compact_result", "id": legacy_id,
        "message": "compacting in the background", "success": true,
    }));
    match &frames[0].event {
        ApiEvent::Compacted { message, .. } => assert_eq!(message, "compacting in the background"),
        other => panic!("unexpected: {other:?}"),
    }
}
/// Clearing a title is distinct from setting an empty one, so an absent title
/// must not be sent as `""`, which the daemon would store as a real title.
#[test]
fn renaming_distinguishes_clearing_from_setting() {
    let mut state = state_with_session();
    let set = state.api_request_to_legacy(&json!({
        "id": 7, "req": "rename_session", "title": "my session",
    }));
    match &set[0] {
        Outbound::Legacy(value) => assert_eq!(value["title"], "my session"),
        other => panic!("unexpected: {other:?}"),
    }

    let clear = state.api_request_to_legacy(&json!({"id": 8, "req": "rename_session"}));
    match &clear[0] {
        Outbound::Legacy(value) => assert!(
            value.get("title").is_none(),
            "a cleared title must be absent, not empty: {value}"
        ),
        other => panic!("unexpected: {other:?}"),
    }
}
