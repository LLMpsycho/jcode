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
