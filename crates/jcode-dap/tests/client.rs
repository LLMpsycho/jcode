use std::time::Duration;

use jcode_dap::testing::FakeAdapter;
use jcode_dap::{DapError, Message, Response};
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
    assert_ne!(one.seq, two.seq);
    let (first_request, second_request) = if one.command == "first" {
        (one, two)
    } else {
        (two, one)
    };
    assert!(first_request.seq < second_request.seq);
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
