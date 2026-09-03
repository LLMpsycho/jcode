use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use jcode_lsp::{
    Diagnostic, DiagnosticSeverity, LspConfig, LspError, LspServicePool, LspWorkspace, Position,
    discover_executable,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Tool, ToolContext, ToolOutput};

pub struct LspTool {
    pool: Arc<LspServicePool>,
    config_override: Option<LspConfig>,
}

impl LspTool {
    pub(crate) fn new(pool: Arc<LspServicePool>) -> Self {
        Self {
            pool,
            config_override: None,
        }
    }

    #[cfg(test)]
    fn with_config(pool: Arc<LspServicePool>, config: LspConfig) -> Self {
        Self {
            pool,
            config_override: Some(config),
        }
    }

    fn config(&self) -> LspConfig {
        self.config_override
            .clone()
            .unwrap_or_else(|| crate::config::config().lsp.clone())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LspAction {
    Status,
    Diagnostics,
    Hover,
    Definition,
    References,
    DocumentSymbols,
    WorkspaceSymbols,
    Capabilities,
}

impl LspAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Diagnostics => "diagnostics",
            Self::Hover => "hover",
            Self::Definition => "definition",
            Self::References => "references",
            Self::DocumentSymbols => "document_symbols",
            Self::WorkspaceSymbols => "workspace_symbols",
            Self::Capabilities => "capabilities",
        }
    }
}

#[derive(Deserialize)]
struct LspInput {
    action: LspAction,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    query: Option<String>,
    intent: String,
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Query shared language-server status, diagnostics, symbols, hover, definitions, and references."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action", "intent"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "status", "diagnostics", "hover", "definition", "references",
                        "document_symbols", "workspace_symbols", "capabilities"
                    ]
                },
                "file": {"type": ["string", "null"], "description": "Workspace-relative source file."},
                "line": {"type": ["integer", "null"], "minimum": 1, "description": "One-based source line."},
                "character": {"type": ["integer", "null"], "minimum": 1, "description": "One-based UTF-16 character offset."},
                "query": {"type": ["string", "null"], "description": "Workspace-symbol query."},
                "intent": super::intent_schema_property()
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: LspInput = serde_json::from_value(input)?;
        let _ = &params.intent;
        let config = self.config();
        let root = workspace_root(&ctx)?;

        if matches!(params.action, LspAction::Status) {
            return Ok(render_status(&config, &root));
        }
        if !config.enabled {
            return Ok(unavailable_output(
                params.action,
                &root,
                None,
                "LSP is disabled by configuration",
            ));
        }

        let file = params
            .file
            .as_deref()
            .map(|file| resolve_file(&root, file))
            .transpose()?;
        let server_id = select_server(&config, file.as_deref())?;
        let workspace = match self
            .pool
            .get_or_start(&root, root.display().to_string(), &server_id, &config)
            .await
        {
            Ok(workspace) => workspace,
            Err(error @ (LspError::ExecutableNotFound { .. } | LspError::NotExecutable { .. })) => {
                return Ok(unavailable_output(
                    params.action,
                    &root,
                    Some(&server_id),
                    &error.to_string(),
                ));
            }
            Err(error) => return Err(error.into()),
        };

        let max_chars = config.max_output_tokens.saturating_mul(4).max(256);
        match params.action {
            LspAction::Status => unreachable!(),
            LspAction::Capabilities => {
                let keys = workspace
                    .capabilities()
                    .get("capabilities")
                    .and_then(Value::as_object)
                    .map(|capabilities| capabilities.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                let text = if keys.is_empty() {
                    "No capabilities reported.".to_owned()
                } else {
                    format!("Capabilities: {}", keys.join(", "))
                };
                Ok(shaped_output(
                    params.action,
                    &workspace,
                    None,
                    text,
                    json!(keys),
                    max_chars,
                    "fresh",
                ))
            }
            LspAction::WorkspaceSymbols => {
                let query = params.query.as_deref().unwrap_or_default();
                let value = workspace.workspace_symbols(query).await?;
                let (text, count) = render_symbols(&value, &root);
                Ok(shaped_output(
                    params.action,
                    &workspace,
                    None,
                    text,
                    json!({"count": count}),
                    max_chars,
                    "fresh",
                ))
            }
            action => {
                let file = file
                    .as_deref()
                    .ok_or_else(|| anyhow!("lsp action `{}` requires `file`", action.as_str()))?;
                let language_id = language_id(file);
                let document = workspace.sync_document_from_disk(file, language_id).await?;
                match action {
                    LspAction::Diagnostics => {
                        let snapshot = workspace
                            .wait_for_diagnostics(
                                &document,
                                Duration::from_millis(config.post_edit_wait_ms),
                            )
                            .await
                            .or(workspace.diagnostics(file).await?);
                        let freshness = if snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.version)
                            .is_some_and(|version| version >= document.version)
                        {
                            "fresh"
                        } else {
                            "stale"
                        };
                        let items = snapshot
                            .as_ref()
                            .map(|snapshot| snapshot.items.as_slice())
                            .unwrap_or_default();
                        let text = render_diagnostics(items, &root, file);
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            text,
                            json!({"count": items.len()}),
                            max_chars,
                            freshness,
                        ))
                    }
                    LspAction::Hover | LspAction::Definition | LspAction::References => {
                        let position = one_based_position(params.line, params.character)?;
                        let value = match action {
                            LspAction::Hover => workspace.hover(file, position).await?,
                            LspAction::Definition => workspace.definition(file, position).await?,
                            LspAction::References => workspace.references(file, position).await?,
                            _ => unreachable!(),
                        };
                        let (text, count) = match action {
                            LspAction::Hover => {
                                (render_hover(&value), usize::from(!value.is_null()))
                            }
                            _ => render_locations(&value, &root),
                        };
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            text,
                            json!({"count": count}),
                            max_chars,
                            "fresh",
                        ))
                    }
                    LspAction::DocumentSymbols => {
                        let value = workspace.document_symbols(file).await?;
                        let (text, count) = render_symbols(&value, &root);
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            text,
                            json!({"count": count}),
                            max_chars,
                            "fresh",
                        ))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

