//! Stable wire types for the Debug Adapter Protocol.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MIN_OUTPUT_BYTES: usize = 1_024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1_024 * 1_024;
pub const MIN_OPAQUE_HANDLES_PER_OWNER: usize = 1;
pub const MAX_OPAQUE_HANDLES_PER_OWNER: usize = 65_536;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DapAdapterKind {
    #[default]
    LldbDap,
    GdbDap,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DapAdapterConfig {
    pub kind: DapAdapterKind,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DapConfig {
    pub enabled: bool,
    pub allow_evaluate: bool,
    pub max_output_bytes: usize,
    pub max_opaque_handles_per_owner: usize,
    pub adapters: BTreeMap<String, DapAdapterConfig>,
}

impl Default for DapConfig {
    fn default() -> Self {
        let adapters = BTreeMap::from([(
            "lldb-dap".to_owned(),
            DapAdapterConfig {
                kind: DapAdapterKind::LldbDap,
                command: "lldb-dap".to_owned(),
            },
        )]);
        Self {
            enabled: false,
            allow_evaluate: false,
            max_output_bytes: 1024 * 1024,
            max_opaque_handles_per_owner: 8_192,
            adapters,
        }
    }
}

impl DapConfig {
    pub fn validation_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(MIN_OUTPUT_BYTES..=MAX_OUTPUT_BYTES).contains(&self.max_output_bytes) {
            issues.push(format!(
                "dap.max_output_bytes must be between {MIN_OUTPUT_BYTES} and {MAX_OUTPUT_BYTES}"
            ));
        }
        if !(MIN_OPAQUE_HANDLES_PER_OWNER..=MAX_OPAQUE_HANDLES_PER_OWNER)
            .contains(&self.max_opaque_handles_per_owner)
        {
            issues.push(format!(
                "dap.max_opaque_handles_per_owner must be between {MIN_OPAQUE_HANDLES_PER_OWNER} and {MAX_OPAQUE_HANDLES_PER_OWNER}"
            ));
        }
        for (adapter_id, adapter) in &self.adapters {
            if adapter_id.trim().is_empty() {
                issues.push("dap.adapters contains an empty adapter id".to_owned());
            }
            if adapter.command.trim().is_empty() {
                issues.push(format!(
                    "dap.adapters.{adapter_id}.command must not be empty"
                ));
            }
        }
        issues
    }
}

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
    #[serde(rename = "clientID", skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(rename = "adapterID")]
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
    fn dap_config_defaults_are_opt_in_and_use_local_lldb_dap() {
        let config = DapConfig::default();
        assert!(!config.enabled);
        assert!(!config.allow_evaluate);
        assert_eq!(config.adapters["lldb-dap"].kind, DapAdapterKind::LldbDap);
        assert_eq!(config.adapters["lldb-dap"].command, "lldb-dap");
        assert!(config.validation_issues().is_empty());
        assert_eq!(
            serde_json::from_value::<DapAdapterKind>(json!("gdb-dap")).unwrap(),
            DapAdapterKind::GdbDap
        );
    }

    #[test]
    fn dap_config_is_strict_and_reports_bounded_limits() {
        let unknown = serde_json::from_value::<DapConfig>(json!({"downloadAdapters": true}))
            .expect_err("unknown DAP config keys must fail")
            .to_string();
        assert!(unknown.contains("downloadAdapters"));

        let unsupported_kind = serde_json::from_value::<DapAdapterConfig>(
            json!({"kind":"custom","command":"custom-dap"}),
        )
        .expect_err("unsupported adapter kinds must fail")
        .to_string();
        assert!(unsupported_kind.contains("custom"));

        let config = DapConfig {
            max_output_bytes: MAX_OUTPUT_BYTES + 1,
            max_opaque_handles_per_owner: MIN_OPAQUE_HANDLES_PER_OWNER - 1,
            ..DapConfig::default()
        };
        let issues = config.validation_issues();
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("max_output_bytes"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("max_opaque_handles_per_owner"))
        );
    }

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
        assert_eq!(value["adapterID"], "lldb");
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

#[cfg(test)]
mod initialize_wire_contract_tests {
    use super::*;

    fn initialize() -> InitializeRequestArguments {
        InitializeRequestArguments {
            client_id: Some("jcode".to_owned()),
            client_name: Some("Jcode".to_owned()),
            adapter_id: "lldb".to_owned(),
            locale: None,
            lines_start_at1: Some(true),
            columns_start_at1: Some(true),
            path_format: Some("path".to_owned()),
            supports_variable_type: Some(true),
            supports_variable_paging: Some(true),
            supports_run_in_terminal_request: Some(false),
        }
    }

    #[test]
    fn initialize_request_serialization_preserves_exact_wire_key_casing() {
        let value = serde_json::to_value(initialize()).unwrap();
        assert_eq!(value["clientID"], "jcode");
        assert_eq!(value["adapterID"], "lldb");
        assert!(value.get("clientId").is_none());
        assert!(value.get("adapterId").is_none());
    }

    #[test]
    fn initialize_request_deserialization_accepts_exact_wire_key_casing() {
        let decoded: InitializeRequestArguments = serde_json::from_value(serde_json::json!({
            "clientID": "jcode",
            "clientName": "Jcode",
            "adapterID": "lldb",
            "supportsVariableType": true,
            "supportsVariablePaging": true
        }))
        .unwrap();
        assert_eq!(decoded.client_id.as_deref(), Some("jcode"));
        assert_eq!(decoded.adapter_id, "lldb");
        assert_eq!(decoded.supports_variable_type, Some(true));
        assert_eq!(decoded.supports_variable_paging, Some(true));
    }
}
