use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, FileOp, FileTouch};
use crate::server::{FileSnapshotLedger, SnapshotWrite};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use jcode_edit_core::{display_tag_hex, parse_anchored_edit, preflight_plan};
use jcode_edit_types::ObservedFile;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::Permissions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tempfile::{Builder, TempPath};

const DIFF_DETAIL_MAX_BYTES: usize = 800;

pub struct AnchoredEditTool {
    file_snapshots: FileSnapshotLedger,
}

impl AnchoredEditTool {
    pub(crate) fn new(file_snapshots: FileSnapshotLedger) -> Self {
        Self { file_snapshots }
    }
}

#[derive(Deserialize)]
struct AnchoredEditInput {
    intent: String,
    input: String,
}

struct TargetFile {
    relative_path: String,
    canonical_path: PathBuf,
    original: Vec<u8>,
    permissions: Permissions,
    observed_revision: jcode_edit_types::FileRevision,
    replacement: Vec<u8>,
    staged_replacement: Option<TempPath>,
    staged_rollback: Option<TempPath>,
}

#[async_trait]
impl Tool for AnchoredEditTool {
    fn capability(&self, _input: &serde_json::Value) -> crate::tool::ToolCapability {
        crate::tool::ToolCapability::WriteFiles
    }

    fn name(&self) -> &str {
        "anchored_edit"
    }

