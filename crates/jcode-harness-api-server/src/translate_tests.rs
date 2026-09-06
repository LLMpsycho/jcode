use super::*;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn jcode_home_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ScopedJcodeHome {
    path: PathBuf,
    previous: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl ScopedJcodeHome {
    fn new(label: &str) -> Self {
        let guard = jcode_home_test_lock();
        let previous = std::env::var_os("JCODE_HOME");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "jcode-harness-api-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create isolated JCODE_HOME");
        // SAFETY: all tests in this module that mutate JCODE_HOME share `LOCK`,
        // and this guard restores the prior value before it is released.
        unsafe { std::env::set_var("JCODE_HOME", &path) };
        Self {
            path,
            previous,
            _guard: guard,
        }
    }
}

impl Drop for ScopedJcodeHome {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("JCODE_HOME", value) },
            None => unsafe { std::env::remove_var("JCODE_HOME") },
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_session_record(home: &Path, session_id: &str, working_dir: &Path) -> PathBuf {
    write_session_record_with_titles(home, session_id, working_dir, None, None)
}

fn write_session_record_with_titles(
    home: &Path,
    session_id: &str,
    working_dir: &Path,
    title: Option<&str>,
    custom_title: Option<&str>,
) -> PathBuf {
    let sessions = home.join("sessions");
    std::fs::create_dir_all(&sessions).expect("create sessions directory");
    let path = sessions.join(format!("{session_id}.json"));
    std::fs::write(
        &path,
        json!({
            "working_dir": working_dir,
            "title": title,
            "custom_title": custom_title,
            "messages": [{"role": "user", "content": "hello"}],
        })
        .to_string(),
    )
    .expect("write session record");
    path
}

#[test]
fn persisted_metadata_reads_large_transcripts_from_bounded_windows() {
    let home = ScopedJcodeHome::new("bounded-metadata");
    let sessions = home.path.join("sessions");
    std::fs::create_dir_all(&sessions).expect("create sessions directory");
    let path = sessions.join("session_large.json");
    let mut file = std::fs::File::create(&path).expect("create large session");
    write!(
        file,
        "{{\"id\":\"session_large\",\"title\":\"Generated title\",\"messages\":[\""
    )
    .unwrap();
    for _ in 0..(2 * 1024) {
        file.write_all(&[b'x'; 1024]).unwrap();
    }
    write!(
        file,
        "\"],\"working_dir\":\"/workspace/large\",\"custom_title\":\"Pinned title\"}}"
    )
    .unwrap();
    drop(file);

    let metadata = BridgeState::resolve_session_metadata("session_large").expect("metadata");
    assert_eq!(metadata.working_dir.as_deref(), Some("/workspace/large"));
    assert_eq!(metadata.title.as_deref(), Some("Generated title"));
    assert_eq!(metadata.custom_title.as_deref(), Some("Pinned title"));
    assert_eq!(metadata.display_title().as_deref(), Some("Pinned title"));
}

fn only_reply_event(outbound: Vec<Outbound>) -> ApiEvent {
    assert_eq!(outbound.len(), 1, "expected exactly one reply");
    match outbound.into_iter().next().expect("one outbound") {
        Outbound::Reply(frame) => frame.event,
        other => panic!("expected API reply, got {other:?}"),
    }
}

fn state_with_session() -> BridgeState {
    BridgeState {
        session_id: Some("s1".into()),
        ..Default::default()
    }
}

#[test]
fn connection_phase_is_forwarded_to_api_clients() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "connection_phase",
        "phase": "sending request",
    }));

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].reply_to, None);
    assert_eq!(
        frames[0].event,
        ApiEvent::ConnectionPhase {
            session_id: "s1".into(),
            phase: "sending request".into(),
        }
    );
}

#[test]
fn wake_request_is_forwarded_with_explicit_session_and_payload() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "wake_requested",
        "session_id": "target",
        "reason": "background_task_completed",
        "notification": "finished",
    }));
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].event,
        ApiEvent::WakeRequested {
            session_id: "target".into(),
            reason: "background_task_completed".into(),
            notification: "finished".into(),
        }
    );
}

#[test]
fn create_session_maps_to_subscribe() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "create_session", "id": 1}));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(value["type"], "subscribe");
    assert!(value["working_dir"].is_string());
}

