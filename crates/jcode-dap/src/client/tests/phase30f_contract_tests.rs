use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;

use super::super::next_sequence;
use super::super::transaction::{
    Admission, AdmissionPhase, ClientInstance, CorrelationState, OutcomeState, RequestTransaction,
    SettlementReason,
};
use super::BlockingWriter;
use crate::testing::FakeAdapter;
use crate::{DapError, Event, Message, Request, Response, decode_message, encode_message};

fn request(message: Message) -> Request {
    match message {
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

fn payload(frame: &[u8]) -> &[u8] {
    let separator = frame
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    &frame[separator..]
}

#[test]
fn protocol_sequence_fields_accept_positive_int32_boundaries() {
    for sequence in [1_i64, i64::from(i32::MAX)] {
        let request = Request::new(sequence, "threads", None);
        assert!(matches!(
            decode_message(payload(&encode_message(&request).unwrap())).unwrap(),
            Message::Request(decoded) if decoded.seq == sequence
        ));
    }
}

#[test]
fn public_protocol_encode_decode_preserve_positive_i64_compatibility() {
    let sequence = i64::MAX;
    for encoded in [
        encode_message(&Request::new(sequence, "threads", None)).unwrap(),
        encode_message(&Response::success(sequence, sequence, "threads", None)).unwrap(),
        encode_message(&Event::new(sequence, "stopped", None)).unwrap(),
    ] {
        assert!(decode_message(payload(&encoded)).is_ok());
    }
}

#[test]
fn adapter_protocol_decode_accepts_zero_sequence_for_lldb_compatibility() {
    for payload in [
        br#"{"seq":0,"type":"response","request_seq":1,"success":true,"command":"initialize"}"#
            .as_slice(),
        br#"{"seq":0,"type":"event","event":"initialized"}"#.as_slice(),
    ] {
        assert!(decode_message(payload).is_ok());
    }
    assert!(matches!(
        decode_message(br#"{"seq":0,"type":"request","command":"runInTerminal"}"#),
        Err(DapError::InvalidMessage(message))
            if message == "request seq must be positive; response and event seq must be non-negative"
    ));
}

#[test]
fn outbound_protocol_encode_rejects_nonpositive_sequence() {
    for encoded in [
        encode_message(&Request::new(0, "threads", None)),
        encode_message(&Response::success(0, 1, "threads", None)),
        encode_message(&Event::new(0, "stopped", None)),
    ] {
        assert!(
            matches!(encoded, Err(DapError::InvalidMessage(message)) if message == "seq must be positive")
        );
    }
}

#[test]
fn positive_int32_numeric_representations_are_exact() {
    let counter = std::sync::atomic::AtomicI64::new(1);
    assert_eq!(next_sequence(&counter).unwrap(), 1);
    counter.store(i64::from(i32::MAX), Ordering::Release);
    assert_eq!(next_sequence(&counter).unwrap(), i64::from(i32::MAX));
    assert!(next_sequence(&counter).is_err());
    assert!(next_sequence(&counter).is_err());
}

#[test]
fn first_terminal_transaction_outcome_wins() {
    let instance = ClientInstance::new();
    let transaction = RequestTransaction::new(instance.clone(), true);
    assert!(transaction.commit_admission(7, || true));
    assert!(transaction.route_response(&instance, 7, || true));
    assert!(!transaction.settle_caller_drop().won);
    assert!(!transaction.settle_deadline().won);
    assert!(transaction.settle_response());
    assert_eq!(
        transaction.observer().snapshot().outcome,
        OutcomeState::Response
    );
    assert_eq!(
        transaction.observer().snapshot().correlation,
        CorrelationState::Settled
    );
}

#[tokio::test]
async fn admission_observer_phases_are_exact_and_monotonic() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let tracked = client
        .tracked_request("threads", None, Duration::from_secs(1))
        .unwrap();
    let observer = tracked.admission_observer();
    assert_eq!(
        observer.snapshot().admission_phase,
        AdmissionPhase::PreAdmission
    );
    assert!(!observer.is_admitted());
    assert!(observer.is_exact_client(&client.inner.shared.client_instance));

    let pending = tokio::spawn(tracked);
    let sent = request(adapter.recv().await.unwrap());
    assert_eq!(
        observer.snapshot().admission_phase,
        AdmissionPhase::Admitted
    );
    assert!(observer.is_admitted());
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(pending.await.unwrap().is_ok());
    assert_eq!(observer.snapshot().admission_phase, AdmissionPhase::Settled);
    assert_eq!(observer.snapshot().outcome, OutcomeState::Response);
}

#[tokio::test]
async fn admission_observer_marks_enqueue_locked_writer_queue_commit() {
    let (reader, _adapter) = tokio::io::duplex(64);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let client = super::super::DapClient::start_split(
        reader,
        BlockingWriter {
            started: Some(started_tx),
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
    );
    let tracked = client
        .tracked_request("queued", None, Duration::from_secs(1))
        .unwrap();
    let observer = tracked.admission_observer();
    let pending = tokio::spawn(tracked);
    started_rx.await.unwrap();
    assert!(observer.is_admitted());
    assert_eq!(
        observer.snapshot().admission_phase,
        AdmissionPhase::Admitted
    );
    pending.abort();
    let _cancelled = pending.await;
}

#[tokio::test]
async fn inbound_decode_spawn_blocking_is_bounded() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let mut events = client.subscribe_events();
    assert_eq!(client.inner.shared.decoder.available_permits(), 1);
    let held = Arc::clone(&client.inner.shared.decoder)
        .acquire_owned()
        .await
        .unwrap();
    adapter.event("stopped", None).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), events.recv())
            .await
            .is_err()
    );
    assert_eq!(client.inner.shared.decoder.available_permits(), 0);
    drop(held);
    assert_eq!(events.recv().await.unwrap().event, "stopped");
    tokio::task::yield_now().await;
    assert_eq!(client.inner.shared.decoder.available_permits(), 1);
}

