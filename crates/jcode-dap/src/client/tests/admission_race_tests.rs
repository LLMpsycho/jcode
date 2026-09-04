use std::sync::Arc;
use std::time::Duration;

use super::super::transaction::{AdmissionPhase, OutcomeState, SettlementReason};
use super::super::{ClientCloseCause, begin_transport_termination_after_contention, fail_pending};
use crate::testing::FakeAdapter;
use crate::{DapError, Message, Request};

fn request(message: Message) -> Request {
    match message {
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

#[tokio::test]
async fn invalidation_before_admission_synchronously_vetoes_queue_commit() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let lane = Arc::clone(&client.inner.shared.serializer)
        .acquire_owned()
        .await
        .unwrap();
    let tracked = client
        .tracked_request("never-admitted", None, Duration::from_secs(1))
        .unwrap();
    let invalidator = tracked.invalidator();
    let pending = tokio::spawn(tracked);
    tokio::task::yield_now().await;
    assert_eq!(invalidator.admission_phase(), AdmissionPhase::PreAdmission);
    let snapshot = invalidator.invalidate();
    assert_eq!(
        snapshot.outcome,
        OutcomeState::NotDispatched(SettlementReason::Invalidated)
    );
    drop(lane);
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        DapError::TransportClosed
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admission_gate_false_atomically_rejects_original_queue_commit() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let tracked = client
        .tracked_request_with_admission_gate(
            "gate-vetoed",
            None,
            Duration::from_secs(1),
            Box::new(|| false),
        )
        .unwrap();
    let observer = tracked.admission_observer();
    assert_eq!(tracked.await.unwrap_err(), DapError::TransportClosed);
    assert_eq!(
        observer.snapshot().outcome,
        OutcomeState::NotDispatched(SettlementReason::Invalidated)
    );
    assert!(!observer.is_admitted());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_linearizes_with_admission_before_pending_installation() {
    let (client, _adapter) = FakeAdapter::pair(4096);
    let gate_entered = Arc::new(std::sync::Barrier::new(2));
    let gate_release = Arc::new(std::sync::Barrier::new(2));
    let tracked = client
        .tracked_request_with_admission_gate(
            "close-race",
            None,
            Duration::from_secs(1),
            Box::new({
                let gate_entered = Arc::clone(&gate_entered);
                let gate_release = Arc::clone(&gate_release);
                move || {
                    gate_entered.wait();
                    gate_release.wait();
                    true
                }
            }),
        )
        .unwrap();
    let pending = tokio::spawn(tracked);
    tokio::task::spawn_blocking(move || gate_entered.wait())
        .await
        .unwrap();

    let (close_contended, close_contended_rx) = tokio::sync::oneshot::channel();
    let (close_done, close_done_rx) = std::sync::mpsc::channel();
    let shared = Arc::clone(&client.inner.shared);
    let close_thread = std::thread::spawn(move || {
        if let Some(pending) = begin_transport_termination_after_contention(
            &shared,
            ClientCloseCause::ExplicitClose,
            || {
                let _ignored = close_contended.send(());
            },
        ) {
            fail_pending(pending, DapError::TransportClosed);
        }
        let _ignored = close_done.send(());
    });
    close_contended_rx.await.unwrap();
    assert!(close_done_rx.try_recv().is_err());

    tokio::task::spawn_blocking(move || gate_release.wait())
        .await
        .unwrap();
    close_thread.join().unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        DapError::TransportClosed
    );
}

#[tokio::test]
async fn invalidation_after_admission_synchronously_removes_exact_pending_correlation() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let tracked = client
        .tracked_request("invalidate-admitted", None, Duration::from_secs(1))
        .unwrap();
    let invalidator = tracked.invalidator();
    let pending = tokio::spawn(tracked);
    let original = request(adapter.recv().await.unwrap());
    assert_eq!(invalidator.admission_phase(), AdmissionPhase::Admitted);
    let snapshot = invalidator.invalidate();
    assert_eq!(
        snapshot.outcome,
        OutcomeState::Failed(SettlementReason::Invalidated)
    );
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        DapError::TransportClosed
    );

    adapter.respond_ok(&original, None).await.unwrap();
    let following = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("following-invalidation", None, Duration::from_secs(1))
                .await
        }
    });
    let sent = request(adapter.recv().await.unwrap());
    assert_eq!(sent.command, "following-invalidation");
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(following.await.unwrap().is_ok());
}

#[tokio::test]
async fn shared_outbound_lane_orders_waiting_ordinary_before_reverse_and_skips_busy_cancel() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(true);
    let timed_out = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("will-time-out", None, Duration::from_millis(20))
                .await
        }
    });
    let first = request(adapter.recv().await.unwrap());
    assert_eq!(first.seq, 1);

    let lane = Arc::clone(&client.inner.shared.serializer)
        .acquire_owned()
        .await
        .unwrap();
    let ordinary = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("waiting-ordinary", None, Duration::from_secs(1))
                .await
        }
    });
    tokio::task::yield_now().await;
    adapter
        .reverse_request("runInTerminal", None)
        .await
        .unwrap();
    assert!(matches!(
        timed_out.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    drop(lane);

    let second = request(adapter.recv().await.unwrap());
    assert_eq!(second.command, "waiting-ordinary");
    assert_eq!(second.seq, 2);
    let Message::Response(reverse) = adapter.recv().await.unwrap() else {
        panic!("expected reverse response")
    };
    assert_eq!(reverse.seq, 3);
    adapter.respond_ok(&second, None).await.unwrap();
    assert!(ordinary.await.unwrap().is_ok());
}