#[test]
fn desktop_owned_session_requests_crash_on_disconnect() {
    let mut state = BridgeState::with_crash_on_disconnect(true);
    let out = state.api_request_to_legacy(&json!({"req": "create_session", "id": 1}));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(value["crash_on_disconnect"], true);
}

#[test]
fn detach_disarms_crash_on_disconnect() {
    let mut state = BridgeState::with_crash_on_disconnect(true);
    let out = state.api_request_to_legacy(&json!({
        "req": "detach_session",
        "id": 2,
        "session_id": "abc",
    }));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(value["type"], "prepare_disconnect");
}

#[test]
fn state_event_answers_pending_attach() {
    let home = ScopedJcodeHome::new("attach-title");
    let project = home.path.join("project");
    std::fs::create_dir_all(&project).unwrap();
    write_session_record_with_titles(
        &home.path,
        "abc",
        &project,
        Some("Generated attach title"),
        Some("Persisted attach rename"),
    );
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "create_session", "id": 5}));
    assert_eq!(
        out.len(),
        3,
        "subscribe + state chase + model catalog probe"
    );
    let Outbound::Legacy(state_req) = &out[1] else {
        panic!("expected legacy state request");
    };
    assert_eq!(state_req["type"], "state");
    let state_id = state_req["id"].as_u64().unwrap();

    // A subscribe `done` must not leak a turn_done.
    let done = state.legacy_event_to_api(&json!({"type": "done", "id": 1}));
    assert!(done.is_empty());

    let frames = state.legacy_event_to_api(&json!({
        "type": "state", "id": state_id, "session_id": "abc",
        "message_count": 0, "is_processing": false,
    }));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].reply_to, Some(5));
    match &frames[0].event {
        ApiEvent::Attached { session } => {
            assert_eq!(session.session_id, "abc");
            assert_eq!(session.title.as_deref(), Some("Persisted attach rename"));
            assert_eq!(session.working_dir.as_deref(), project.to_str());
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(state.session_id.as_deref(), Some("abc"));
}

#[test]
fn send_message_then_done_becomes_turn_done() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(
        &json!({"req": "send_message", "id": 2, "session_id": "s1", "content": "hi"}),
    );
    let Outbound::Legacy(message) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(message["type"], "message");
    let legacy_id = message["id"].as_u64().unwrap();

    let deltas = state.legacy_event_to_api(&json!({"type": "text_delta", "text": "yo"}));
    assert!(matches!(
        &deltas[0].event,
        ApiEvent::TextDelta { session_id, text } if session_id == "s1" && text == "yo"
    ));

    let done = state.legacy_event_to_api(&json!({"type": "done", "id": legacy_id}));
    assert!(matches!(
        &done[0].event,
        ApiEvent::TurnDone { session_id } if session_id == "s1"
    ));
}

/// The daemon acking the in-flight message is the only signal that the agent
/// took delivery, so it must surface as its own event rather than being
/// swallowed as a bookkeeping ack. A client that shows "sent" until the first
/// token of the reply is showing a lie for as long as the model thinks.
#[test]
fn acking_the_pending_message_reports_acceptance() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(
        &json!({"req": "send_message", "id": 2, "session_id": "s1", "content": "hi"}),
    );
    let Outbound::Legacy(message) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = message["id"].as_u64().unwrap();

    let accepted = state.legacy_event_to_api(&json!({"type": "ack", "id": legacy_id}));
    assert!(matches!(
        &accepted[0].event,
        ApiEvent::MessageAccepted { session_id } if session_id == "s1"
    ));
    // The turn must still end normally: the acceptance event must not consume
    // the pending id the `done` boundary depends on.
    let done = state.legacy_event_to_api(&json!({"type": "done", "id": legacy_id}));
    assert!(matches!(&done[0].event, ApiEvent::TurnDone { .. }));
}