fn workspace_root(ctx: &ToolContext) -> Result<PathBuf> {
    ctx.working_dir
        .as_deref()
        .ok_or_else(|| anyhow!("lsp requires a session working directory"))?
        .canonicalize()
        .map_err(Into::into)
}

fn resolve_file(root: &Path, file: &str) -> Result<PathBuf> {
    let path = root.join(file).canonicalize()?;
    if !path.starts_with(root) {
        bail!("LSP file must remain inside the workspace");
    }
    Ok(path)
}

fn select_server(config: &LspConfig, file: Option<&Path>) -> Result<String> {
    if let Some(extension) = file
        .and_then(Path::extension)
        .and_then(|value| value.to_str())
    {
        if let Some((server_id, _)) = config.servers.iter().find(|(_, server)| {
            server
                .file_extensions
                .iter()
                .any(|candidate| candidate == extension)
        }) {
            return Ok(server_id.clone());
        }
        bail!("no LSP server is configured for .{extension} files");
    }
    config
        .servers
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| anyhow!("no LSP servers are configured"))
}

fn language_id(path: &Path) -> &str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") => "javascript",
        Some("jsx") => "javascriptreact",
        Some("py") => "python",
        Some("go") => "go",
        _ => "plaintext",
    }
}

fn one_based_position(line: Option<u32>, character: Option<u32>) -> Result<Position> {
    let line = line.ok_or_else(|| anyhow!("this LSP action requires `line`"))?;
    if line == 0 || character == Some(0) {
        bail!("LSP line and character values are one-based");
    }
    Ok(Position {
        line: line - 1,
        character: character.unwrap_or(1) - 1,
    })
}

