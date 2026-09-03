//! Stable, runtime-independent contracts used by Jcode's LSP integration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSON_RPC_VERSION: &str = "2.0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostEditDiagnosticsMode {
    Off,
    #[default]
    Delta,
    File,
    Workspace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub root_markers: Vec<String>,
    pub file_extensions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LspConfig {
    pub enabled: bool,
    pub shared: bool,
    pub idle_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub post_edit_diagnostics: PostEditDiagnosticsMode,
    pub post_edit_wait_ms: u64,
    pub max_output_tokens: usize,
    pub servers: BTreeMap<String, LspServerConfig>,
}

impl Default for LspConfig {
    fn default() -> Self {
        let mut servers = BTreeMap::new();
        servers.insert(
            "rust-analyzer".to_owned(),
            LspServerConfig {
                command: "rust-analyzer".to_owned(),
                args: Vec::new(),
                root_markers: vec!["Cargo.toml".to_owned(), "rust-project.json".to_owned()],
                file_extensions: vec!["rs".to_owned()],
            },
        );
        Self {
            enabled: true,
            shared: true,
            idle_timeout_seconds: 300,
            request_timeout_seconds: 20,
            post_edit_diagnostics: PostEditDiagnosticsMode::Delta,
            post_edit_wait_ms: 750,
            max_output_tokens: 2500,
            servers,
        }
    }
}

impl LspConfig {
    pub fn validation_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.idle_timeout_seconds == 0 {
            issues.push("lsp.idle_timeout_seconds must be greater than zero".to_owned());
        }
        if self.request_timeout_seconds == 0 {
            issues.push("lsp.request_timeout_seconds must be greater than zero".to_owned());
        }
        if self.max_output_tokens == 0 {
            issues.push("lsp.max_output_tokens must be greater than zero".to_owned());
        }
        for (server_id, server) in &self.servers {
            if server_id.trim().is_empty() {
                issues.push("lsp.servers contains an empty server id".to_owned());
            }
            if server.command.trim().is_empty() {
                issues.push(format!("lsp.servers.{server_id}.command must not be empty"));
            }
            if server.file_extensions.iter().any(|extension| {
                extension.is_empty() || extension.contains('/') || extension.contains('\\')
            }) {
                issues.push(format!(
                    "lsp.servers.{server_id}.file_extensions must contain bare extensions"
                ));
            }
        }
        issues
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestMessage {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RequestMessage {
    pub fn new(id: RequestId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationMessage {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl NotificationMessage {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticSeverity(pub u32);

impl DiagnosticSeverity {
    pub const ERROR: Self = Self(1);
    pub const WARNING: Self = Self(2);
    pub const INFORMATION: Self = Self(3);
    pub const HINT: Self = Self(4);
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<DiagnosticSeverity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishDiagnosticsParams {
    pub uri: String,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticVerificationStatus {
    Unavailable,
    Stale,
    Clean,
    IssuesFound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticVerification {
    pub status: SemanticVerificationStatus,
    pub document_version: Option<i64>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub range: Range,
    #[serde(rename = "newText")]
    pub new_text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEdit {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub changes: BTreeMap<String, Vec<TextEdit>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_rpc_ids_and_messages_have_stable_wire_shapes() {
        let request = RequestMessage::new(
            RequestId::Number(7),
            "textDocument/definition",
            Some(json!({"position": {"line": 4, "character": 2}})),
        );
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "textDocument/definition",
                "params": {"position": {"line": 4, "character": 2}}
            })
        );

        let notification = NotificationMessage::new("initialized", None);
        assert_eq!(
            serde_json::to_value(notification).unwrap(),
            json!({"jsonrpc": "2.0", "method": "initialized"})
        );
    }

    #[test]
    fn diagnostic_and_workspace_edit_round_trip() {
        let value = json!({
            "range": {
                "start": {"line": 1, "character": 2},
                "end": {"line": 1, "character": 5}
            },
            "severity": 1,
            "code": "E0308",
            "source": "rustc",
            "message": "mismatched types"
        });
        let diagnostic: Diagnostic = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(serde_json::to_value(diagnostic).unwrap(), value);

        let edit: WorkspaceEdit = serde_json::from_value(json!({
            "changes": {
                "file:///workspace/src/lib.rs": [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 3}
                    },
                    "newText": "new"
                }]
            }
        }))
        .unwrap();
        assert_eq!(edit.changes.len(), 1);
    }

    #[test]
    fn lsp_defaults_match_the_documented_rust_mvp() {
        let config = LspConfig::default();
        assert!(config.enabled);
        assert!(config.shared);
        assert_eq!(config.request_timeout_seconds, 20);
        let rust = &config.servers["rust-analyzer"];
        assert_eq!(rust.command, "rust-analyzer");
        assert_eq!(rust.file_extensions, ["rs"]);
        assert!(config.validation_issues().is_empty());
    }

    #[test]
    fn lsp_config_rejects_unknown_keys_and_reports_semantic_issues() {
        let unknown = r#"{"enabled":true,"surprise":1}"#;
        assert!(serde_json::from_str::<LspConfig>(unknown).is_err());

        let config = LspConfig {
            request_timeout_seconds: 0,
            max_output_tokens: 0,
            servers: BTreeMap::from([(
                "rust-analyzer".to_owned(),
                LspServerConfig {
                    command: String::new(),
                    file_extensions: vec!["src/rs".to_owned()],
                    ..LspServerConfig::default()
                },
            )]),
            ..LspConfig::default()
        };
        let issues = config.validation_issues();
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("request_timeout_seconds"))
        );
        assert!(issues.iter().any(|issue| issue.contains("command")));
        assert!(issues.iter().any(|issue| issue.contains("file_extensions")));
    }
}