#[test]
fn context_only_message_waits_for_persistence_event_and_replies_ok() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "req": "send_message", "id": 27, "session_id": "s1",
        "content": "context", "no_reply": true
    }));
    let Outbound::Legacy(message) = &out[0] else {
        panic!("expected legacy outbound");
    };
    assert_eq!(message["type"], "message");
    assert_eq!(message["no_reply"], true);
    let legacy_id = message["id"].as_u64().unwrap();

    assert!(
        state
            .legacy_event_to_api(&json!({"type": "ack", "id": legacy_id}))
            .is_empty(),
        "the daemon's early ack does not prove persistence"
    );
    let frames =
        state.legacy_event_to_api(&json!({"type": "context_message_added", "id": legacy_id}));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].reply_to, Some(27));
    assert!(matches!(frames[0].event, ApiEvent::Ok));
    assert!(
        state
            .legacy_event_to_api(&json!({"type": "done", "id": legacy_id}))
            .is_empty(),
        "context-only messages never create turn boundaries"
    );
}

#[test]
fn context_only_message_error_is_correlated_to_the_request() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "req": "send_message", "id": 28, "session_id": "s1",
        "content": "context", "no_reply": true
    }));
    let Outbound::Legacy(message) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = message["id"].as_u64().unwrap();
    let frames = state
        .legacy_event_to_api(&json!({"type": "error", "id": legacy_id, "message": "save failed"}));
    assert_eq!(frames[0].reply_to, Some(28));
    assert!(matches!(
        &frames[0].event,
        ApiEvent::Error { message, .. } if message == "save failed"
    ));
    assert!(
        state
            .legacy_event_to_api(&json!({"type": "context_message_added", "id": legacy_id}))
            .is_empty()
    );
}

/// An ack for anything else (a ping, a clear) is still a plain request reply:
/// promoting those to acceptance would wiggle a message that nobody sent.
#[test]
fn acking_an_unrelated_request_stays_a_reply() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"req": "clear", "id": 9, "session_id": "s1"}));
    let Outbound::Legacy(clear) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = clear["id"].as_u64().unwrap();
    let frames = state.legacy_event_to_api(&json!({"type": "ack", "id": legacy_id}));
    assert_eq!(frames[0].reply_to, Some(9));
    assert!(matches!(&frames[0].event, ApiEvent::Ok));
}

#[test]
fn ping_pong_roundtrip() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"req": "ping", "id": 9}));
    let Outbound::Legacy(ping) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = ping["id"].as_u64().unwrap();
    let frames = state.legacy_event_to_api(&json!({"type": "pong", "id": legacy_id}));
    assert_eq!(frames[0].reply_to, Some(9));
    assert!(matches!(frames[0].event, ApiEvent::Pong));
}

#[test]
fn history_reply_is_mapped() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"req": "get_history", "id": 4}));
    let Outbound::Legacy(get) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = get["id"].as_u64().unwrap();
    let frames = state.legacy_event_to_api(&json!({
        "type": "history",
        "id": legacy_id,
        "session_id": "s1",
        "messages": [{"role": "user", "content": "hi"}],
    }));
    match &frames[0].event {
        ApiEvent::History { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "user");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn unknown_legacy_events_are_dropped() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({"type": "swarm_event", "data": {}}));
    assert!(frames.is_empty());
}

#[test]
fn unknown_api_request_gets_error_reply() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "frobnicate", "id": 3}));
    let Outbound::Reply(frame) = &out[0] else {
        panic!("expected direct reply");
    };
    assert_eq!(frame.reply_to, Some(3));
    assert!(matches!(
        frame.event,
        ApiEvent::Error {
            code: ErrorCode::UnknownRequest,
            ..
        }
    ));
}

#[test]
fn error_routes_to_pending_request() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"req": "clear", "id": 7}));
    let Outbound::Legacy(clear) = &out[0] else {
        panic!("expected legacy outbound");
    };
    let legacy_id = clear["id"].as_u64().unwrap();
    let frames =
        state.legacy_event_to_api(&json!({"type": "error", "id": legacy_id, "message": "nope"}));
    assert_eq!(frames[0].reply_to, Some(7));
}

#[test]
fn background_notifications_become_progress_events() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "notification",
        "from_session": "background_task",
        "message": "**Background task progress** `t9` · `bash`\n\n[#####-----] 50% · Running tests (reported)",
    }));
    assert_eq!(frames.len(), 1);
    match &frames[0].event {
        ApiEvent::BackgroundProgress {
            session_id,
            task_id,
            percent,
            done,
            ..
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(task_id, "t9");
            assert_eq!(*percent, Some(50.0));
            assert!(!done);
        }
        other => panic!("unexpected background event: {other:?}"),
    }
}

