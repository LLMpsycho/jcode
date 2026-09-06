use super::*;
use jcode_base::protocol as daemon;
use jcode_base::provider::{ModelRoute, RouteSelection, RuntimeKey};
use jcode_harness_api::{AdvisorModelOptions, AdvisorRequest, ApiRequest, ClientFrame};

fn attached() -> BridgeState {
    BridgeState {
        session_id: Some("s1".into()),
        ..Default::default()
    }
}

fn selection() -> RouteSelection {
    RouteSelection {
        model: "gpt-5".into(),
        runtime_key: RuntimeKey::OpenAIOAuth,
        api_method: "OpenAI OAuth".into(),
        provider_label: "OpenAI".into(),
        detail: String::new(),
    }
}

fn control(id: u64, action: Value) -> Value {
    json!({"req": "advisor", "id": id, "session_id": "s1", "request": action})
}

fn forwarded(out: Vec<Outbound>) -> Value {
    match out.into_iter().next().expect("one outbound") {
        Outbound::Legacy(value) => value,
        other => panic!("expected legacy request, got {other:?}"),
    }
}

#[test]
fn advisor_wire_controls_match_the_production_protocol() {
    for request in [
        daemon::AdvisorRequest::Status,
        daemon::AdvisorRequest::Inspect,
        daemon::AdvisorRequest::Enable,
        daemon::AdvisorRequest::Disable,
        daemon::AdvisorRequest::Acknowledge {
            note_id: "adv-1".into(),
        },
        daemon::AdvisorRequest::Dismiss {
            note_id: "adv-2".into(),
        },
        daemon::AdvisorRequest::ModelOptions { selection: None },
        daemon::AdvisorRequest::ModelOptions {
            selection: Some(selection()),
        },
        daemon::AdvisorRequest::SelectModel {
            selection: selection(),
            reasoning_effort: Some("high".into()),
        },
        daemon::AdvisorRequest::UsePrimary,
    ]
    .into_iter()
    .flat_map(|request| {
        [
            request.clone(),
            daemon::AdvisorRequest::ForAdvisor {
                name: "security".into(),
                request: Box::new(request),
            },
        ]
    }) {
        let wire = serde_json::to_value(&request).unwrap();
        let public: AdvisorRequest = serde_json::from_value(wire.clone()).unwrap();
        let frame = ClientFrame::new(
            7,
            ApiRequest::Advisor {
                session_id: "s1".into(),
                request: public,
            },
        );
        let mut state = attached();
        let legacy = forwarded(state.api_request_to_legacy(&serde_json::to_value(frame).unwrap()));
        assert_eq!(legacy["request"], wire);
        let decoded: daemon::Request = serde_json::from_value(legacy.clone()).unwrap();
        assert!(matches!(decoded, daemon::Request::Advisor { request: got, .. } if got == request));
        let id = legacy["id"].as_u64().unwrap();
        assert!(
            state
                .legacy_event_to_api(&json!({"type": "ack", "id": id}))
                .is_empty()
        );
        assert!(
            state
                .legacy_event_to_api(&json!({"type": "done", "id": id}))
                .is_empty()
        );
        let result = state.legacy_event_to_api(&json!({
            "type": "advisor_result", "id": id, "result": {"message": "Advisor: on"},
        }));
        assert_eq!(result[0].reply_to, Some(7));
        assert!(
            matches!(&result[0].event, ApiEvent::AdvisorResult { result, .. } if result.error.is_none())
        );
    }
}

