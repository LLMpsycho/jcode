use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use jcode_lsp::{
    Diagnostic, DiagnosticSeverity, LspConfig, LspError, LspServicePool, LspWorkspace,
    PostEditDiagnosticsMode, Range, SemanticVerification, SemanticVerificationStatus,
    discover_executable,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::lsp_action::LspAction;
use super::lsp_support::{
    discover_server_root, language_id, one_based_position, resolve_file, resolve_new_file,
    select_server, workspace_root,
};
use super::{Tool, ToolContext, ToolOutput};

pub struct LspTool {
    pool: Arc<LspServicePool>,
    file_snapshots: crate::server::FileSnapshotLedger,
    config_override: Option<LspConfig>,
}

impl LspTool {
    pub(crate) fn new(
        pool: Arc<LspServicePool>,
        file_snapshots: crate::server::FileSnapshotLedger,
    ) -> Self {
        Self {
            pool,
            file_snapshots,
            config_override: None,
        }
    }

    #[cfg(test)]
    fn with_config(pool: Arc<LspServicePool>, config: LspConfig) -> Self {
        Self {
            pool,
            file_snapshots: crate::server::FileSnapshotLedger::new(),
            config_override: Some(config),
        }
    }

    fn config(&self) -> LspConfig {
        self.config_override
            .clone()
            .unwrap_or_else(|| crate::config::config().lsp.clone())
    }
}

#[derive(Deserialize)]
struct LspInput {
    action: LspAction,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    new_name: Option<String>,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    selector: Option<String>,
    intent: String,
}

#[async_trait]
impl Tool for LspTool {
    fn capability(&self, input: &serde_json::Value) -> crate::tool::ToolCapability {
        use crate::tool::ToolCapability;
        if input
            .get("apply")
            .is_some_and(|v| !v.is_null() && v != &serde_json::Value::Bool(false))
        {
            return ToolCapability::WriteFiles;
        }
        ToolCapability::for_actions(
            input,
            &[
                "status",
                "diagnostics",
                "hover",
                "definition",
                "references",
                "document_symbols",
                "workspace_symbols",
                "rename",
                "rename_file",
                "code_actions",
                "implementation",
                "type_definition",
                "signature_help",
                "incoming_calls",
                "outgoing_calls",
                "capabilities",
            ],
            ToolCapability::Execute,
        )
    }

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
                        "document_symbols", "workspace_symbols", "capabilities", "rename", "rename_file",
                        "implementation", "type_definition", "signature_help", "incoming_calls",
                        "outgoing_calls", "code_actions", "reload"
                    ]
                },
                "server": {"type": ["string", "null"], "description": "Configured server id. Required for workspace-only actions when multiple servers exist."},
                "file": {"type": ["string", "null"], "description": "Workspace-relative source file."},
                "line": {"type": ["integer", "null"], "minimum": 1, "description": "One-based source line."},
                "character": {"type": ["integer", "null"], "minimum": 1, "description": "One-based UTF-16 character offset."},
                "query": {"type": ["string", "null"], "description": "Workspace-symbol query."},
                "new_name": {"type": ["string", "null"], "description": "New symbol name, or workspace-relative destination path for rename_file."},
                "selector": {"type": ["string", "null"], "description": "Exact code-action title or one-based list index to apply."},
                "apply": {"type": "boolean", "default": false, "description": "Apply a semantic/file rename or explicitly selected code action through the snapshot guard."},
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
            return Ok(render_status(&config, &root, &ctx.session_id, &self.pool).await);
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
        let server_id = select_server(&config, params.server.as_deref(), file.as_deref())?;
        let service_root = discover_server_root(
            &root,
            file.as_deref(),
            config
                .servers
                .get(&server_id)
                .ok_or_else(|| anyhow!("LSP server `{server_id}` is not configured"))?,
        );
        if matches!(params.action, LspAction::Reload) {
            let workspace_identity = workspace_identity(&config, &root, &ctx.session_id);
            return match self
                .pool
                .reload(&service_root, workspace_identity, &server_id, &config)
                .await
            {
                Ok(workspace) => Ok(shaped_output(
                    params.action,
                    &workspace,
                    None,
                    format!("Reloaded {} for {}.", server_id, root.display()),
                    json!([]),
                    config.max_output_tokens.saturating_mul(4).max(256),
                    "fresh",
                )),
                Err(
                    error @ (LspError::ExecutableNotFound { .. }
                    | LspError::NotExecutable { .. }
                    | LspError::RestartBackoff { .. }),
                ) => Ok(unavailable_output(
                    params.action,
                    &root,
                    Some(&server_id),
                    &error.to_string(),
                )),
                Err(error) => Err(error.into()),
            };
        }
        let workspace_identity = workspace_identity(&config, &root, &ctx.session_id);
        let workspace = match self
            .pool
            .get_or_start(&service_root, workspace_identity, &server_id, &config)
            .await
        {
            Ok(workspace) => workspace,
            Err(
                error @ (LspError::ExecutableNotFound { .. }
                | LspError::NotExecutable { .. }
                | LspError::RestartBackoff { .. }),
            ) => {
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
            LspAction::Reload => unreachable!(),
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
                let document_language_id = language_id(file);
                let document = workspace
                    .sync_document_from_disk(file, document_language_id)
                    .await?;
                match action {
                    LspAction::Diagnostics => {
                        let snapshot = workspace
                            .current_diagnostics(
                                &document,
                                Duration::from_millis(config.post_edit_wait_ms),
                            )
                            .await?
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
                            json!({"count": items.len(), "diagnostic_evidence": {"path": file, "freshness": freshness, "items": items.iter().take(32).collect::<Vec<_>>()}}),
                            max_chars,
                            freshness,
                        ))
                    }
                    LspAction::Hover
                    | LspAction::Definition
                    | LspAction::References
                    | LspAction::Implementation
                    | LspAction::TypeDefinition
                    | LspAction::SignatureHelp
                    | LspAction::IncomingCalls
                    | LspAction::OutgoingCalls => {
                        let position = one_based_position(params.line, params.character)?;
                        let value = match action {
                            LspAction::Hover => workspace.hover(file, position).await?,
                            LspAction::Definition => workspace.definition(file, position).await?,
                            LspAction::References => workspace.references(file, position).await?,
                            LspAction::Implementation => {
                                workspace.implementation(file, position).await?
                            }
                            LspAction::TypeDefinition => {
                                workspace.type_definition(file, position).await?
                            }
                            LspAction::SignatureHelp => {
                                workspace.signature_help(file, position).await?
                            }
                            LspAction::IncomingCalls => {
                                workspace.incoming_calls(file, position).await?
                            }
                            LspAction::OutgoingCalls => {
                                workspace.outgoing_calls(file, position).await?
                            }
                            _ => unreachable!(),
                        };
                        let (text, count) = match action {
                            LspAction::Hover => {
                                (render_hover(&value), usize::from(!value.is_null()))
                            }
                            LspAction::SignatureHelp => render_signature_help(&value),
                            LspAction::IncomingCalls | LspAction::OutgoingCalls => {
                                render_call_hierarchy(&value, &root, action)
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
                    LspAction::CodeActions => {
                        let position = one_based_position(params.line, params.character)?;
                        let value = workspace
                            .code_actions(
                                file,
                                Range {
                                    start: position,
                                    end: position,
                                },
                            )
                            .await?;
                        let (text, actions) = render_code_actions(&value);
                        if params.apply {
                            let selector = params
                                .selector
                                .as_deref()
                                .filter(|selector| !selector.trim().is_empty())
                                .ok_or_else(|| {
                                    anyhow!(
                                        "applying a code action requires an explicit `selector`"
                                    )
                                })?;
                            let selected = select_code_action(&value, selector)?;
                            let title = selected
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("selected code action")
                                .to_owned();
                            let resolved = if selected.get("edit").is_some() {
                                selected
                            } else {
                                workspace.resolve_code_action(selected).await?
                            };
                            let edit = resolved.get("edit").ok_or_else(|| {
                                anyhow!(
                                    "code action `{title}` has no WorkspaceEdit; command-only actions must run through normal tool permissions"
                                )
                            })?;
                            let command_skipped = resolved.get("command").is_some();
                            let applied = super::lsp_rename::apply_workspace_edit_for_operation(
                                edit,
                                &root,
                                &ctx,
                                self.file_snapshots.clone(),
                                params.intent.clone(),
                                "code action",
                            )
                            .await?;
                            let mut metadata = applied.metadata;
                            if let Some(metadata) = metadata.as_object_mut() {
                                metadata.insert("code_action".to_owned(), json!(title));
                                metadata.insert("selector".to_owned(), json!(selector));
                                metadata
                                    .insert("command_skipped".to_owned(), json!(command_skipped));
                            }
                            let mut output = ToolOutput::new(format!(
                                "Applied code action `{title}` to {} file(s), {} edit(s).{}",
                                applied.file_count,
                                applied.edit_count,
                                if command_skipped {
                                    " The action's command was not run; execute it separately through normal tool permissions."
                                } else {
                                    ""
                                }
                            ))
                            .with_title("lsp code_actions")
                            .with_metadata(metadata);
                            attach_post_edit_feedback(
                                "anchored_edit",
                                &mut output,
                                &ctx,
                                Arc::clone(&self.pool),
                            )
                            .await;
                            return Ok(output);
                        }
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            text,
                            json!({"count": actions.len(), "actions": actions, "applied": false}),
                            max_chars,
                            "fresh",
                        ))
                    }
                    LspAction::Rename => {
                        let position = one_based_position(params.line, params.character)?;
                        let new_name = params
                            .new_name
                            .as_deref()
                            .filter(|name| !name.trim().is_empty())
                            .ok_or_else(|| anyhow!("lsp rename requires `new_name`"))?;
                        let preparation = workspace.prepare_rename(file, position).await?;
                        if preparation.is_null() {
                            bail!("the language server rejected rename at this position");
                        }
                        let edit = workspace.rename(file, position, new_name).await?;
                        if params.apply {
                            let applied = super::lsp_rename::apply_workspace_edit(
                                &edit,
                                &root,
                                &ctx,
                                self.file_snapshots.clone(),
                                params.intent.clone(),
                            )
                            .await?;
                            let mut output = ToolOutput::new(format!(
                                "Applied semantic rename to {} file(s), {} edit(s).",
                                applied.file_count, applied.edit_count
                            ))
                            .with_title("lsp rename")
                            .with_metadata(applied.metadata);
                            attach_post_edit_feedback(
                                "anchored_edit",
                                &mut output,
                                &ctx,
                                Arc::clone(&self.pool),
                            )
                            .await;
                            return Ok(output);
                        }
                        let (text, summary, count) = render_workspace_edit(&edit, &root)?;
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            text,
                            json!({"files": summary, "edit_count": count, "applied": false}),
                            max_chars,
                            "fresh",
                        ))
                    }
                    LspAction::RenameFile => {
                        let new_name = params
                            .new_name
                            .as_deref()
                            .filter(|name| !name.trim().is_empty())
                            .ok_or_else(|| anyhow!("lsp rename_file requires `new_name`"))?;
                        let destination = resolve_new_file(&root, new_name)?;
                        let edit = workspace.will_rename_file(file, &destination).await?;
                        let edit = if edit.is_null() { json!({}) } else { edit };
                        if params.apply {
                            workspace.close_document(file).await?;
                            let applied = super::lsp_rename::apply_workspace_edit_and_file_rename(
                                &edit,
                                file,
                                &destination,
                                &root,
                                &ctx,
                                self.file_snapshots.clone(),
                                params.intent.clone(),
                            )
                            .await;
                            let applied = match applied {
                                Ok(applied) => applied,
                                Err(error) => {
                                    let _ = workspace
                                        .sync_document_from_disk(file, document_language_id)
                                        .await;
                                    return Err(error);
                                }
                            };
                            let mut lifecycle_warnings = Vec::new();
                            if let Err(error) = workspace.did_rename_file(file, &destination).await
                            {
                                lifecycle_warnings.push(format!("didRenameFiles failed: {error}"));
                            }
                            if let Err(error) = workspace
                                .sync_document_from_disk(&destination, language_id(&destination))
                                .await
                            {
                                lifecycle_warnings
                                    .push(format!("reopening destination failed: {error}"));
                            }
                            let warning = if lifecycle_warnings.is_empty() {
                                String::new()
                            } else {
                                format!(" LSP lifecycle warning: {}", lifecycle_warnings.join("; "))
                            };
                            let mut output = ToolOutput::new(format!(
                                "Renamed {} to {} and applied {} related edit(s).{}",
                                file.strip_prefix(&root).unwrap_or(file).display(),
                                destination
                                    .strip_prefix(&root)
                                    .unwrap_or(&destination)
                                    .display(),
                                applied.edit_count,
                                warning
                            ))
                            .with_title("lsp rename_file")
                            .with_metadata(applied.metadata);
                            attach_post_edit_feedback(
                                "anchored_edit",
                                &mut output,
                                &ctx,
                                Arc::clone(&self.pool),
                            )
                            .await;
                            return Ok(output);
                        }
                        let (text, summary, count) = render_workspace_edit(&edit, &root)?;
                        let source = file.strip_prefix(&root).unwrap_or(file).display();
                        let destination_display = destination
                            .strip_prefix(&root)
                            .unwrap_or(&destination)
                            .display();
                        Ok(shaped_output(
                            action,
                            &workspace,
                            Some(document.version),
                            format!(
                                "File rename preview: {source} -> {destination_display}\n{text}"
                            ),
                            json!({
                                "source": source.to_string(),
                                "destination": destination_display.to_string(),
                                "files": summary,
                                "edit_count": count,
                                "applied": false
                            }),
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

async fn render_status(
    config: &LspConfig,
    root: &Path,
    session_id: &str,
    pool: &LspServicePool,
) -> ToolOutput {
    if !config.enabled {
        return unavailable_output(
            LspAction::Status,
            root,
            None,
            "LSP is disabled by configuration",
        );
    }
    let path = std::env::var_os("PATH");
    let identity = workspace_identity(config, root, session_id);
    let mut statuses = Vec::new();
    let mut lines = Vec::new();
    for (id, server) in &config.servers {
        let discovered = discover_executable(&server.command, path.as_deref(), root);
        let runtime = pool
            .status(root, identity.clone(), id, config)
            .await
            .ok()
            .flatten();
        let (status, exit_code) = match runtime.as_ref().map(|runtime| &runtime.process_status) {
            Some(jcode_lsp::ProcessStatus::Running) => ("running", None),
            Some(jcode_lsp::ProcessStatus::Exited { code }) => ("exited", *code),
            None if discovered.is_ok() => ("available", None),
            None => ("missing", None),
        };
        let recent_stderr = runtime
            .as_ref()
            .map(|runtime| {
                bounded_tail(
                    &crate::message::redact_secrets(&runtime.recent_stderr),
                    1024,
                )
            })
            .filter(|stderr| !stderr.trim().is_empty());
        lines.push(format!(
            "{id}: {status}{}{}",
            discovered
                .as_ref()
                .ok()
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default(),
            recent_stderr
                .as_ref()
                .map(|stderr| format!("\n  recent stderr: {}", stderr.replace('\n', "\n  ")))
                .unwrap_or_default()
        ));
        statuses.push(json!({
            "id": id,
            "status": status,
            "exit_code": exit_code,
            "recent_stderr": recent_stderr
        }));
    }
    let text = lines.join("\n");
    ToolOutput::new(text)
        .with_title("lsp status")
        .with_metadata(json!({
            "action": "status",
            "workspace": root,
            "freshness": "fresh",
            "servers": statuses,
            "truncated": false
        }))
}

fn bounded_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    text.chars().skip(count - max_chars).collect()
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

fn render_signature_help(value: &Value) -> (String, usize) {
    let signatures = value
        .get("signatures")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let lines = signatures
        .iter()
        .filter_map(|signature| signature.get("label").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let count = lines.len();
    if lines.is_empty() {
        ("No signature help.".to_owned(), 0)
    } else {
        (lines.join("\n"), count)
    }
}

fn render_call_hierarchy(value: &Value, root: &Path, action: LspAction) -> (String, usize) {
    let entries = value.as_array().map(Vec::as_slice).unwrap_or_default();
    let key = if matches!(action, LspAction::IncomingCalls) {
        "from"
    } else {
        "to"
    };
    let lines = entries
        .iter()
        .filter_map(|entry| entry.get(key))
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?;
            let uri = item.get("uri").and_then(Value::as_str)?;
            Some(format!("{name} — {}", display_uri(uri, root)))
        })
        .collect::<Vec<_>>();
    let count = lines.len();
    if lines.is_empty() {
        ("No calls found.".to_owned(), 0)
    } else {
        (lines.join("\n"), count)
    }
}

fn render_code_actions(value: &Value) -> (String, Vec<Value>) {
    let actions = value.as_array().map(Vec::as_slice).unwrap_or_default();
    let summaries = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            let title = action.get("title").and_then(Value::as_str)?;
            Some(json!({
                "index": index + 1,
                "title": title,
                "kind": action.get("kind"),
                "disabled_reason": action.get("disabled").and_then(|value| value.get("reason"))
            }))
        })
        .collect::<Vec<_>>();
    let lines = summaries
        .iter()
        .filter_map(|action| {
            let index = action.get("index").and_then(Value::as_u64)?;
            let title = action.get("title").and_then(Value::as_str)?;
            let kind = action
                .get("kind")
                .and_then(Value::as_str)
                .map(|kind| format!(" [{kind}]"))
                .unwrap_or_default();
            let disabled = action
                .get("disabled_reason")
                .and_then(Value::as_str)
                .map(|reason| format!(" (disabled: {reason})"))
                .unwrap_or_default();
            Some(format!("{index}. {title}{kind}{disabled}"))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        ("No code actions.".to_owned(), summaries)
    } else {
        (lines.join("\n"), summaries)
    }
}

fn select_code_action(value: &Value, selector: &str) -> Result<Value> {
    let actions = value
        .as_array()
        .ok_or_else(|| anyhow!("language server returned an invalid code-action list"))?;
    let selector = selector.trim();
    let selected = selector
        .strip_prefix('#')
        .unwrap_or(selector)
        .parse::<usize>()
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| actions.get(index))
        .cloned()
        .or_else(|| {
            let matches = actions
                .iter()
                .filter(|action| action.get("title").and_then(Value::as_str) == Some(selector))
                .collect::<Vec<_>>();
            (matches.len() == 1).then(|| matches[0].clone())
        })
        .ok_or_else(|| anyhow!("code-action selector `{selector}` matched no unique action"))?;
    if let Some(reason) = selected
        .get("disabled")
        .and_then(|disabled| disabled.get("reason"))
        .and_then(Value::as_str)
    {
        bail!("selected code action is disabled: {reason}");
    }
    Ok(selected)
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

fn render_workspace_edit(value: &Value, root: &Path) -> Result<(String, Vec<Value>, usize)> {
    let mut lines = Vec::new();
    let mut summary = Vec::new();
    let mut total = 0;
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            collect_workspace_edit_summary(
                uri,
                edits.as_array().map(Vec::len).unwrap_or(0),
                root,
                &mut lines,
                &mut summary,
                &mut total,
            );
        }
    }
    if let Some(document_changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            let Some(uri) = change
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            collect_workspace_edit_summary(
                uri,
                change
                    .get("edits")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0),
                root,
                &mut lines,
                &mut summary,
                &mut total,
            );
        }
    }
    if !value.is_object() {
        bail!("language server returned an unsupported WorkspaceEdit shape");
    }
    if lines.is_empty() {
        Ok(("Rename produced no edits.".to_owned(), summary, 0))
    } else {
        Ok((
            format!("Rename preview:\n{}", lines.join("\n")),
            summary,
            total,
        ))
    }
}

