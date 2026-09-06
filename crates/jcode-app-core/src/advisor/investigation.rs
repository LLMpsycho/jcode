//! Workspace-confined, permission-aware investigative access for an advisor.
//! Shares implementations and grants with the primary registry, never its read
//! coverage or tool-result capture. No shell, MCP, writes, or delegated tools.

use super::{redact_secrets, truncate_utf8};
use crate::message::ToolDefinition;
use crate::tool::{Registry, ToolContext, ToolExecutionMode};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_RESULT_BYTES: usize = 8 * 1024;
const TOOL_DEADLINE: Duration = Duration::from_secs(5);

pub struct AdvisorInvestigation {
    registry: Registry,
    parent_session: String,
    root: PathBuf,
    isolated_session: String,
}

impl AdvisorInvestigation {
    pub fn restriction_notice(&self) -> Option<&'static str> {
        crate::hooks::hook_configured("pre_tool").then_some(
            "Investigation tools are unavailable because a pre_tool policy hook is configured; the advisor cannot bypass the hook or execute its shell commands. Review the supplied evidence and report this limitation when material."
        )
    }

    pub fn new(registry: Registry, parent_session: String, working_dir: PathBuf) -> Result<Self> {
        let root = working_dir
            .canonicalize()
            .context("Advisor workspace is unavailable")?;
        ensure!(root.is_dir(), "Advisor workspace must be a directory");
        Ok(Self {
            registry,
            parent_session,
            root,
            isolated_session: format!("advisor-investigation-{}", uuid::Uuid::new_v4()),
        })
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();
        for name in ["read", "agentgrep"] {
            if let Ok(tool) = self
                .registry
                .advisor_read_tool(&self.parent_session, name, &Value::Null)
                .await
            {
                let mut definition = tool.to_definition();
                if name == "agentgrep" {
                    definition.description = "Search this workspace's code or filenames. Only grep and find modes. Hidden files and credential stores are excluded. Ignore rules apply by default; an explicit glob can include matching ignored files. Narrow path/query when results are truncated.".into();
                    definition.input_schema["properties"]["mode"]["enum"] = json!(["grep", "find"]);
                    if let Some(properties) = definition.input_schema["properties"].as_object_mut()
                    {
                        properties.remove("terms");
                    }
                } else {
                    definition.description = "Read a text file inside this workspace, up to 200 lines per call. Files above 1 MiB and credential stores are unavailable.".into();
                    definition.input_schema["properties"]["limit"]["maximum"] = json!(200);
                }
                definitions.push(definition);
            }
        }
        definitions
    }

    pub async fn execute(&self, name: &str, input: &Value) -> Result<String> {
        let name = Registry::resolve_tool_name(name);
        // Check grants and actual implementation metadata before interpreting
        // paths; revoked permission must fail even for an existing reader.
        let tool = self
            .registry
            .advisor_read_tool(&self.parent_session, name, input)
            .await?;
        let input = self.scoped_input(name, input)?;
        let ctx = ToolContext {
            session_id: self.isolated_session.clone(),
            message_id: "advisor-investigation".into(),
            tool_call_id: format!("advisor-read-{}", uuid::Uuid::new_v4()),
            working_dir: Some(self.root.clone()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::Direct,
        };
        let output = tokio::time::timeout(TOOL_DEADLINE, tool.execute(input, ctx))
            .await
            .map_err(|_| {
                anyhow::anyhow!("Advisor investigation timed out; narrow the request")
            })??;
        Ok(bounded_excerpt(&output.output, MAX_RESULT_BYTES))
    }

    fn scoped_input(&self, name: &str, input: &Value) -> Result<Value> {
        ensure!(input.is_object(), "Tool input must be an object");
        ensure!(
            input.to_string().len() <= 8 * 1024,
            "Investigative arguments are too large"
        );
        match name {
            "read" => {
                let path = self.scoped_path(
                    input
                        .get("file_path")
                        .and_then(Value::as_str)
                        .context("read requires file_path")?,
                )?;
                let metadata = std::fs::metadata(&path)?;
                ensure!(
                    metadata.is_file() && metadata.len() <= MAX_FILE_BYTES,
                    "Advisor reads require a regular text file at most 1 MiB"
                );
                let start = input
                    .get("start_line")
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        input
                            .get("offset")
                            .and_then(Value::as_u64)
                            .and_then(|offset| offset.checked_add(1))
                    })
                    .unwrap_or(1);
                ensure!((1..=1_000_000).contains(&start), "Invalid read start line");
                let limit = input
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .clamp(1, 200);
                Ok(json!({"file_path":path,"start_line":start,"limit":limit}))
            }
            "agentgrep" => {
                ensure!(
                    input.get("hidden") != Some(&Value::Bool(true))
                        && input.get("no_ignore") != Some(&Value::Bool(true)),
                    "Advisor searches do not support hidden or no_ignore options"
                );
                let mode = input.get("mode").and_then(Value::as_str).unwrap_or("grep");
                ensure!(
                    matches!(mode, "grep" | "find"),
                    "Advisor search supports grep and find only"
                );
                let requested_path = input
                    .get("path")
                    .or_else(|| input.get("file"))
                    .or_else(|| input.get("file_path"))
                    .and_then(Value::as_str)
                    .unwrap_or(".");
                let path = self.scoped_path(requested_path)?;
                let query = input
                    .get("query")
                    .or_else(|| input.get("pattern"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                ensure!(mode == "find" || !query.is_empty(), "grep requires a query");
                let mut scoped = json!({"mode":mode,"path":path,"query":query,"regex":input.get("regex").and_then(Value::as_bool).unwrap_or(false)});
                for key in ["glob", "type"] {
                    if let Some(value) = input.get(key).and_then(Value::as_str) {
                        ensure!(value.len() <= 512, "Search filter is too long");
                        scoped[key] = json!(value);
                    }
                }
                Ok(scoped)
            }
            _ => anyhow::bail!("Tool is not available to advisor investigation"),
        }
    }

    fn scoped_path(&self, value: &str) -> Result<PathBuf> {
        let requested = Path::new(value);
        ensure!(
            !sensitive_path(requested),
            "Credential stores are unavailable to advisor investigation"
        );
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };
        let canonical = joined
            .canonicalize()
            .context("Investigative path is unavailable")?;
        let relative = canonical.strip_prefix(&self.root).map_err(|_| {
            anyhow::anyhow!("Advisor path must remain inside the session workspace")
        })?;
        ensure!(
            !sensitive_path(relative),
            "Credential stores are unavailable to advisor investigation"
        );
        Ok(canonical)
    }
}