fn render_status(config: &LspConfig, root: &Path) -> ToolOutput {
    if !config.enabled {
        return unavailable_output(
            LspAction::Status,
            root,
            None,
            "LSP is disabled by configuration",
        );
    }
    let path = std::env::var_os("PATH");
    let statuses = config
        .servers
        .iter()
        .map(|(id, server)| {
            let discovered = discover_executable(&server.command, path.as_deref(), root);
            let status = if discovered.is_ok() {
                "available"
            } else {
                "missing"
            };
            (id, status, discovered.ok())
        })
        .collect::<Vec<_>>();
    let text = statuses
        .iter()
        .map(|(id, status, path)| {
            format!(
                "{id}: {status}{}",
                path.as_ref()
                    .map(|path| format!(" ({})", path.display()))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ToolOutput::new(text)
        .with_title("lsp status")
        .with_metadata(json!({
            "action": "status",
            "workspace": root,
            "freshness": "fresh",
            "servers": statuses.iter().map(|(id, status, _)| json!({"id": id, "status": status})).collect::<Vec<_>>(),
            "truncated": false
        }))
}

fn unavailable_output(
    action: LspAction,
    root: &Path,
    server: Option<&str>,
    reason: &str,
) -> ToolOutput {
    ToolOutput::new(format!("LSP unavailable: {reason}"))
        .with_title(format!("lsp {}", action.as_str()))
        .with_metadata(json!({
            "server": server,
            "workspace": root,
            "action": action.as_str(),
            "freshness": "unavailable",
            "items": [],
            "truncated": false
        }))
}

#[allow(clippy::too_many_arguments)]
fn shaped_output(
    action: LspAction,
    workspace: &LspWorkspace,
    document_version: Option<i64>,
    mut text: String,
    mut items: Value,
    max_chars: usize,
    freshness: &str,
) -> ToolOutput {
    let truncated = text.chars().count() > max_chars;
    if truncated {
        text = text.chars().take(max_chars).collect::<String>();
        text.push_str("\n… output truncated");
        items = json!([]);
    }
    ToolOutput::new(text)
        .with_title(format!("lsp {}", action.as_str()))
        .with_metadata(json!({
            "server": workspace.key().server_id,
            "workspace": workspace.key().canonical_root,
            "action": action.as_str(),
            "freshness": freshness,
            "document_version": document_version,
            "items": items,
            "truncated": truncated
        }))
}

fn render_diagnostics(items: &[Diagnostic], root: &Path, file: &Path) -> String {
    if items.is_empty() {
        return "No diagnostics.".to_owned();
    }
    let path = file.strip_prefix(root).unwrap_or(file).display();
    items
        .iter()
        .map(|diagnostic| {
            let severity = match diagnostic.severity {
                Some(DiagnosticSeverity::ERROR) => "error",
                Some(DiagnosticSeverity::WARNING) => "warning",
                Some(DiagnosticSeverity::INFORMATION) => "info",
                Some(DiagnosticSeverity::HINT) => "hint",
                _ => "diagnostic",
            };
            format!(
                "{severity} {path}:{}:{}: {}",
                diagnostic.range.start.line + 1,
                diagnostic.range.start.character + 1,
                diagnostic.message.replace('\n', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_locations(value: &Value, root: &Path) -> (String, usize) {
    let values = value.as_array().map(Vec::as_slice).unwrap_or_else(|| {
        if value.is_null() {
            &[]
        } else {
            std::slice::from_ref(value)
        }
    });
    let lines = values
        .iter()
        .filter_map(|location| {
            let uri = location
                .get("uri")
                .or_else(|| location.get("targetUri"))?
                .as_str()?;
            let range = location
                .get("range")
                .or_else(|| location.get("targetSelectionRange"))?;
            let start = range.get("start")?;
            let line = start.get("line")?.as_u64()? + 1;
            let character = start.get("character")?.as_u64()? + 1;
            Some(format!("{}:{line}:{character}", display_uri(uri, root)))
        })
        .collect::<Vec<_>>();
    let count = lines.len();
    if lines.is_empty() {
        ("No locations found.".to_owned(), 0)
    } else {
        (lines.join("\n"), count)
    }
}

fn render_hover(value: &Value) -> String {
    let Some(contents) = value.get("contents") else {
        return "No hover information.".to_owned();
    };
    if let Some(text) = contents.as_str() {
        return text.to_owned();
    }
    if let Some(text) = contents.get("value").and_then(Value::as_str) {
        return text.to_owned();
    }
    if let Some(items) = contents.as_array() {
        let text = items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .or_else(|| item.get("value").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    "Hover information was returned in an unsupported shape.".to_owned()
}

fn render_symbols(value: &Value, root: &Path) -> (String, usize) {
    let mut lines = Vec::new();
    collect_symbols(value, root, &mut lines, 0);
    let count = lines.len();
    if lines.is_empty() {
        ("No symbols found.".to_owned(), 0)
    } else {
        (lines.join("\n"), count)
    }
}

fn collect_symbols(value: &Value, root: &Path, lines: &mut Vec<String>, depth: usize) {
    let Some(items) = value.as_array() else {
        return;
    };
    for item in items {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            let location = item
                .get("location")
                .and_then(|location| location.get("uri"))
                .and_then(Value::as_str)
                .map(|uri| format!(" — {}", display_uri(uri, root)))
                .unwrap_or_default();
            lines.push(format!("{}{}{location}", "  ".repeat(depth), name));
        }
        if let Some(children) = item.get("children") {
            collect_symbols(children, root, lines, depth + 1);
        }
    }
}

fn display_uri(uri: &str, root: &Path) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|uri| uri.to_file_path().ok())
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
        })
        .unwrap_or_else(|| uri.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, ToolDefinition};
    use crate::provider::{EventStream, Provider};

    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            _system: &str,
            _resume_session_id: Option<&str>,
        ) -> anyhow::Result<EventStream> {
            Err(anyhow!("mock provider should not be called"))
        }

        fn name(&self) -> &str {
            "mock"
        }

        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self)
        }
    }

    #[test]
    fn converts_one_based_agent_positions_to_lsp_positions() {
        assert_eq!(
            one_based_position(Some(42), Some(5)).unwrap(),
            Position {
                line: 41,
                character: 4
            }
        );
        assert!(one_based_position(Some(0), Some(1)).is_err());
        assert!(one_based_position(Some(1), Some(0)).is_err());
    }

    #[test]
    fn missing_executable_is_a_graceful_status() {
        let config = LspConfig {
            servers: std::collections::BTreeMap::from([(
                "missing".to_owned(),
                jcode_lsp::LspServerConfig {
                    command: "jcode-definitely-missing-lsp".to_owned(),
                    ..Default::default()
                },
            )]),
            ..LspConfig::default()
        };
        let tool = LspTool::with_config(Arc::new(LspServicePool::new()), config);
        let output = render_status(&tool.config(), &std::env::current_dir().unwrap());
        assert!(output.output.contains("missing"));
        assert_eq!(output.metadata.unwrap()["servers"][0]["status"], "missing");
    }

    #[test]
    fn renders_locations_without_raw_protocol_payloads() {
        let root = Path::new("/workspace");
        let value = json!([{
            "uri": "file:///workspace/src/lib.rs",
            "range": {"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 8}}
        }]);
        assert_eq!(
            render_locations(&value, root),
            ("src/lib.rs:5:3".to_owned(), 1)
        );
    }

    #[tokio::test]
    async fn service_registry_exposes_lsp_without_starting_a_process() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider);
        let pool = Arc::new(LspServicePool::new());
        let registry = super::super::Registry::new_with_services(
            provider,
            crate::server::FileSnapshotLedger::new(),
            Arc::clone(&pool),
        )
        .await;
        assert!(registry.tool_names().await.iter().any(|name| name == "lsp"));
        assert!(pool.is_empty().await);
    }

    #[tokio::test]
    async fn new_without_server_services_does_not_expose_lsp() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider);
        let registry = super::super::Registry::new(provider).await;
        assert!(!registry.tool_names().await.iter().any(|name| name == "lsp"));
    }

    #[tokio::test]
    async fn installed_rust_analyzer_definition_flows_through_the_tool_contract() {
        let current = std::env::current_dir().unwrap();
        if discover_executable(
            "rust-analyzer",
            std::env::var_os("PATH").as_deref(),
            &current,
        )
        .is_err()
        {
            return;
        }

        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::write(
            project.path().join("Cargo.toml"),
            "[package]\nname = \"tool-lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        let source = "pub fn target() {}\npub fn caller() { target(); }\n";
        std::fs::write(project.path().join("src/lib.rs"), source).unwrap();
        let character = source.lines().nth(1).unwrap().find("target").unwrap() as u32 + 1;

        let pool = Arc::new(LspServicePool::new());
        let tool = LspTool::with_config(Arc::clone(&pool), LspConfig::default());
        let output = tool
            .execute(
                json!({
                    "action": "definition",
                    "file": "src/lib.rs",
                    "line": 2,
                    "character": character,
                    "intent": "Verify the public LSP tool path"
                }),
                ToolContext {
                    session_id: "lsp-test".to_owned(),
                    message_id: "message".to_owned(),
                    tool_call_id: "call".to_owned(),
                    working_dir: Some(project.path().to_owned()),
                    stdin_request_tx: None,
                    graceful_shutdown_signal: None,
                    execution_mode: super::super::ToolExecutionMode::Direct,
                },
            )
            .await
            .unwrap();
        assert!(output.output.contains("src/lib.rs:1:"), "{}", output.output);
        assert_eq!(output.metadata.as_ref().unwrap()["freshness"], "fresh");
        pool.shutdown_all(Duration::from_secs(5)).await;
    }
}