fn collect_workspace_edit_summary(
    uri: &str,
    count: usize,
    root: &Path,
    lines: &mut Vec<String>,
    summary: &mut Vec<Value>,
    total: &mut usize,
) {
    if count == 0 {
        return;
    }
    *total += count;
    let path = display_uri(uri, root);
    lines.push(format!("{path}: {count} edit(s)"));
    summary.push(json!({"path": path, "edit_count": count}));
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

pub(crate) async fn attach_post_edit_feedback(
    tool_name: &str,
    output: &mut ToolOutput,
    ctx: &ToolContext,
    pool: Arc<LspServicePool>,
) {
    if !is_post_edit_tool(tool_name) {
        return;
    }
    let config = crate::config::config().lsp.clone();
    if !config.enabled || config.post_edit_diagnostics == PostEditDiagnosticsMode::Off {
        return;
    }
    let Some(root) = ctx
        .working_dir
        .as_deref()
        .and_then(|root| root.canonicalize().ok())
    else {
        return;
    };
    let paths = output
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("files"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| file.get("path").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return;
    }

    let mut files = Vec::new();
    let mut issue_lines = Vec::new();
    for relative_path in paths {
        let result =
            verify_written_file(&pool, &config, &root, &ctx.session_id, &relative_path).await;
        match result {
            Ok(verification) => {
                if verification.status == SemanticVerificationStatus::IssuesFound {
                    let path = root.join(&relative_path);
                    issue_lines.extend(
                        render_diagnostics(&verification.diagnostics, &root, &path)
                            .lines()
                            .map(|line| format!("+ {line}")),
                    );
                }
                files.push(json!({
                    "path": relative_path,
                    "status": verification.status,
                    "document_version": verification.document_version,
                    "diagnostics": verification.diagnostics
                }));
            }
            Err(error) => files.push(json!({
                "path": relative_path,
                "status": SemanticVerificationStatus::Unavailable,
                "document_version": null,
                "diagnostics": [],
                "reason": error.to_string()
            })),
        }
    }

    if !issue_lines.is_empty() {
        output.output.push_str(&format!(
            "\n\nDiagnostics delta after edit:\n{}",
            issue_lines.join("\n")
        ));
    }
    let aggregate = if files.iter().any(|file| file["status"] == "issues_found") {
        SemanticVerificationStatus::IssuesFound
    } else if files.iter().any(|file| file["status"] == "stale") {
        SemanticVerificationStatus::Stale
    } else if files.iter().any(|file| file["status"] == "unavailable") {
        SemanticVerificationStatus::Unavailable
    } else {
        SemanticVerificationStatus::Clean
    };
    let metadata = output.metadata.get_or_insert_with(|| json!({}));
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert(
            "semantic_verification".to_owned(),
            json!({"status": aggregate, "files": files}),
        );
    }
}

