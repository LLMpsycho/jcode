//! Stable wire types for the Debug Adapter Protocol.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub seq: i64,
    #[serde(rename = "type")]
    pub message_type: RequestType,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

impl Request {
    pub fn new(seq: i64, command: impl Into<String>, arguments: Option<Value>) -> Self {
        Self {
            seq,
            message_type: RequestType::Request,
            command: command.into(),
            arguments,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestType {
    Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub seq: i64,
    #[serde(rename = "type")]
    pub message_type: ResponseType,
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Response {
    pub fn success(
        seq: i64,
        request_seq: i64,
        command: impl Into<String>,
        body: Option<Value>,
    ) -> Self {
        Self {
            seq,
            message_type: ResponseType::Response,
            request_seq,
            success: true,
            command: command.into(),
            message: None,
            body,
        }
    }

    pub fn error(
        seq: i64,
        request_seq: i64,
        command: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            message_type: ResponseType::Response,
            request_seq,
            success: false,
            command: command.into(),
            message: Some(message.into()),
            body: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseType {
    Response,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    #[serde(rename = "type")]
    pub message_type: EventType,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Event {
    pub fn new(seq: i64, event: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            seq,
            message_type: EventType::Event,
            event: event.into(),
            body,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Event,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequestArguments {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    pub adapter_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_start_at1: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns_start_at1: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_variable_type: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_variable_paging: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_run_in_terminal_request: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_cancel_request: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_terminate_request: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_configuration_done_request: Option<bool>,
    #[serde(flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInTerminalRequestArguments {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<RunInTerminalKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub cwd: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_can_be_interpreted_by_shell: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunInTerminalKind {
    Integrated,
    External,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_dtos_have_stable_wire_json() {
        assert_eq!(
            serde_json::to_value(Request::new(1, "threads", None)).unwrap(),
            json!({"seq":1,"type":"request","command":"threads"})
        );
        assert_eq!(
            serde_json::to_value(Response::error(2, 1, "threads", "nope")).unwrap(),
            json!({"seq":2,"type":"response","request_seq":1,"success":false,"command":"threads","message":"nope"})
        );
        assert_eq!(
            serde_json::to_value(Event::new(
                3,
                "stopped",
                Some(json!({"reason":"breakpoint"}))
            ))
            .unwrap(),
            json!({"seq":3,"type":"event","event":"stopped","body":{"reason":"breakpoint"}})
        );
    }

    #[test]
    fn initialize_and_run_in_terminal_shapes_are_camel_case() {
        let init = InitializeRequestArguments {
            adapter_id: "lldb".into(),
            supports_run_in_terminal_request: Some(true),
            ..Default::default()
        };
        let value = serde_json::to_value(init).unwrap();
        assert_eq!(value["adapterId"], "lldb");
        assert_eq!(value["supportsRunInTerminalRequest"], true);
        let terminal: RunInTerminalRequestArguments = serde_json::from_value(json!({"kind":"integrated","cwd":"/work","args":["echo","ok"],"env":{"TOKEN":null},"argsCanBeInterpretedByShell":false})).unwrap();
        assert_eq!(terminal.kind, Some(RunInTerminalKind::Integrated));
    }

    #[test]
    fn capabilities_preserve_unknown_fields() {
        let caps: Capabilities = serde_json::from_value(
            json!({"supportsCancelRequest":true,"futureCapability":{"mode":1}}),
        )
        .unwrap();
        assert_eq!(caps.supports_cancel_request, Some(true));
        assert_eq!(caps.additional["futureCapability"], json!({"mode":1}));
        assert_eq!(
            serde_json::to_value(caps).unwrap()["futureCapability"],
            json!({"mode":1})
        );
    }
}