    fn description(&self) -> &str {
        "Apply a strict, read-anchored, stale-safe multi-file edit."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["intent", "input"],
            "properties": {
                "intent": super::intent_schema_property(),
                "input": {
                    "type": "string",
                    "description": "Strict anchored-edit document. Sections use [relative/path#ABCD], followed by PUT, CUT, REM, or MV commands."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: AnchoredEditInput = serde_json::from_value(input)?;
        let parsed =
            parse_anchored_edit(&params.input).map_err(|error| rejected(error.to_string()))?;
        let workspace_root = canonical_workspace_root(&ctx)?;
        let _transaction = self.file_snapshots.lock_write_transaction().await;

        let mut targets = resolve_targets(&workspace_root, &parsed.files)?;
        let mut observed_files = Vec::with_capacity(targets.len());
        let mut read_snapshots = Vec::with_capacity(targets.len());

        for target in &mut targets {
            let metadata = std::fs::metadata(&target.canonical_path).with_context(|| {
                format!("failed to inspect {}", target.canonical_path.display())
            })?;
            target.original = std::fs::read(&target.canonical_path)
                .with_context(|| format!("failed to read {}", target.canonical_path.display()))?;
            target.permissions = metadata.permissions();
            let record = self
                .file_snapshots
                .observe_text(
                    &workspace_root,
                    &target.relative_path,
                    &target.original,
                    modified_ns(&metadata),
                )
                .await
                .map_err(|error| rejected(error.to_string()))?;
            target.observed_revision = record.revision.clone();
            observed_files.push(ObservedFile {
                path: target.relative_path.clone(),
                revision: record.revision.revision,
                contents: target.original.clone(),
                mtime_ns: modified_ns(&metadata),
            });
            if let Some(read) = self
                .file_snapshots
                .session_read(&ctx.session_id, &workspace_root, &target.relative_path)
                .await?
            {
                read_snapshots.push(read);
            }
        }

        let plan = preflight_plan(&params.input, &observed_files, &read_snapshots)
            .map_err(|error| rejected(error.to_string()))?;
        for (target, planned) in targets.iter_mut().zip(plan.files.iter()) {
            target.replacement = planned.contents.clone();
            target.staged_replacement = Some(stage_file(
                &target.canonical_path,
                &target.replacement,
                &target.permissions,
            )?);
            target.staged_rollback = Some(stage_file(
                &target.canonical_path,
                &target.original,
                &target.permissions,
            )?);
        }

        // Catch filesystem changes that occurred while staging. This happens
        // before the first target path is replaced, preserving zero-write
        // rejection for stale multi-file requests.
        for target in &targets {
            let live = std::fs::read(&target.canonical_path).with_context(|| {
                format!("failed to revalidate {}", target.canonical_path.display())
            })?;
            if live != target.original {
                let metadata = std::fs::metadata(&target.canonical_path)?;
                if self
                    .file_snapshots
                    .observe_text(
                        &workspace_root,
                        &target.relative_path,
                        &live,
                        modified_ns(&metadata),
                    )
                    .await
                    .is_err()
                {
                    crate::logging::warn(
                        "Stale anchored edit rejected; latest snapshot could not be recorded",
                    );
                }
                return Err(rejected(format!(
                    "{} changed while the edit was being staged",
                    target.relative_path
                )));
            }
        }

        publish_all(&mut targets)?;

        let writes = targets
            .iter()
            .map(|target| SnapshotWrite {
                relative_path: target.relative_path.clone(),
                expected_revision: target.observed_revision.clone(),
                contents: target.replacement.clone(),
                mtime_ns: match std::fs::metadata(&target.canonical_path) {
                    Ok(metadata) => modified_ns(&metadata),
                    Err(_) => {
                        crate::logging::warn("Published anchored edit metadata unavailable; retaining content revision");
                        None
                    }
                },
            })
            .collect();
        let records = match self
            .file_snapshots
            .record_writes(&ctx.session_id, &workspace_root, writes)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                let rollback = rollback_published(&mut targets);
                let suffix = rollback
                    .err()
                    .map(|rollback| format!("; rollback also failed: {rollback}"))
                    .unwrap_or_else(String::new);
                bail!("anchored edit ledger update failed: {error}{suffix}");
            }
        };

        let intent = Some(params.intent).filter(|value| !value.trim().is_empty());
        let mut output = format!("Applied anchored edit to {} file(s):", targets.len());
        let mut metadata = Vec::with_capacity(targets.len());
        for ((target, planned), record) in targets.iter().zip(plan.files.iter()).zip(records.iter())
        {
            let before_tag = display_tag_hex(planned.revision_before.display_tag);
            let after_tag = display_tag_hex(record.revision.display_tag);
            output.push_str(&format!(
                "\n- {}: rev {} #{} -> rev {} #{}",
                target.relative_path,
                planned.revision_before.revision,
                before_tag,
                record.revision.revision,
                after_tag
            ));
            let detail = compact_diff(&target.original, &target.replacement);
            Bus::global().publish(BusEvent::FileTouch(FileTouch {
                session_id: ctx.session_id.clone(),
                path: target.canonical_path.clone(),
                op: FileOp::Edit,
                intent: intent.clone(),
                summary: Some(format!(
                    "anchored edit rev {} -> {}",
                    planned.revision_before.revision, record.revision.revision
                )),
                detail: Some(detail),
            }));
            metadata.push(json!({
                "path": target.relative_path,
                "revision_before": planned.revision_before,
                "revision_after": record.revision,
                "writer_session_id": record.writer_session_id,
            }));
        }

        Ok(ToolOutput::new(output).with_metadata(json!({ "files": metadata })))
    }
}