fn is_post_edit_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write" | "edit" | "multiedit" | "patch" | "apply_patch" | "anchored_edit"
    )
}

async fn verify_written_file(
    pool: &LspServicePool,
    config: &LspConfig,
    root: &Path,
    session_id: &str,
    relative_path: &str,
) -> std::result::Result<SemanticVerification, LspError> {
    let path = root
        .join(relative_path)
        .canonicalize()
        .map_err(LspError::from)?;
    if !path.starts_with(root) {
        return Err(LspError::InvalidWorkspaceUri {
            path: path.display().to_string(),
        });
    }
    let server_id = select_server(config, None, Some(&path))
        .map_err(|error| LspError::InvalidConfig(error.to_string()))?;
    let service_root = discover_server_root(
        root,
        Some(&path),
        config
            .servers
            .get(&server_id)
            .ok_or_else(|| LspError::UnknownServer {
                server_id: server_id.clone(),
            })?,
    );
    let workspace_identity = workspace_identity(config, root, session_id);
    let workspace = pool
        .get_or_start(&service_root, workspace_identity, &server_id, config)
        .await?;
    workspace
        .verify_disk_change(
            &path,
            language_id(&path),
            Duration::from_millis(config.post_edit_wait_ms),
        )
        .await
}

fn workspace_identity(config: &LspConfig, root: &Path, session_id: &str) -> String {
    if config.shared {
        root.display().to_string()
    } else {
        format!("{}#session={session_id}", root.display())
    }
}

#[cfg(test)]
#[path = "lsp/tests.rs"]
mod tests;