async fn exercise_caller_drop_after_admission() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(false);
    let tracked = client
        .tracked_request("abandoned", None, Duration::from_secs(10))
        .unwrap();
    let observer = tracked.admission_observer();
    let pending = tokio::spawn(tracked);
    let abandoned = request(adapter.recv().await.unwrap());
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(
        observer.snapshot().outcome,
        OutcomeState::AbandonedCaller {
            admission: Admission::PostAdmission,
        }
    );
    assert_eq!(
        observer.snapshot().correlation,
        CorrelationState::SettledWithoutResponse(SettlementReason::CallerDrop)
    );

    adapter
        .respond_ok(&abandoned, Some(json!({"late": true})))
        .await
        .unwrap();
    let following = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("following", None, Duration::from_secs(1))
                .await
        }
    });
    let sent = request(adapter.recv().await.unwrap());
    assert_eq!(sent.command, "following");
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(following.await.unwrap().is_ok());
}

#[tokio::test]
async fn caller_drop_after_admission_promptly_settles_and_ignores_late_response() {
    exercise_caller_drop_after_admission().await;
}

#[tokio::test]
async fn late_response_cleanup_preserves_next_request_routing() {
    exercise_caller_drop_after_admission().await;
}

#[tokio::test]
async fn inspection_unsupported_supports_cancel_request_emits_zero_cancel_requests() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slow", None, Duration::from_millis(10))
                .await
        }
    });
    let _sent = request(adapter.recv().await.unwrap());
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn inspection_exact_true_supports_cancel_request_allows_at_most_one_cancel_attempt() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(true);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slow", None, Duration::from_millis(10))
                .await
        }
    });
    let original = request(adapter.recv().await.unwrap());
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    let cancel = request(adapter.recv().await.unwrap());
    assert_eq!(cancel.command, "cancel");
    assert_eq!(cancel.arguments.unwrap()["requestId"], original.seq);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancel_is_never_enqueued_after_response_received() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(true);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("fast", None, Duration::from_millis(40))
                .await
        }
    });
    let sent = request(adapter.recv().await.unwrap());
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(pending.await.unwrap().is_ok());
    assert!(
        tokio::time::timeout(Duration::from_millis(60), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancel_request_id_uses_positive_int32_domain() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(true);
    client
        .inner
        .shared
        .next_seq
        .store(i64::from(i32::MAX) - 1, Ordering::Release);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slow", None, Duration::from_millis(10))
                .await
        }
    });
    let original = request(adapter.recv().await.unwrap());
    assert_eq!(original.seq, i64::from(i32::MAX) - 1);
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    let cancel = request(adapter.recv().await.unwrap());
    assert_eq!(cancel.seq, i64::from(i32::MAX));
    assert_eq!(cancel.arguments.unwrap()["requestId"], original.seq);
}

#[tokio::test]
async fn shared_private_outbound_allocator_covers_all_requests_cancel_and_reverse_responses() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client.set_supports_cancel_request(true);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slow", None, Duration::from_millis(10))
                .await
        }
    });
    let ordinary = request(adapter.recv().await.unwrap());
    assert_eq!(ordinary.seq, 1);
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    let cancel = request(adapter.recv().await.unwrap());
    assert_eq!(cancel.seq, 2);
    adapter
        .reverse_request("runInTerminal", None)
        .await
        .unwrap();
    let Message::Response(reverse) = adapter.recv().await.unwrap() else {
        panic!("expected reverse response")
    };
    assert_eq!(reverse.seq, 3);
}

#[tokio::test]
async fn shared_outbound_allocator_exhaustion_closes_client_and_never_reuses() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    client
        .inner
        .shared
        .next_seq
        .store(i64::from(i32::MAX), Ordering::Release);
    let last = tokio::spawn({
        let client = client.clone();
        async move { client.request("last", None, Duration::from_secs(1)).await }
    });
    let sent = request(adapter.recv().await.unwrap());
    assert_eq!(sent.seq, i64::from(i32::MAX));
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(last.await.unwrap().is_ok());

    assert!(matches!(
        client
            .request("exhausted", None, Duration::from_secs(1))
            .await,
        Err(DapError::InvalidMessage(_))
    ));
    assert_eq!(
        client
            .request("closed", None, Duration::from_secs(1))
            .await
            .unwrap_err(),
        DapError::TransportClosed
    );
    assert_eq!(client.inner.shared.next_seq.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn old_and_new_clients_same_sequence_isolate_late_old_response() {
    let (old, mut old_adapter) = FakeAdapter::pair(4096);
    let (new, mut new_adapter) = FakeAdapter::pair(4096);
    let old_pending = tokio::spawn({
        let old = old.clone();
        async move { old.request("old", None, Duration::from_secs(1)).await }
    });
    let new_pending = tokio::spawn({
        let new = new.clone();
        async move { new.request("new", None, Duration::from_secs(1)).await }
    });
    let old_request = request(old_adapter.recv().await.unwrap());
    let new_request = request(new_adapter.recv().await.unwrap());
    assert_eq!(old_request.seq, new_request.seq);
    assert!(!Arc::ptr_eq(&old.inner, &new.inner));

    old_adapter
        .respond_ok(&old_request, Some(json!("old")))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), async {
            while !new_pending.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err()
    );
    new_adapter
        .respond_ok(&new_request, Some(json!("new")))
        .await
        .unwrap();
    assert_eq!(old_pending.await.unwrap().unwrap().body, Some(json!("old")));
    assert_eq!(new_pending.await.unwrap().unwrap().body, Some(json!("new")));
}