/// A DM or a shared-context push is not progress, and inventing a bar for it
/// would put a phantom task on every client's screen.
#[test]
fn unrelated_notifications_are_dropped() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "notification",
        "from_session": "fox",
        "message": "hello from another agent",
    }));
    assert!(frames.is_empty());
}

/// The daemon answers a `ping` that arrives as the first frame on a connection
/// and then closes it, because it classifies ping as a one-shot lightweight
/// control request. Forwarding an unattached ping therefore destroys the
/// client's connection before it ever gets a session, which is the opposite of
/// what a liveness probe should do.
#[test]
fn ping_before_attach_is_answered_locally() {
    let mut state = BridgeState::default();
    let out = state.api_request_to_legacy(&json!({"req": "ping", "id": 4}));
    match out.as_slice() {
        [Outbound::Reply(frame)] => {
            assert_eq!(frame.reply_to, Some(4));
            assert_eq!(frame.event, ApiEvent::Pong);
        }
        other => panic!("ping must not reach the daemon before attach: {other:?}"),
    }
}

/// Once attached the connection is a normal session connection, so ping is a
/// genuine round trip and should measure the daemon, not the bridge.
#[test]
fn ping_after_attach_reaches_the_daemon() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({"req": "ping", "id": 5}));
    match out.as_slice() {
        [Outbound::Legacy(value)] => assert_eq!(value["type"], "ping"),
        other => panic!("expected a forwarded ping: {other:?}"),
    }
}

/// The daemon closes the connection on a stateful request that arrives before
/// a subscribe. Forwarding one therefore does not just fail the request: it
/// destroys the client's whole connection, taking every other in-flight
/// request with it, and the SDK sees a bare EPIPE. Answer locally.
#[test]
fn stateful_requests_before_attach_are_refused_locally() {
    for req in [
        "send_message",
        "cancel",
        "soft_interrupt",
        "clear",
        "rewind",
        "get_history",
    ] {
        let mut state = BridgeState::default();
        let out = state.api_request_to_legacy(&json!({
            "req": req,
            "id": 7,
            "session_id": "session_does_not_exist",
        }));
        assert_eq!(out.len(), 1, "{req} should produce exactly one reply");
        let Outbound::Reply(frame) = &out[0] else {
            panic!("{req} was forwarded to the daemon, which will close the connection");
        };
        assert_eq!(frame.reply_to, Some(7));
        match &frame.event {
            ApiEvent::Error { code, message } => {
                assert_eq!(*code, ErrorCode::UnknownSession, "{req}");
                assert!(
                    message.contains("session_does_not_exist"),
                    "{req} error should name the session: {message}"
                );
            }
            other => panic!("{req} expected an error frame, got {other:?}"),
        }
    }
}

/// The legacy protocol has no session field, so a request naming a *different*
/// session than the attached one would be applied to the attached one. A
/// `clear` or `rewind` aimed at the wrong id would then destroy a transcript
/// the caller never named.
#[test]
fn requests_for_another_session_do_not_hit_the_attached_one() {
    let mut state = state_with_session();
    let out = state.api_request_to_legacy(&json!({
        "req": "clear",
        "id": 9,
        "session_id": "some_other_session",
    }));
    let Outbound::Reply(frame) = &out[0] else {
        panic!("clear for another session must not reach the daemon");
    };
    match &frame.event {
        ApiEvent::Error { code, message } => {
            assert_eq!(*code, ErrorCode::UnknownSession);
            assert!(message.contains("s1") && message.contains("some_other_session"));
        }
        other => panic!("expected an error frame, got {other:?}"),
    }
}

/// The guard must not break the normal path: the attached session's own id,
/// and an omitted id, both still reach the daemon.
#[test]
fn attached_requests_still_reach_the_daemon() {
    let mut state = state_with_session();
    let named = state.api_request_to_legacy(&json!({
        "req": "get_history", "id": 1, "session_id": "s1",
    }));
    assert!(matches!(named[0], Outbound::Legacy(_)), "explicit id");

    let bare = state.api_request_to_legacy(&json!({"req": "get_history", "id": 2}));
    assert!(matches!(bare[0], Outbound::Legacy(_)), "omitted id");
}

