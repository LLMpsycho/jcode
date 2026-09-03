use serde_json::json;

use super::*;

fn output(value: &str) -> Event {
    Event::new(
        1,
        "output",
        Some(json!({"category":"stdout","output":value})),
    )
}

#[test]
fn workspace_key_canonicalizes_and_preserves_identity() {
    let key = DebugWorkspaceKey::new(Path::new("."), "tree-a").unwrap();
    assert!(key.canonical_root().is_absolute());
    assert_eq!(key.worktree_identity(), "tree-a");
}

#[test]
fn state_kind_and_terminal_are_stable() {
    assert_eq!(
        DebugSessionState::Reserved.kind(),
        DebugSessionStateKind::Reserved
    );
    let ended = DebugSessionState::Ended(DebugSessionEnd {
        reason: DebugSessionEndReason::Requested,
        cleanup_error: None,
    });
    assert!(ended.is_terminal());
}

#[test]
fn stopped_event_body_is_normalized() {
    let event = Event::new(
        1,
        "stopped",
        Some(
            json!({"reason":"breakpoint","description":"hit","threadId":7,"allThreadsStopped":true}),
        ),
    );
    match parse_event(event).unwrap() {
        SessionEvent::Stopped(state) => assert_eq!(
            state,
            StoppedState {
                reason: "breakpoint".into(),
                description: Some("hit".into()),
                thread_id: Some(7),
                all_threads_stopped: true,
            }
        ),
        _ => panic!("expected stopped"),
    }
}

#[test]
fn malformed_lifecycle_body_fails_closed() {
    assert!(parse_event(Event::new(1, "stopped", Some(json!({})))).is_err());
    assert!(parse_event(Event::new(1, "continued", Some(json!(1)))).is_err());
    assert!(parse_event(Event::new(1, "exited", Some(json!({})))).is_err());
}

#[test]
fn output_event_is_normalized_without_retaining_json() {
    match parse_event(output("hello")).unwrap() {
        SessionEvent::Output(DebugOutputCategory::Stdout, value) => assert_eq!(value, "hello"),
        _ => panic!("expected output"),
    }
}

#[test]
fn output_ring_evicts_by_count_with_monotonic_cursors() {
    let mut ring = OutputRing::new(2, 100);
    for value in ["a", "b", "c"] {
        ring.push(DebugOutputCategory::Console, value.into());
    }
    let page = ring.page(None, 10);
    assert_eq!(
        page.records
            .iter()
            .map(|r| r.output.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert_eq!(page.status.evicted_events, 1);
    assert_eq!(page.status.next_cursor, DebugOutputCursor(4));
}

#[test]
fn output_ring_evicts_by_byte_count() {
    let mut ring = OutputRing::new(10, 4);
    ring.push(DebugOutputCategory::Console, "abc".into());
    ring.push(DebugOutputCategory::Console, "de".into());
    let page = ring.page(None, 10);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].output, "de");
    assert_eq!(page.status.retained_bytes, 2);
}

#[test]
fn oversized_output_keeps_utf8_safe_tail() {
    let mut ring = OutputRing::new(2, 5);
    ring.push(DebugOutputCategory::Console, "aé😊".into());
    let record = &ring.page(None, 10).records[0];
    assert_eq!(record.output, "😊");
    assert_eq!(record.truncated_prefix_bytes, 3);
}

#[test]
fn output_page_is_exclusive_and_reports_evicted_history() {
    let mut ring = OutputRing::new(2, 100);
    ring.push(DebugOutputCategory::Console, "a".into());
    ring.push(DebugOutputCategory::Console, "b".into());
    ring.push(DebugOutputCategory::Console, "c".into());
    let page = ring.page(Some(DebugOutputCursor(0)), 1);
    assert!(page.requested_history_was_evicted);
    assert_eq!(page.records[0].cursor, DebugOutputCursor(2));
}

#[test]
fn source_loss_is_distinct_from_ring_eviction() {
    let mut ring = OutputRing::new(1, 10);
    ring.push(DebugOutputCategory::Console, "a".into());
    ring.push(DebugOutputCategory::Console, "b".into());
    ring.add_source_loss(3);
    let status = ring.status();
    assert_eq!(status.evicted_events, 1);
    assert_eq!(status.source_dropped_events, 3);
}

#[test]
fn zero_page_limit_returns_status_only() {
    let mut ring = OutputRing::new(2, 10);
    ring.push(DebugOutputCategory::Console, "a".into());
    let page = ring.page(None, 0);
    assert!(page.records.is_empty());
    assert_eq!(page.status.retained_events, 1);
}
