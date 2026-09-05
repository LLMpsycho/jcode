use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, FileOp, FileTouch};
use crate::server::FileSnapshotLedger;
use crate::tool::file_write_guard::{FileWriteGuard, RequiredCoverage, metadata_for_writes};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};
use std::path::Path;

const FILE_TOUCH_PREVIEW_MAX_LINES: usize = 6;
const FILE_TOUCH_PREVIEW_MAX_BYTES: usize = 240;

pub struct WriteTool {
    write_guard: Option<FileWriteGuard>,
}

impl WriteTool {
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
struct WriteInput {
    #[serde(default)]
    intent: Option<String>,
    file_path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn capability(&self, _input: &serde_json::Value) -> crate::tool::ToolCapability {
        crate::tool::ToolCapability::WriteFiles
    }

    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write a file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "intent": super::intent_schema_property(),
                "file_path": {
                    "type": "string",
                    "description": "File path."
                },
                "content": {
                    "type": "string",
                    "description": "File content."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: WriteInput = serde_json::from_value(input)?;

        let path = ctx.resolve_path(Path::new(&params.file_path));

        // Create parent directories if needed
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Check if file existed before and read old content for diff
        let existed = path.exists();
        let old_content = if existed {
            tokio::fs::read_to_string(&path).await.ok()
        } else {
            None
        };
        let transaction = match &self.write_guard {
            Some(guard) => Some(guard.begin(&ctx).await?),
            None => None,
        };
        let guarded = match (&transaction, old_content.as_deref()) {
            (Some(transaction), Some(old_content)) => Some(
                transaction
                    .preflight_existing(&path, old_content.as_bytes(), RequiredCoverage::FullFile)
                    .await?,
            ),
            (Some(transaction), None) => Some(transaction.prepare_new(&path)?),
            (None, _) => None,
        };

        // Write the file
        tokio::fs::write(&path, &params.content).await?;
        let recorded = match (transaction.as_ref(), guarded.clone()) {
            (Some(transaction), Some(guarded)) => Some(
                transaction
                    .record_success(guarded, &path, params.content.as_bytes())
                    .await?,
            ),
            _ => None,
        };

        let _new_len = params.content.len();
        let line_count = params.content.lines().count();
        let diff = if let Some(old) = old_content.as_deref() {
            generate_diff_summary(old, &params.content)
        } else {
            generate_diff_summary("", &params.content)
        };
        let detail = build_file_touch_preview(&diff);

        // Publish file touch event for swarm coordination
        Bus::global().publish(BusEvent::FileTouch(FileTouch {
            session_id: ctx.session_id.clone(),
            path: path.to_path_buf(),
            op: FileOp::Write,
            intent: params
                .intent
                .clone()
                .filter(|value| !value.trim().is_empty()),
            summary: Some(if existed {
                format!("overwrote file ({} lines)", line_count)
            } else {
                format!("created new file ({} lines)", line_count)
            }),
            detail,
        }));

        let mut body = if existed {
            format!(
                "Updated {} ({} lines){}\n{}",
                params.file_path,
                line_count,
                if diff.is_empty() { "" } else { ":" },
                diff
            )
        } else {
            // For new files, show all lines as additions
            let diff = generate_diff_summary("", &params.content);
            format!(
                "Created {} ({} lines):\n{}",
                params.file_path, line_count, diff
            )
        };

        // A write that lands on the active config.toml states exactly which
        // settings changed and whether they are live, so neither the agent nor
        // the user has to guess whether the edit took effect.
        super::config_edit_notice::append_config_edit_notice(
            &mut body,
            &path,
            old_content.as_deref().unwrap_or(""),
            &params.content,
        );

        if let Some(guarded) = &guarded {
            guarded.append_warnings(&mut body);
        }

        let output = ToolOutput::new(body).with_title(params.file_path.clone());
        Ok(match recorded {
            Some(recorded) => output.with_metadata(metadata_for_writes(&[recorded])),
            None => output,
        })
    }
}

/// Generate a compact diff: "42- old" / "42+ new" (max 20 lines)
fn generate_diff_summary(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut output = String::new();
    let mut lines_shown = 0;
    const MAX_LINES: usize = 20;

    let mut old_line = 1usize;
    let mut new_line = 1usize;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                old_line += 1;
                new_line += 1;
                continue;
            }
            ChangeTag::Delete => {
                let content = change.value().trim();
                old_line += 1;
                if content.is_empty() {
                    continue;
                }
                if lines_shown >= MAX_LINES {
                    output.push_str("...\n");
                    break;
                }
                output.push_str(&format!("{}- {}\n", old_line - 1, content));
                lines_shown += 1;
            }
            ChangeTag::Insert => {
                let content = change.value().trim();
                new_line += 1;
                if content.is_empty() {
                    continue;
                }
                if lines_shown >= MAX_LINES {
                    output.push_str("...\n");
                    break;
                }
                output.push_str(&format!("{}+ {}\n", new_line - 1, content));
                lines_shown += 1;
            }
        }
    }

    output.trim_end().to_string()
}