#[test]
fn advisor_runtime_tags_match_every_serializable_production_identity() {
    for key in [
        RuntimeKey::JcodeSubscription,
        RuntimeKey::ClaudeOAuth,
        RuntimeKey::AnthropicApiKey,
        RuntimeKey::OpenAIOAuth,
        RuntimeKey::OpenAIApiKey,
        RuntimeKey::OpenRouter,
        RuntimeKey::OpenAiCompatible { profile_id: None },
        RuntimeKey::OpenAiCompatible {
            profile_id: Some("work-profile".into()),
        },
        RuntimeKey::Copilot,
        RuntimeKey::Gemini,
        RuntimeKey::Cursor,
        RuntimeKey::Bedrock,
        RuntimeKey::Antigravity,
        RuntimeKey::CodeAssistOAuth,
        RuntimeKey::RemoteCatalog,
        RuntimeKey::Current,
    ] {
        let key_json = serde_json::to_value(&key).unwrap();
        assert!(
            known_runtime_kind(key_json["kind"].as_str().unwrap()),
            "unrecognized {key:?}"
        );
        let mut route = selection();
        route.runtime_key = key;
        let action = serde_json::to_value(daemon::AdvisorRequest::SelectModel {
            selection: route,
            reasoning_effort: None,
        })
        .unwrap();
        let legacy = forwarded(attached().api_request_to_legacy(&control(9, action)));
        assert_eq!(legacy["request"]["selection"]["runtime_key"], key_json);
        assert!(serde_json::from_value::<daemon::Request>(legacy).is_ok());
    }
}

#[test]
fn advisor_model_options_preserve_canonical_routes_and_legacy_defaults() {
    let mut state = attached();
    let legacy =
        forwarded(state.api_request_to_legacy(&control(3, json!({"action": "model_options"}))));
    let event = daemon::ServerEvent::AdvisorResult {
        id: legacy["id"].as_u64().unwrap(),
        result: daemon::AdvisorControlResult {
            message: "Choose a model".into(),
            model_settings: Some(daemon::AdvisorModelSettings {
                enabled: true,
                selection: Some(selection()),
                reasoning_effort: Some("high".into()),
                follows_primary: false,
            }),
            model_options: Some(daemon::AdvisorModelOptions {
                selection: Some(selection()),
                reasoning_effort: Some("high".into()),
                available_routes: vec![ModelRoute {
                    model: "gpt-5".into(),
                    provider: "OpenAI".into(),
                    api_method: "OpenAI OAuth".into(),
                    available: true,
                    detail: "subscription".into(),
                    cheapness: None,
                }],
                available_selections: vec![selection()],
                available_efforts: vec!["low".into(), "high".into()],
            }),
            error: None,
        },
    };
    let replies = state.legacy_event_to_api(&serde_json::to_value(event).unwrap());
    let ApiEvent::AdvisorResult { result, session_id } = &replies[0].event else {
        panic!("expected advisor result");
    };
    assert_eq!(session_id, "s1");
    let options = result.model_options.as_ref().unwrap();
    assert_eq!(options.available_routes[0].api_method, "OpenAI OAuth");
    assert_eq!(options.available_efforts, ["low", "high"]);
    assert_eq!(
        serde_json::to_value(&options.available_selections[0]).unwrap(),
        serde_json::to_value(selection()).unwrap()
    );
    let old: AdvisorModelOptions = serde_json::from_value(json!({
        "selection": null, "reasoning_effort": null, "available_routes": [], "available_efforts": [],
    })).unwrap();
    assert!(old.available_selections.is_empty());
    assert!(
        serde_json::to_value(old)
            .unwrap()
            .get("available_selections")
            .is_none()
    );
}

