use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, FileOp, FileTouch};
use crate::server::FileSnapshotLedger;
use crate::tool::file_write_guard::{
    FileWriteGuard, GuardedFile, RequiredCoverage, full_file_range, metadata_for_writes,
};
use anyhow::Result;
use async_trait::async_trait;
use jcode_edit_core::LineRange;
use serde::Deserialize;
use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};
use std::path::Path;

pub struct PatchTool {
    write_guard: Option<FileWriteGuard>,
}

impl PatchTool {
    pub fn new() -> Self {
        Self { write_guard: None }
    }

    pub(crate) fn with_file_snapshots(file_snapshots: FileSnapshotLedger) -> Self {
        Self {
            write_guard: Some(FileWriteGuard::new(file_snapshots)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_write_guard(write_guard: FileWriteGuard) -> Self {
        Self {
            write_guard: Some(write_guard),
        }
    }
}

#[derive(Deserialize)]
struct PatchInput {
    #[serde(default)]
    intent: Option<String>,
    patch_text: String,
}

struct PreparedPatch {
    path: String,
    resolved_path: std::path::PathBuf,
    old_content: String,
    new_content: Option<String>,
    message: String,
    diff: String,
    guarded: Option<GuardedFile>,
}

#[derive(Debug)]
struct FilePatch {
    path: String,
    hunks: Vec<Hunk>,
    is_new: bool,
    is_delete: bool,
}

#[derive(Debug)]
struct Hunk {
    old_start: usize,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

#[async_trait]
impl Tool for PatchTool {
    fn capability(&self, _input: &serde_json::Value) -> crate::tool::ToolCapability {
        crate::tool::ToolCapability::WriteFiles
    }

    fn name(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff (---/+++ headers). Prefer apply_patch for Codex patches."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["patch_text"],
            "properties": {
                "intent": super::intent_schema_property(),
                "patch_text": {
                    "type": "string",
                    "description": "Patch text."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: PatchInput = serde_json::from_value(input)?;

        let patches = parse_patch(&params.patch_text)?;

        if patches.is_empty() {
            return Err(anyhow::anyhow!("No valid patches found in input"));
        }

        // Watch config.toml across the whole invocation so an edit that lands
        // on it is reported regardless of which patch produced it.
        let config_watch = super::config_edit_notice::ConfigEditWatch::begin();
        let transaction = match &self.write_guard {
            Some(guard) => Some(guard.begin(&ctx).await?),
            None => None,
        };
        let mut prepared = Vec::with_capacity(patches.len());
        for patch in &patches {
            let resolved_path = ctx.resolve_path(Path::new(&patch.path));
            let (old_content, new_content, message, diff) =
                prepare_patch_with_diff(patch, &resolved_path).await?;
            let guarded = match &transaction {
                Some(transaction) if !old_content.is_empty() || resolved_path.exists() => Some(
                    transaction
                        .preflight_existing(
                            &resolved_path,
                            old_content.as_bytes(),
                            RequiredCoverage::Ranges(if patch.is_delete {
                                full_file_range(&old_content)
                            } else {
                                patch_ranges(patch, &old_content)
                            }),
                        )
                        .await?,
                ),
                Some(transaction) => Some(transaction.prepare_new(&resolved_path)?),
                None => None,
            };
            prepared.push(PreparedPatch {
                path: patch.path.clone(),
                resolved_path,
                old_content,
                new_content,
                message,
                diff,
                guarded,
            });
        }

        let mut results = Vec::new();
        let mut recorded_writes = Vec::new();

        for prepared in prepared {
            match &prepared.new_content {
                Some(content) => {
                    if let Some(parent) = prepared.resolved_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&prepared.resolved_path, content).await?;
                }
                None => tokio::fs::remove_file(&prepared.resolved_path).await?,
            }
            if let (Some(transaction), Some(guarded)) = (&transaction, prepared.guarded.clone()) {
                recorded_writes.push(
                    transaction
                        .record_success(
                            guarded,
                            &prepared.resolved_path,
                            prepared.new_content.as_deref().unwrap_or("").as_bytes(),
                        )
                        .await?,
                );
            }
            Bus::global().publish(BusEvent::FileTouch(FileTouch {
                session_id: ctx.session_id.clone(),
                path: prepared.resolved_path.clone(),
                op: FileOp::Edit,
                intent: params
                    .intent
                    .clone()
                    .filter(|value| !value.trim().is_empty()),
                summary: Some(format!("{} via patch", prepared.message)),
                detail: Some(prepared.diff.clone()).filter(|value| !value.is_empty()),
            }));
            let mut result = if prepared.diff.is_empty() {
                format!("✓ {}: {}", prepared.path, prepared.message)
            } else {
                format!(
                    "✓ {}: {}\n{}",
                    prepared.path, prepared.message, prepared.diff
                )
            };
            if let Some(guarded) = &prepared.guarded {
                guarded.append_warnings(&mut result);
            }
            results.push(result);
        }

        let mut body = results.join("\n\n");
        config_watch.finish(&mut body);
        let output = ToolOutput::new(body);
        Ok(if recorded_writes.is_empty() {
            output
        } else {
            output.with_metadata(metadata_for_writes(&recorded_writes))
        })
    }
}

fn patch_ranges(patch: &FilePatch, contents: &str) -> Vec<LineRange> {
    let line_count = contents.lines().count() as u64;
    patch
        .hunks
        .iter()
        .filter_map(|hunk| {
            if line_count == 0 {
                return None;
            }
            let start = (hunk.old_start as u64).clamp(1, line_count);
            let end = if hunk.old_lines.is_empty() {
                start
            } else {
                (start + hunk.old_lines.len() as u64 - 1).min(line_count)
            };
            Some(LineRange { start, end })
        })
        .collect()
}

fn parse_patch(text: &str) -> Result<Vec<FilePatch>> {
    let mut patches = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Look for --- line
        if lines[i].starts_with("---") {
            let old_file = lines[i]
                .strip_prefix("--- ")
                .unwrap_or("")
                .split('\t')
                .next()
                .unwrap_or("");

            i += 1;
            if i >= lines.len() || !lines[i].starts_with("+++") {
                continue;
            }

            let new_file = lines[i]
                .strip_prefix("+++ ")
                .unwrap_or("")
                .split('\t')
                .next()
                .unwrap_or("");

            // Determine the actual file path
            let path = if new_file == "/dev/null" {
                old_file.strip_prefix("a/").unwrap_or(old_file).to_string()
            } else {
                new_file.strip_prefix("b/").unwrap_or(new_file).to_string()
            };

            let is_new = old_file == "/dev/null";
            let is_delete = new_file == "/dev/null";

            i += 1;

            // Parse hunks
            let mut hunks = Vec::new();
            while i < lines.len() && !lines[i].starts_with("---") {
                if lines[i].starts_with("@@") {
                    if let Some(hunk) = parse_hunk(&lines, &mut i) {
                        hunks.push(hunk);
                    }
                } else {
                    i += 1;
                }
            }

            if !hunks.is_empty() || is_new || is_delete {
                patches.push(FilePatch {
                    path,
                    hunks,
                    is_new,
                    is_delete,
                });
            }
        } else {
            i += 1;
        }
    }

    Ok(patches)
}

fn parse_hunk(lines: &[&str], i: &mut usize) -> Option<Hunk> {
    // Parse @@ -start,count +start,count @@
    let header = lines[*i];
    let parts: Vec<&str> = header.split_whitespace().collect();

    if parts.len() < 3 {
        *i += 1;
        return None;
    }

    let old_range = parts[1].strip_prefix('-').unwrap_or(parts[1]);
    let old_start: usize = old_range
        .split(',')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    *i += 1;

    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();

    while *i < lines.len() {
        let line = lines[*i];

        if line.starts_with("@@") || line.starts_with("---") {
            break;
        }

        if let Some(content) = line.strip_prefix('-') {
            old_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix('+') {
            new_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix(' ') {
            old_lines.push(content.to_string());
            new_lines.push(content.to_string());
        } else if line.is_empty() || line == "\\ No newline at end of file" {
            // Context line or special marker
        }

        *i += 1;
    }

    Some(Hunk {
        old_start,
        old_lines,
        new_lines,
    })
}

/// Compute a patch result without mutating disk.
async fn prepare_patch_with_diff(
    patch: &FilePatch,
    path: &Path,
) -> Result<(String, Option<String>, String, String)> {
    // Handle deletion
    if patch.is_delete {
        if path.exists() {
            let old_content = tokio::fs::read_to_string(path).await.unwrap_or_default();
            let diff = generate_diff(&old_content, "", 1);
            return Ok((old_content, None, "deleted".to_string(), diff));
        } else {
            return Err(anyhow::anyhow!("file does not exist"));
        }
    }

    // Handle new file
    if patch.is_new {
        if path.exists() {
            return Err(anyhow::anyhow!("file already exists"));
        }

        // Collect all new lines from hunks
        let content: String = patch
            .hunks
            .iter()
            .flat_map(|h| h.new_lines.iter())
            .map(|l| format!("{}\n", l))
            .collect();

        let diff = generate_diff("", &content, 1);
        return Ok((String::new(), Some(content), "created".to_string(), diff));
    }

    // Handle modification
    if !path.exists() {
        return Err(anyhow::anyhow!("file does not exist"));
    }

    let old_content = tokio::fs::read_to_string(path).await?;
    let mut lines: Vec<String> = old_content.lines().map(|s| s.to_string()).collect();

    // Find the first affected line for diff context
    let first_line = patch.hunks.iter().map(|h| h.old_start).min().unwrap_or(1);

    // Apply hunks in reverse order to preserve line numbers
    for hunk in patch.hunks.iter().rev() {
        let start = hunk.old_start.saturating_sub(1);
        let end = (start + hunk.old_lines.len()).min(lines.len());

        // Remove old lines and insert new ones
        lines.splice(start..end, hunk.new_lines.iter().cloned());
    }

    let new_content = lines.join("\n") + "\n";
    let diff = generate_diff(&old_content, &new_content, first_line);
    Ok((
        old_content,
        Some(new_content),
        format!("modified ({} hunks)", patch.hunks.len()),
        diff,
    ))
}

/// Generate a compact diff with line numbers (max 30 lines)
fn generate_diff(old: &str, new: &str, start_line: usize) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut output = String::new();
    let mut line_count = 0;
    const MAX_LINES: usize = 30;

    let mut old_line = start_line;
    let mut new_line = start_line;

    for change in diff.iter_all_changes() {
        if line_count >= MAX_LINES {
            output.push_str("... (diff truncated)\n");
            break;
        }

        let content = change.value().trim_end_matches('\n');
        let (prefix, line_num) = match change.tag() {
            ChangeTag::Delete => {
                let num = old_line;
                old_line += 1;
                if content.trim().is_empty() {
                    continue;
                }
                ("-", num)
            }
            ChangeTag::Insert => {
                let num = new_line;
                new_line += 1;
                if content.trim().is_empty() {
                    continue;
                }
                ("+", num)
            }
            ChangeTag::Equal => {
                old_line += 1;
                new_line += 1;
                continue;
            }
        };

        output.push_str(&format!("{}{} {}\n", line_num, prefix, content));
        line_count += 1;
    }

    output.trim_end().to_string()
}