fn build_file_touch_preview(diff: &str) -> Option<String> {
    let trimmed = diff.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut lines = trimmed.lines();
    let mut preview = lines
        .by_ref()
        .take(FILE_TOUCH_PREVIEW_MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let mut truncated = lines.next().is_some();

    if preview.len() > FILE_TOUCH_PREVIEW_MAX_BYTES {
        preview = crate::util::truncate_str(&preview, FILE_TOUCH_PREVIEW_MAX_BYTES)
            .trim_end()
            .to_string();
        truncated = true;
    }

    if truncated {
        preview.push_str("\n…");
    }

    Some(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_diff_summary_single_change() {
        let old = "hello world";
        let new = "hello rust";
        let diff = generate_diff_summary(old, new);

        // Compact format: "1- content" / "1+ content"
        assert!(diff.contains("1- hello world"), "Should show deleted line");
        assert!(diff.contains("1+ hello rust"), "Should show added line");
    }

    #[test]
    fn test_generate_diff_summary_multi_line() {
        let old = "line one\nline two\nline three";
        let new = "line one\nchanged two\nline three";
        let diff = generate_diff_summary(old, new);

        assert!(diff.contains("2- line two"), "Should show deleted line");
        assert!(diff.contains("2+ changed two"), "Should show added line");
        // Equal lines should not appear
        assert!(
            !diff.contains("line one"),
            "Should not show unchanged lines"
        );
    }

    #[test]
    fn test_generate_diff_summary_new_file() {
        let old = "";
        let new = "line one\nline two\nline three";
        let diff = generate_diff_summary(old, new);

        assert!(diff.contains("1+ line one"), "Should show line 1 added");
        assert!(diff.contains("2+ line two"), "Should show line 2 added");
        assert!(diff.contains("3+ line three"), "Should show line 3 added");
    }

    #[test]
    fn test_generate_diff_summary_truncation() {
        // Create old and new with more than 20 changed lines
        let old = (1..=25)
            .map(|i| format!("old line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (1..=25)
            .map(|i| format!("new line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let diff = generate_diff_summary(&old, &new);

        assert!(diff.contains("..."), "Should truncate after 20 lines");
    }

    #[test]
    fn test_generate_diff_summary_line_number_format() {
        let old = "old";
        let new = "new";
        let diff = generate_diff_summary(old, new);

        // Compact format: no padding
        assert!(
            diff.contains("1- old"),
            "Should have line number directly before minus"
        );
        assert!(
            diff.contains("1+ new"),
            "Should have line number directly before plus"
        );
    }

    #[test]
    fn test_generate_diff_summary_empty_result() {
        let old = "same content";
        let new = "same content";
        let diff = generate_diff_summary(old, new);

        assert!(diff.is_empty(), "No changes should produce empty diff");
    }
}
