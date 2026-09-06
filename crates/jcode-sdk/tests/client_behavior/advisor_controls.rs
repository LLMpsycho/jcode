use super::*;
use jcode_sdk::{AdvisorControlResult, AdvisorRequest, AdvisorRouteSelection};
use serde_json::json;

fn selection() -> AdvisorRouteSelection {
    serde_json::from_value(json!({
        "model": "gpt-5", "runtime_key": {"kind": "open-a-i-o-auth"},
        "api_method": "openai-oauth", "provider_label": "OpenAI",
    }))
    .unwrap()
}

#[test]
fn advisor_forwards_all_controls_with_canonical_route_and_effort() {
    let commands = vec![
        AdvisorRequest::Status,
        AdvisorRequest::Inspect,
        AdvisorRequest::Enable,
        AdvisorRequest::Disable,
        AdvisorRequest::Acknowledge {
            note_id: "adv-1".into(),
        },
        AdvisorRequest::Dismiss {
            note_id: "adv-2".into(),
        },
        AdvisorRequest::ModelOptions { selection: None },
        AdvisorRequest::ModelOptions {
            selection: Some(selection()),
        },
        AdvisorRequest::SelectModel {
            selection: selection(),
            reasoning_effort: Some("high".into()),
        },
        AdvisorRequest::UsePrimary,
    ];
    let (sent, received) = channel();
    let expected: AdvisorControlResult = serde_json::from_value(json!({
        "message": "Choose an advisor model",
        "model_settings": {
            "enabled": true, "selection": selection(), "reasoning_effort": "high", "follows_primary": false,
        },
        "model_options": {
            "selection": selection(), "reasoning_effort": "high", "available_routes": [],
            "available_selections": [selection()], "available_efforts": ["low", "high"],
        },
    })).unwrap();
    let result = expected.clone();
    let client = fake_harness(move |frame, writer| {
        sent.send(frame.request.clone()).unwrap();
        reply(
            frame,
            ApiEvent::AdvisorResult {
                session_id: "s1".into(),
                result: Box::new(result.clone()),
            },
            writer,
        );
    });
    for request in commands.into_iter().flat_map(|request| {
        [
            request.clone(),
            AdvisorRequest::ForAdvisor {
                name: "security".into(),
                request: Box::new(request),
            },
        ]
    }) {
        assert_eq!(client.advisor("s1", request.clone()).unwrap(), expected);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            ApiRequest::Advisor {
                session_id: "s1".into(),
                request,
            }
        );
    }
}

#[test]
fn advisor_correlated_reply_does_not_unblock_the_primary_turn() {
    let (started, observed) = channel();
    let client = fake_harness(move |frame, writer| match &frame.request {
        ApiRequest::SendMessage { .. } => {
            push(
                ApiEvent::MessageAccepted {
                    session_id: "s1".into(),
                },
                writer,
            );
            push(
                ApiEvent::TextDelta {
                    session_id: "s1".into(),
                    text: "main ".into(),
                },
                writer,
            );
            started.send(()).unwrap();
        }
        ApiRequest::Advisor { .. } => {
            push(
                ApiEvent::TextDelta {
                    session_id: "s1".into(),
                    text: "response".into(),
                },
                writer,
            );
            reply(
                frame,
                ApiEvent::AdvisorResult {
                    session_id: "s1".into(),
                    result: Box::new(AdvisorControlResult {
                        message: "Advisor: on".into(),
                        ..Default::default()
                    }),
                },
                writer,
            );
        }
        ApiRequest::Ping => {
            push(
                ApiEvent::TurnDone {
                    session_id: "s1".into(),
                },
                writer,
            );
            reply(frame, ApiEvent::Pong, writer);
        }
        other => panic!("unexpected request: {other:?}"),
    });
    let primary = client.clone();
    let (finished, result) = channel();
    let runner = std::thread::spawn(move || {
        finished
            .send(primary.run("s1", "hello", Default::default()))
            .unwrap();
    });
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        client
            .advisor("s1", AdvisorRequest::Status)
            .unwrap()
            .message,
        "Advisor: on"
    );
    assert!(matches!(
        result.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    client.ping().unwrap();
    assert_eq!(
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .text,
        "main response"
    );
    runner.join().unwrap();
}

#[test]
fn advisor_preserves_control_failures_and_rejects_transport_reply_errors() {
    let client = fake_harness(|frame, writer| {
        let event = match &frame.request {
            ApiRequest::Advisor {
                request: AdvisorRequest::Disable,
                ..
            } => ApiEvent::AdvisorResult {
                session_id: "s1".into(),
                result: Box::new(AdvisorControlResult {
                    message: "Could not save advisor state".into(),
                    error: Some("checkpoint write failed".into()),
                    ..Default::default()
                }),
            },
            ApiRequest::Advisor {
                request: AdvisorRequest::Status,
                ..
            } => ApiEvent::Error {
                code: jcode_harness_api::ErrorCode::UnknownSession,
                message: "not attached".into(),
            },
            _ => ApiEvent::Ok,
        };
        reply(frame, event, writer);
    });
    assert_eq!(
        client
            .advisor("s1", AdvisorRequest::Disable)
            .unwrap()
            .error
            .as_deref(),
        Some("checkpoint write failed")
    );
    assert_eq!(
        client
            .advisor("s1", AdvisorRequest::Status)
            .unwrap_err()
            .code(),
        "unknown_session"
    );
    assert_eq!(
        client
            .advisor("s1", AdvisorRequest::Inspect)
            .unwrap_err()
            .code(),
        "unexpected_reply"
    );
}