/// Reading around without attaching is the entire point of `peek_session` and
/// `list_sessions`, so the attach guard must leave them alone.
#[test]
fn browsing_requests_work_without_attaching() {
    let _home = ScopedJcodeHome::new("browsing-without-attach");
    let mut state = BridgeState::default();
    for req in ["list_sessions", "peek_session", "ping"] {
        let out = state.api_request_to_legacy(&json!({
            "req": req, "id": 1, "session_id": "whatever",
        }));
        let Outbound::Reply(frame) = &out[0] else {
            panic!("{req} should be answered locally");
        };
        assert!(
            !matches!(frame.event, ApiEvent::Error { .. }),
            "{req} must not be refused by the attach guard: {:?}",
            frame.event
        );
    }
}

/// A client may pipeline: `create_session` then `send_message` without
/// awaiting the attach. The subscribe is already on the wire, so the daemon
/// will have a session by the time the message lands. Refusing here would
/// break the SDK's own `run()` path.
#[test]
fn a_message_pipelined_behind_create_session_is_forwarded() {
    let mut state = BridgeState::default();
    state.api_request_to_legacy(&json!({"req": "create_session", "id": 1}));
    let out = state.api_request_to_legacy(&json!({
        "req": "send_message", "id": 2, "content": "hi",
    }));
    let Outbound::Legacy(value) = &out[0] else {
        panic!("a pipelined message must reach the daemon, not be refused");
    };
    assert_eq!(value["type"], "message");
}

// --- Capabilities added to close the API coverage gaps --------------------