fn sensitive_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(value) = component else {
            return false;
        };
        let value = value.to_string_lossy().to_ascii_lowercase();
        matches!(
            value.as_str(),
            ".git" | ".ssh" | ".aws" | ".jcode" | ".netrc" | ".npmrc" | "auth.json" | "oauth.json"
        ) || value.starts_with(".env")
            || value.contains("credentials")
            || value.contains("secrets")
            || value.starts_with("id_rsa")
            || value.starts_with("id_ed25519")
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|suffix| value.ends_with(suffix))
    })
}

pub(crate) fn bounded_json_excerpt(value: &Value, max_bytes: usize) -> String {
    struct LimitedWriter(Vec<u8>);
    impl std::io::Write for LimitedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = (64 * 1024 - self.0.len()).min(bytes.len());
            if count == 0 {
                return Err(std::io::Error::other("advisor JSON byte limit"));
            }
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = LimitedWriter(Vec::new());
    let truncated = serde_json::to_writer(&mut writer, value).is_err();
    let mut text = String::from_utf8_lossy(&writer.0).into_owned();
    if truncated {
        text.push_str("\n[JSON excerpt truncated]");
    }
    bounded_excerpt(&text, max_bytes)
}

pub(crate) fn bounded_excerpt(value: &str, max_bytes: usize) -> String {
    // Bound redaction work as well as model input. Never expose a partial PEM
    // block when the scan boundary cuts off its closing delimiter.
    let mut end = value.len().min(64 * 1024);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut prefix = value[..end].to_string();
    if let Some(begin) = prefix.find("-----BEGIN ")
        && prefix[begin..].contains("PRIVATE KEY-----")
    {
        prefix.truncate(begin);
        prefix.push_str("[redacted private key]");
    }
    // Tool arguments and source excerpts also contain JSON credentials, whose
    // opaque values need not resemble a vendor token or shell assignment.
    static JSON_CREDENTIAL: OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        OnceLock::new();
    let Ok(credential) = JSON_CREDENTIAL.get_or_init(|| {
        regex::Regex::new(r#"(?i)("(?:[a-z0-9_]*(?:api_key|token|secret|password|cookie|private_key)|authorization)"\s*:\s*)"(?:\\.|[^"\\])*"?"#)
    }) else {
        return truncate_utf8("[excerpt unavailable: credential redaction failed]".into(), max_bytes);
    };
    let prefix = credential.replace_all(&prefix, "${1}\"[REDACTED_SECRET]\"");
    let redacted = redact_secrets(&prefix);
    if end < value.len() || redacted.len() > max_bytes {
        let mut result = truncate_utf8(redacted, max_bytes.saturating_sub(32));
        result.push_str("\n[excerpt truncated]");
        result
    } else {
        redacted
    }
}

#[cfg(test)]
mod tests;