#[test]
fn advisor_control_replies_do_not_complete_or_consume_a_primary_turn() {
    let mut state = attached();
    let turn = forwarded(state.api_request_to_legacy(&json!({
        "req": "send_message", "id": 1, "session_id": "s1", "content": "hello",
    })));
    let first = forwarded(state.api_request_to_legacy(&control(2, json!({"action": "status"}))));
    let second = forwarded(state.api_request_to_legacy(&control(3, json!({"action": "disable"}))));
    let wrong =
        json!({"type": "advisor_result", "id": turn["id"], "result": {"message": "wrong id"}});
    assert!(state.legacy_event_to_api(&wrong).is_empty());
    let delta = state.legacy_event_to_api(&json!({"type": "text_delta", "text": "hello"}));
    assert!(matches!(delta[0].event, ApiEvent::TextDelta { .. }));
    for (request, api_id) in [(second, 3), (first, 2)] {
        let id = &request["id"];
        assert!(
            state
                .legacy_event_to_api(&json!({"type": "done", "id": id}))
                .is_empty()
        );
        let event = json!({"type": "advisor_result", "id": id, "result": {"message": "ok"}});
        let replies = state.legacy_event_to_api(&event);
        assert_eq!(replies[0].reply_to, Some(api_id));
        assert!(matches!(replies[0].event, ApiEvent::AdvisorResult { .. }));
        assert!(state.legacy_event_to_api(&event).is_empty());
    }
    let done = state.legacy_event_to_api(&json!({"type": "done", "id": turn["id"]}));
    assert!(matches!(done[0].event, ApiEvent::TurnDone { .. }));
}

#[test]
fn advisor_controls_reject_wrong_sessions_and_malformed_requests_locally() {
    let valid = control(4, json!({"action": "status"}));
    let mut wrong = valid.clone();
    wrong["session_id"] = json!("someone-else");
    let mut absent = valid.clone();
    absent.as_object_mut().unwrap().remove("session_id");
    let mut bad_runtime = serde_json::to_value(selection()).unwrap();
    bad_runtime["runtime_key"]["kind"] = json!("not-a-runtime");
    for (mut state, request) in [
        (BridgeState::default(), valid),
        (attached(), wrong),
        (attached(), absent),
        (attached(), control(4, json!({"action": "dismiss"}))),
        (attached(), control(4, json!({"action": "anything"}))),
        (
            attached(),
            control(
                4,
                json!({
                    "action": "for_advisor", "name": "security",
                    "request": {"action": "select_model", "selection": bad_runtime},
                }),
            ),
        ),
        (
            attached(),
            control(
                4,
                json!({
                    "action": "for_advisor", "name": "security",
                    "request": {
                        "action": "for_advisor", "name": "performance",
                        "request": {"action": "status"},
                    },
                }),
            ),
        ),
        (
            attached(),
            control(
                4,
                json!({"action": "select_model", "selection": bad_runtime}),
            ),
        ),
    ] {
        let replies = state.api_request_to_legacy(&request);
        assert!(matches!(
            &replies[0],
            Outbound::Reply(ServerFrame {
                reply_to: Some(4),
                event: ApiEvent::Error { .. },
                ..
            })
        ));
        assert!(state.pending_simple.is_empty());
    }
}

#[test]
fn advisor_replies_keep_the_originating_session_and_report_durability_errors() {
    let mut state = attached();
    let request = forwarded(state.api_request_to_legacy(&control(4, json!({"action": "enable"}))));
    state.session_id = Some("s2".into());
    let replies = state.legacy_event_to_api(&json!({
        "type": "advisor_result", "id": request["id"],
        "result": {"message": "state was not saved", "error": "checkpoint write failed"},
    }));
    assert!(
        matches!(&replies[0].event, ApiEvent::AdvisorResult { session_id, result }
        if session_id == "s1" && result.error.as_deref() == Some("checkpoint write failed"))
    );
}

#[test]
fn advisor_malformed_results_fail_the_waiter_without_later_duplicate_replies() {
    let mut state = attached();
    let request = forwarded(state.api_request_to_legacy(&control(4, json!({"action": "enable"}))));
    let malformed =
        json!({"type": "advisor_result", "id": request["id"], "result": {"message": 12}});
    let replies = state.legacy_event_to_api(&malformed);
    assert_eq!(replies[0].reply_to, Some(4));
    assert!(matches!(
        replies[0].event,
        ApiEvent::Error {
            code: ErrorCode::Internal,
            ..
        }
    ));
    assert!(state.legacy_event_to_api(&malformed).is_empty());
}
