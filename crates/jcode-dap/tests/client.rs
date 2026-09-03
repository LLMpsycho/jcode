use std::time::Duration;

use jcode_dap::testing::FakeAdapter;
use jcode_dap::{
    DapError, EVENT_CHANNEL_CAPACITY, MAX_RETAINED_EVENT_BYTES, MAX_RETAINED_EVENT_SIZE, Message,
    Response,
};
use serde_json::json;

fn request(message: Message) -> jcode_dap::Request {
    match message {
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

#[tokio::test]
async fn correlates_out_of_order_responses_and_monotonic_sequences() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let first = tokio::spawn({
        let client = client.clone();
        async move { client.request("first", None, Duration::from_secs(1)).await }
    });
    let second = tokio::spawn({
        let client = client.clone();
        async move { client.request("second", None, Duration::from_secs(1)).await }
    });
    let one = request(adapter.recv().await.unwrap());
    let two = request(adapter.recv().await.unwrap());
    assert!(one.seq < two.seq);
    let (first_request, second_request) = if one.command == "first" {
        (one, two)
    } else {
        (two, one)
    };
    adapter
        .respond_ok(&second_request, Some(json!(2)))
        .await
        .unwrap();
    adapter
        .respond_ok(&first_request, Some(json!(1)))
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().unwrap().body, Some(json!(1)));
    assert_eq!(second.await.unwrap().unwrap().body, Some(json!(2)));
}

#[tokio::test]
async fn concurrent_request_sequences_are_monotonic_in_wire_order() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let requests = (0..32)
        .map(|index| {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .request(format!("request-{index}"), None, Duration::from_secs(1))
                    .await
            })
        })
        .collect::<Vec<_>>();
    let mut previous = 0;
    for _ in 0..requests.len() {
        let sent = request(adapter.recv().await.unwrap());
        assert!(sent.seq > previous);
        previous = sent.seq;
        adapter.respond_ok(&sent, None).await.unwrap();
    }
    for request in requests {
        assert!(request.await.unwrap().is_ok());
    }
}

#[tokio::test]
async fn publishes_events_and_observes_then_rejects_reverse_requests() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let mut events = client.subscribe_events();
    let mut reverse = client.subscribe_reverse_requests();
    adapter
        .event("stopped", Some(json!({"reason":"pause"})))
        .await
        .unwrap();
    assert_eq!(events.recv().await.unwrap().event, "stopped");
    let reverse_seq = adapter
        .reverse_request("runInTerminal", Some(json!({"cwd":"/"})))
        .await
        .unwrap();
    assert_eq!(reverse.recv().await.unwrap().seq, reverse_seq);
    let Message::Response(response) = adapter.recv().await.unwrap() else {
        panic!("expected rejection response")
    };
    assert_eq!(response.request_seq, reverse_seq);
    assert_eq!(response.command, "runInTerminal");
    assert!(!response.success);
}

#[tokio::test]
async fn timeout_cancellation_is_conditional_and_best_effort() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let timed_out = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slow", None, Duration::from_millis(20))
                .await
        }
    });
    let original = request(adapter.recv().await.unwrap());
    assert!(matches!(
        timed_out.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), adapter.recv())
            .await
            .is_err()
    );

    client.set_supports_cancel_request(true);
    let timed_out = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("slower", None, Duration::from_millis(20))
                .await
        }
    });
    let second = request(adapter.recv().await.unwrap());
    assert!(second.seq > original.seq);
    assert!(matches!(
        timed_out.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    let cancel = request(adapter.recv().await.unwrap());
    assert_eq!(cancel.command, "cancel");
    assert_eq!(cancel.arguments.unwrap()["requestId"], second.seq);
}

#[tokio::test]
async fn propagates_transport_exit_and_rejects_future_requests() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("threads", None, Duration::from_secs(5))
                .await
        }
    });
    let _request = adapter.recv().await.unwrap();
    drop(adapter);
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        DapError::TransportClosed
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        client
            .request("again", None, Duration::from_secs(1))
            .await
            .unwrap_err(),
        DapError::TransportClosed
    );
}

#[tokio::test]
async fn rejects_response_command_mismatch() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("threads", None, Duration::from_secs(1))
                .await
        }
    });
    let sent = request(adapter.recv().await.unwrap());
    adapter
        .send(&Response::success(1, sent.seq, "stackTrace", None))
        .await
        .unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::InvalidMessage(_))
    ));
}