#[test]
fn a_rename_push_becomes_a_typed_event() {
    let mut state = state_with_session();
    let frames = state.legacy_event_to_api(&json!({
        "type": "session_renamed", "session_id": "s1",
        "title": "my session", "display_title": "my session",
    }));
    match &frames[0].event {
        ApiEvent::SessionRenamed {
            session_id,
            title,
            display_title,
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(title.as_deref(), Some("my session"));
            assert_eq!(display_title, "my session");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Every capability request is stateful, so none may be forwarded before the
/// connection is attached: the daemon closes the connection on those.
#[test]
fn capability_requests_need_an_attached_session() {
    for (req, extra) in [
        ("list_models", json!({})),
        ("set_model", json!({"model": "x"})),
        ("set_reasoning_effort", json!({"effort": "high"})),
        ("compact", json!({})),
        ("rename_session", json!({})),
        ("rewind_undo", json!({})),
        ("cancel_soft_interrupts", json!({})),
    ] {
        let mut state = BridgeState::default();
        let mut request = json!({"id": 1, "req": req});
        for (key, value) in extra.as_object().unwrap() {
            request[key] = value.clone();
        }
        let out = state.api_request_to_legacy(&request);
        match &out[..] {
            [Outbound::Reply(frame)] => match &frame.event {
                ApiEvent::Error { code, .. } => assert_eq!(
                    *code,
                    ErrorCode::UnknownSession,
                    "{req} should report an unattached session"
                ),
                other => panic!("{req}: unexpected {other:?}"),
            },
            other => panic!("{req} reached the daemon unattached: {other:?}"),
        }
    }
}

#[test]
fn another_sessions_broadcast_does_not_replace_the_attachment() {
    let mut state = BridgeState::default();
    let attach = state.api_request_to_legacy(&json!({
        "id": 7,
        "req": "attach_session",
        "session_id": "session_retriever_1_a",
    }));
    let state_id = match &attach[1] {
        Outbound::Legacy(value) => value["id"].as_u64().expect("state request id"),
        other => panic!("unexpected attach output: {other:?}"),
    };
    state.legacy_event_to_api(&json!({
        "type": "state",
        "id": state_id,
        "session_id": "session_retriever_1_a",
    }));

    state.legacy_event_to_api(&json!({
        "type": "session",
        "session_id": "session_pawprint_2_b",
    }));

    assert!(matches!(
        state
            .api_request_to_legacy(&json!({
                "id": 8,
                "req": "send_message",
                "session_id": "session_retriever_1_a",
                "content": "still routed to the attached session",
            }))
            .as_slice(),
        [Outbound::Legacy(_)]
    ));
}

#[test]
fn another_sessions_state_does_not_replace_the_attachment() {
    let mut state = BridgeState::default();
    let attach = state.api_request_to_legacy(&json!({
        "id": 7,
        "req": "attach_session",
        "session_id": "session_retriever_1_a",
    }));
    let state_id = match &attach[1] {
        Outbound::Legacy(value) => value["id"].as_u64().expect("state request id"),
        other => panic!("unexpected attach output: {other:?}"),
    };
    state.legacy_event_to_api(&json!({
        "type": "state",
        "id": state_id,
        "session_id": "session_retriever_1_a",
    }));

    state.legacy_event_to_api(&json!({
        "type": "state",
        "id": state_id + 100,
        "session_id": "session_pawprint_2_b",
    }));

    assert!(matches!(
        state
            .api_request_to_legacy(&json!({
                "id": 8,
                "req": "send_message",
                "session_id": "session_retriever_1_a",
                "content": "still routed to the attached session",
            }))
            .as_slice(),
        [Outbound::Legacy(_)]
    ));
}

#[test]
fn legacy_request_ids_are_unique_across_bridge_connections() {
    let mut first = BridgeState::default();
    let mut second = BridgeState::default();
    let first_attach = first.api_request_to_legacy(&json!({
        "id": 1,
        "req": "attach_session",
        "session_id": "session_first",
    }));
    let second_attach = second.api_request_to_legacy(&json!({
        "id": 1,
        "req": "attach_session",
        "session_id": "session_second",
    }));
    let request_id = |outbound: &[Outbound]| match &outbound[1] {
        Outbound::Legacy(value) => value["id"].as_u64().expect("state request id"),
        other => panic!("unexpected attach output: {other:?}"),
    };

    assert_ne!(request_id(&first_attach), request_id(&second_attach));
}

#[test]
fn colliding_state_id_for_another_target_does_not_complete_attach() {
    let mut state = BridgeState::default();
    let attach = state.api_request_to_legacy(&json!({
        "id": 7,
        "req": "attach_session",
        "session_id": "session_wanted",
    }));
    let state_id = match &attach[1] {
        Outbound::Legacy(value) => value["id"].as_u64().expect("state request id"),
        other => panic!("unexpected attach output: {other:?}"),
    };

    assert!(
        state
            .legacy_event_to_api(&json!({
                "type": "state",
                "id": state_id,
                "session_id": "session_other",
            }))
            .is_empty()
    );
    assert!(state.session_id.is_none());
}

#[test]
fn runtime_info_reports_the_active_provider_and_complete_route_catalog() {
    let mut state = state_with_session();
    state.legacy_event_to_api(&json!({
        "type": "available_models_updated",
        "provider_name": "anthropic",
        "provider_model": "claude-sonnet",
        "reasoning_effort": "high",
        "available_models": ["claude-sonnet", "gemini-pro"],
        "available_model_routes": [
            {
                "model": "claude-sonnet",
                "provider": "anthropic",
                "api_method": "messages",
                "available": true,
                "detail": "ready"
            },
            {
                "model": "gemini-pro",
                "provider": "gemini",
                "api_method": "generateContent",
                "available": false,
                "detail": "credential missing"
            }
        ]
    }));

    let event = only_reply_event(state.api_request_to_legacy(&json!({
        "req": "get_runtime_info",
        "id": 4,
        "session_id": "s1"
    })));
    let ApiEvent::RuntimeInfo {
        session_id,
        provider,
        model,
        reasoning_effort,
        routes,
    } = event
    else {
        panic!("expected runtime info, got {event:?}");
    };
    assert_eq!(session_id, "s1");
    assert_eq!(provider.as_deref(), Some("anthropic"));
    assert_eq!(model.as_deref(), Some("claude-sonnet"));
    assert_eq!(reasoning_effort.as_deref(), Some("high"));
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[1].provider, "gemini");
    assert!(!routes[1].available);
}

include!("translate_tests/model_controls.rs");
include!("translate_tests/owner_storage.rs");