fn canonical_workspace_root(ctx: &ToolContext) -> Result<PathBuf> {
    let root = ctx
        .working_dir
        .as_deref()
        .context("anchored_edit requires a workspace working directory")?;
    let canonical = std::fs::canonicalize(root)
        .with_context(|| format!("failed to canonicalize workspace {}", root.display()))?;
    if !canonical.is_dir() {
        bail!("workspace is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

fn resolve_targets(
    workspace_root: &Path,
    files: &[jcode_edit_types::FileEdit],
) -> Result<Vec<TargetFile>> {
    let mut canonical_paths = HashSet::with_capacity(files.len());
    files
        .iter()
        .map(|file| {
            let candidate = workspace_root.join(&file.path);
            let canonical_path = std::fs::canonicalize(&candidate)
                .with_context(|| format!("failed to canonicalize {}", file.path))?;
            if !canonical_path.starts_with(workspace_root) {
                bail!("anchored edit path escapes the workspace: {}", file.path);
            }
            let canonical_relative = canonical_path
                .strip_prefix(workspace_root)?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if canonical_relative != file.path {
                bail!(
                    "anchored edit path is not canonical: {} (use {})",
                    file.path,
                    canonical_relative
                );
            }
            if !canonical_paths.insert(canonical_path.clone()) {
                bail!("multiple sections resolve to {}", canonical_path.display());
            }
            let metadata = std::fs::metadata(&canonical_path)?;
            if !metadata.is_file() {
                bail!("anchored edit target is not a file: {}", file.path);
            }
            Ok(TargetFile {
                relative_path: file.path.clone(),
                canonical_path,
                original: Vec::new(),
                permissions: metadata.permissions(),
                observed_revision: jcode_edit_core::file_revision(0, "", None),
                replacement: Vec::new(),
                staged_replacement: None,
                staged_rollback: None,
            })
        })
        .collect()
}

fn stage_file(path: &Path, contents: &[u8], permissions: &Permissions) -> Result<TempPath> {
    let parent = path
        .parent()
        .with_context(|| format!("file has no parent directory: {}", path.display()))?;
    let mut temporary = Builder::new()
        .prefix(".jcode-anchored-")
        .tempfile_in(parent)
        .with_context(|| format!("failed to stage edit beside {}", path.display()))?;
    temporary.as_file_mut().write_all(contents)?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().set_permissions(permissions.clone())?;
    temporary.as_file().sync_all()?;
    Ok(temporary.into_temp_path())
}

fn publish_all(targets: &mut [TargetFile]) -> Result<()> {
    for index in 0..targets.len() {
        let staged = targets[index]
            .staged_replacement
            .take()
            .context("missing staged anchored edit")?;
        if let Err(error) = staged.persist(&targets[index].canonical_path) {
            let rollback = rollback_prefix(targets, index);
            let suffix = rollback
                .err()
                .map(|rollback| format!("; rollback also failed: {rollback}"))
                .unwrap_or_else(String::new);
            bail!(
                "failed to publish {}: {}{}",
                targets[index].relative_path,
                error.error,
                suffix
            );
        }
    }
    Ok(())
}

fn rollback_prefix(targets: &mut [TargetFile], published: usize) -> Result<()> {
    let mut failures = Vec::new();
    for target in targets[..published].iter_mut().rev() {
        let Some(rollback) = target.staged_rollback.take() else {
            failures.push(format!("{}: missing staged rollback", target.relative_path));
            continue;
        };
        if let Err(error) = rollback.persist(&target.canonical_path) {
            failures.push(format!("{}: {}", target.relative_path, error.error));
        }
    }
    if !failures.is_empty() {
        bail!("{}", failures.join("; "));
    }
    Ok(())
}

fn rollback_published(targets: &mut [TargetFile]) -> Result<()> {
    let published = targets.len();
    rollback_prefix(targets, published)
}

fn modified_ns(metadata: &std::fs::Metadata) -> Option<u128> {
    match metadata.modified() {
        Ok(modified) => match modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => Some(duration.as_nanos()),
            Err(_) => None, // Pre-epoch timestamps cannot be represented; content revisions still guard writes.
        },
        Err(_) => {
            crate::logging::debug(
                "File modification time unavailable; using content revision only",
            );
            None
        }
    }
}

fn compact_diff(before: &[u8], after: &[u8]) -> String {
    let before = String::from_utf8_lossy(before);
    let after = String::from_utf8_lossy(after);
    let diff = similar::TextDiff::from_lines(&before, &after)
        .unified_diff()
        .context_radius(2)
        .to_string();
    crate::util::truncate_str(&diff, DIFF_DETAIL_MAX_BYTES).to_string()
}

fn rejected(reason: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "Anchored edit rejected: {reason}. Reread the affected file or range; no bytes were written."
    )
}

#[cfg(test)]
mod tests;