#[tokio::test]
async fn malformed_adapter_payload_fails_pending_and_future_requests() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("threads", None, Duration::from_secs(1))
                .await
        }
    });
    let _sent = adapter.recv().await.unwrap();
    adapter
        .send_raw(b"Content-Length: 1\r\n\r\n{")
        .await
        .unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(DapError::InvalidJson(_))
    ));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        client
            .request("again", None, Duration::from_secs(1))
            .await
            .unwrap_err(),
        DapError::TransportClosed
    );
}

#[tokio::test]
async fn cancelling_request_futures_releases_pending_capacity() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    for _ in 0..1025 {
        let pending = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request("abandoned", None, Duration::from_secs(30))
                    .await
            }
        });
        let _sent = request(adapter.recv().await.unwrap());
        pending.abort();
        let _cancelled = pending.await;
    }

    let final_request = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("still-usable", None, Duration::from_secs(1))
                .await
        }
    });
    let sent = request(adapter.recv().await.unwrap());
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(final_request.await.unwrap().is_ok());
}

#[tokio::test]
async fn abort_during_blocked_write_does_not_corrupt_the_frame() {
    let (client, mut adapter) = FakeAdapter::pair(32);
    let abandoned = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request(
                    "large",
                    Some(json!({"payload": "x".repeat(16 * 1024)})),
                    Duration::from_secs(5),
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());

    let complete = request(adapter.recv().await.unwrap());
    assert_eq!(complete.command, "large");
    let next = tokio::spawn({
        let client = client.clone();
        async move { client.request("next", None, Duration::from_secs(1)).await }
    });
    let sent = request(adapter.recv().await.unwrap());
    assert_eq!(sent.command, "next");
    adapter.respond_ok(&sent, None).await.unwrap();
    assert!(next.await.unwrap().is_ok());
}

#[tokio::test]
async fn timeout_is_a_hard_deadline_while_writer_is_backpressured() {
    let (client, _adapter) = FakeAdapter::pair(16);
    client.set_supports_cancel_request(true);
    let started = tokio::time::Instant::now();
    let result = client
        .request(
            "blocked",
            Some(json!({"payload": "x".repeat(64 * 1024)})),
            Duration::from_millis(25),
        )
        .await;
    assert!(matches!(result, Err(DapError::RequestTimeout { .. })));
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[tokio::test]
async fn event_retention_is_byte_bounded_and_survives_oversized_events() {
    assert_eq!(
        EVENT_CHANNEL_CAPACITY * MAX_RETAINED_EVENT_SIZE,
        MAX_RETAINED_EVENT_BYTES
    );
    let (client, mut adapter) = FakeAdapter::pair(MAX_RETAINED_EVENT_SIZE * 2);
    let mut events = client.subscribe_events();
    adapter
        .event(
            "oversized",
            Some(json!({"payload": "x".repeat(MAX_RETAINED_EVENT_SIZE)})),
        )
        .await
        .unwrap();
    adapter.event("small", None).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.event, "small");
}

#[tokio::test]
async fn event_flooding_remains_bounded_and_reports_lag() {
    let (client, mut adapter) = FakeAdapter::pair(64 * 1024);
    let mut events = client.subscribe_events();
    for index in 0..=EVENT_CHANNEL_CAPACITY {
        adapter
            .event("output", Some(json!({"index": index})))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(matches!(
        events.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(1))
    ));
}

#[tokio::test]
async fn explicit_close_fails_pending_and_closes_transport() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let pending = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .request("threads", None, Duration::from_secs(5))
                .await
        }
    });
    let _sent = adapter.recv().await.unwrap();
    client.close();
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        DapError::TransportClosed
    );
    assert_eq!(adapter.recv().await.unwrap_err(), DapError::TransportClosed);
    assert_eq!(
        client
            .request("again", None, Duration::from_secs(1))
            .await
            .unwrap_err(),
        DapError::TransportClosed
    );
}

#[tokio::test]
async fn dropping_last_client_closes_transport() {
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let clone = client.clone();
    drop(client);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
    drop(clone);
    assert_eq!(adapter.recv().await.unwrap_err(), DapError::TransportClosed);
}
