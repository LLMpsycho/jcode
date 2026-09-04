use std::sync::Arc;

use super::*;
use crate::Event;

async fn wait_for_queue(entry: &SessionEntry, length: usize, overflowed: bool) {
    timeout(Duration::from_secs(1), async {
        loop {
            let ready = {
                let data = lock(&entry.data);
                data.breakpoints
                    .in_flight
                    .as_ref()
                    .is_some_and(|transaction| {
                        transaction.bounded_events.len() == length
                            && transaction.overflowed == overflowed
                    })
            };
            if ready {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn current_operation_source_mutation_between_initial_hash_and_primary_dispatch_emits_no_traffic()
 {
    let mut f = fixture("owner");
    let entry = f.manager.core.entry(f.id).unwrap();
    let held = Arc::clone(&entry.breakpoint_test_gates.0)
        .acquire_owned()
        .await
        .unwrap();
    let manager = f.manager.clone();
    let id = f.id;
    let source = f.source.clone();
    let task = tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
            )
            .await
    });
    timeout(Duration::from_secs(1), async {
        loop {
            if lock(&entry.data).breakpoints.in_flight.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    std::fs::write(&f.source, b"changed after initial hash").unwrap();
    drop(held);
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::DebugSourceChangedDuringOperation { .. })
    ));
    assert!(
        f.manager
            .breakpoints("owner", f.id)
            .unwrap()
            .sources
            .is_empty()
    );
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

async fn start_set(
    f: &mut Fixture,
) -> (
    tokio::task::JoinHandle<Result<DebugBreakpointMutationResult>>,
    crate::Request,
    Arc<SessionEntry>,
) {
    let manager = f.manager.clone();
    let id = f.id;
    let source = f.source.clone();
    let task = tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
            )
            .await
    });
    let request = request(&mut f.adapter).await;
    let entry = f.manager.core.authorized_entry("owner", f.id).unwrap();
    (task, request, entry)
}

#[tokio::test]
async fn response_precedes_higher_seq_id_only_event_but_supervisor_queues_event_first() {
    let mut f = fixture("owner");
    let (task, request, entry) = start_set(&mut f).await;
    let held = Arc::clone(&entry.source_hash)
        .acquire_owned()
        .await
        .unwrap();
    f.adapter
        .send(&Response::success(
            10,
            request.seq,
            "setBreakpoints",
            Some(json!({"breakpoints":[{"id":42,"verified":true}]})),
        ))
        .await
        .unwrap();
    f.adapter
        .send(&Event::new(
            11,
            "breakpoint",
            Some(json!({"reason":"changed","breakpoint":{"id":42,"verified":false}})),
        ))
        .await
        .unwrap();
    wait_for_queue(&entry, 1, false).await;
    drop(held);
    let result = task.await.unwrap().unwrap();
    assert!(!result.source.breakpoints[0].verified);
}

#[tokio::test]
async fn newer_queued_events_delivered_out_of_sequence_apply_in_ascending_seq_order() {
    let mut f = fixture("owner");
    let (task, request, entry) = start_set(&mut f).await;
    let held = Arc::clone(&entry.source_hash)
        .acquire_owned()
        .await
        .unwrap();
    f.adapter
        .send(&Response::success(
            10,
            request.seq,
            "setBreakpoints",
            Some(json!({"breakpoints":[{"id":42,"verified":true}]})),
        ))
        .await
        .unwrap();
    for (seq, verified) in [(12, true), (11, false)] {
        f.adapter
            .send(&Event::new(
                seq,
                "breakpoint",
                Some(json!({"reason":"changed","breakpoint":{"id":42,"verified":verified}})),
            ))
            .await
            .unwrap();
    }
    wait_for_queue(&entry, 2, false).await;
    drop(held);
    assert!(task.await.unwrap().unwrap().source.breakpoints[0].verified);
}

#[tokio::test]
async fn public_queue_overflow_returns_indeterminate_without_synchronized_claim() {
    let mut f = fixture_with_operations(
        "owner",
        DebugOperationConfig {
            operation_timeout: Duration::from_secs(2),
            max_queued_breakpoint_events: 1,
            ..Default::default()
        },
    );
    let (task, request, entry) = start_set(&mut f).await;
    let held = Arc::clone(&entry.source_hash)
        .acquire_owned()
        .await
        .unwrap();
    f.adapter
        .send(&Response::success(
            10,
            request.seq,
            "setBreakpoints",
            Some(json!({"breakpoints":[{"id":42,"verified":true}]})),
        ))
        .await
        .unwrap();
    for (seq, body) in [
        (
            11,
            json!({"reason":"changed","breakpoint":{"id":42,"verified":false}}),
        ),
        (
            12,
            json!({"reason":"changed","breakpoint":{"id":999,"verified":false}}),
        ),
    ] {
        f.adapter
            .send(&Event::new(seq, "breakpoint", Some(body)))
            .await
            .unwrap();
    }
    wait_for_queue(&entry, 1, true).await;
    drop(held);
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::BreakpointReconciliationIndeterminate { .. })
    ));
    let snapshot = f.manager.breakpoints("owner", f.id).unwrap();
    assert_eq!(
        snapshot.sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );
}

#[tokio::test]
async fn all_unresolved_event_forms_queue_through_supervisor_at_boundary_and_overflow_at_plus_one()
{
    for overflow in [false, true] {
        let mut f = fixture_with_operations(
            "owner",
            DebugOperationConfig {
                operation_timeout: Duration::from_secs(2),
                max_queued_breakpoint_events: 4,
                ..Default::default()
            },
        );
        let (task, request, entry) = start_set(&mut f).await;
        let held = Arc::clone(&entry.source_hash)
            .acquire_owned()
            .await
            .unwrap();
        f.adapter
            .send(&Response::success(
                10,
                request.seq,
                "setBreakpoints",
                Some(json!({"breakpoints":[{"id":42,"verified":true}]})),
            ))
            .await
            .unwrap();
        let mut events = vec![
            json!({"reason":"changed","breakpoint":{"id":42,"verified":false}}),
            json!({"reason":"changed","breakpoint":{"id":999,"verified":false}}),
            json!({"reason":"changed","breakpoint":{"verified":false}}),
            json!({"reason":"changed","breakpoint":{"id":77,"verified":false,"source":{"path":f.root.join("unknown.rs")}}}),
        ];
        if overflow {
            events.push(json!({"reason":"removed","breakpoint":{"id":42}}));
        }
        for (offset, body) in events.into_iter().enumerate() {
            f.adapter
                .send(&Event::new(11 + offset as i64, "breakpoint", Some(body)))
                .await
                .unwrap();
        }
        wait_for_queue(&entry, 4, overflow).await;
        drop(held);
        let result = task.await.unwrap();
        if overflow {
            assert!(matches!(
                result,
                Err(DapError::BreakpointReconciliationIndeterminate { .. })
            ));
            assert_eq!(
                f.manager.breakpoints("owner", f.id).unwrap().sources[0].synchronization,
                DebugBreakpointSynchronization::Indeterminate
            );
        } else {
            assert!(result.is_ok());
        }
    }
}
