use std::collections::BTreeMap;
use std::fs::Permissions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use jcode_lsp::{TextEdit, apply_text_edits};
use serde_json::{Value, json};
use tempfile::{Builder, TempPath};

use super::ToolContext;
use super::file_write_guard::{FileWriteGuard, GuardedFile, RequiredCoverage};
use crate::bus::{Bus, BusEvent, FileOp, FileTouch};
use crate::server::{FileSnapshotLedger, SnapshotWrite};

#[derive(Debug)]
pub(crate) struct RenameApplyResult {
    pub(crate) file_count: usize,
    pub(crate) edit_count: usize,
    pub(crate) metadata: Value,
}

struct RenameTarget {
    relative_path: String,
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
    guarded: GuardedFile,
    edit_count: usize,
    staged_replacement: Option<TempPath>,
    staged_rollback: Option<TempPath>,
}

pub(crate) async fn apply_workspace_edit(
    edit: &Value,
    root: &Path,
    ctx: &ToolContext,
    ledger: FileSnapshotLedger,
    intent: String,
) -> Result<RenameApplyResult> {
    apply_workspace_edit_for_operation(edit, root, ctx, ledger, intent, "semantic rename").await
}

pub(crate) async fn apply_workspace_edit_for_operation(
    edit: &Value,
    root: &Path,
    ctx: &ToolContext,
    ledger: FileSnapshotLedger,
    intent: String,
    operation: &str,
) -> Result<RenameApplyResult> {
    let edits = collect_workspace_edits(edit)?;
    if edits.is_empty() {
        bail!("language server returned an empty workspace edit");
    }

    let strict_policy = crate::config::ReadGuardConfig {
        mode: crate::config::ReadGuardMode::Block,
        require_same_revision: true,
        require_covered_ranges: true,
        allow_full_file_write: true,
    };
    let write_guard = FileWriteGuard::with_policy(ledger.clone(), strict_policy);
    let transaction = write_guard.begin(ctx).await?;
    let mut targets = Vec::with_capacity(edits.len());

    for (uri, edits) in edits {
        let path = uri_to_workspace_path(&uri, root)?;
        let relative_path = path
            .strip_prefix(root)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("failed to inspect rename target {relative_path}"))?;
        if !metadata.is_file() {
            bail!("rename target is not a file: {relative_path}");
        }
        let original = std::fs::read(&path)
            .with_context(|| format!("failed to read rename target {relative_path}"))?;
        let original_text = std::str::from_utf8(&original)
            .with_context(|| format!("rename target is not UTF-8 text: {relative_path}"))?;
        let guarded = transaction
            .preflight_existing(&path, &original, RequiredCoverage::FullFile)
            .await?;
        let plan = apply_text_edits(original_text, &edits)?;
        let replacement = plan.updated.into_bytes();
        let permissions = metadata.permissions();
        targets.push(RenameTarget {
            relative_path,
            path: path.clone(),
            original: original.clone(),
            replacement: replacement.clone(),
            guarded,
            edit_count: plan.edit_count,
            staged_replacement: Some(stage_file(&path, &replacement, &permissions)?),
            staged_rollback: Some(stage_file(&path, &original, &permissions)?),
        });
    }

    for target in &targets {
        let live = std::fs::read(&target.path)?;
        if live != target.original {
            bail!(
                "rename target changed after LSP preview: {}\nNo bytes were written.",
                target.relative_path
            );
        }
    }

    publish_all(&mut targets)?;
    let writes = targets
        .iter()
        .map(|target| SnapshotWrite {
            relative_path: target.relative_path.clone(),
            expected_revision: target
                .guarded
                .revision_before
                .clone()
                .expect("existing rename target must have a revision"),
            contents: target.replacement.clone(),
            mtime_ns: std::fs::metadata(&target.path)
                .ok()
                .and_then(|metadata| modified_ns(&metadata)),
        })
        .collect();
    let records = match ledger.record_writes(&ctx.session_id, root, writes).await {
        Ok(records) => records,
        Err(error) => {
            let published = targets.len();
            let rollback = rollback_prefix(&mut targets, published);
            let suffix = rollback
                .err()
                .map(|rollback| format!("; rollback also failed: {rollback}"))
                .unwrap_or_default();
            bail!("semantic rename ledger update failed: {error}{suffix}");
        }
    };

    let intent = Some(intent).filter(|value| !value.trim().is_empty());
    let edit_count = targets.iter().map(|target| target.edit_count).sum();
    let files = targets
        .iter()
        .zip(records.iter())
        .map(|(target, record)| {
            Bus::global().publish(BusEvent::FileTouch(FileTouch {
                session_id: ctx.session_id.clone(),
                path: target.path.clone(),
                op: FileOp::Edit,
                intent: intent.clone(),
                summary: Some(format!("LSP {operation} ({} edits)", target.edit_count)),
                detail: None,
            }));
            json!({
                "path": target.relative_path,
                "revision_before": target.guarded.revision_before,
                "revision_after": record.revision,
                "writer_session_id": record.writer_session_id,
                "edit_count": target.edit_count
            })
        })
        .collect::<Vec<_>>();

    let mut metadata = json!({
        "files": files,
        "workspace_edit_applied": true,
        "operation": operation
    });
    if operation == "semantic rename" {
        metadata["rename_applied"] = json!(true);
    }
    Ok(RenameApplyResult {
        file_count: targets.len(),
        edit_count,
        metadata,
    })
}

fn collect_workspace_edits(edit: &Value) -> Result<BTreeMap<String, Vec<TextEdit>>> {
    let mut collected = BTreeMap::<String, Vec<TextEdit>>::new();
    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            collected
                .entry(uri.clone())
                .or_default()
                .extend(serde_json::from_value::<Vec<TextEdit>>(edits.clone())?);
        }
    }
    if let Some(changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            let Some(document) = change.get("textDocument") else {
                bail!("rename includes an unsupported resource operation");
            };
            let uri = document
                .get("uri")
                .and_then(Value::as_str)
                .context("rename document edit is missing a URI")?;
            let edits = change
                .get("edits")
                .context("rename document edit is missing edits")?;
            collected
                .entry(uri.to_owned())
                .or_default()
                .extend(serde_json::from_value::<Vec<TextEdit>>(edits.clone())?);
        }
    }
    Ok(collected)
}

fn uri_to_workspace_path(uri: &str, root: &Path) -> Result<PathBuf> {
    let path = url::Url::parse(uri)
        .with_context(|| format!("invalid rename URI: {uri}"))?
        .to_file_path()
        .map_err(|()| anyhow::anyhow!("rename URI is not a local file: {uri}"))?
        .canonicalize()?;
    if !path.starts_with(root) {
        bail!("rename edit escapes the workspace: {}", path.display());
    }
    Ok(path)
}

fn stage_file(path: &Path, contents: &[u8], permissions: &Permissions) -> Result<TempPath> {
    let parent = path
        .parent()
        .context("rename target has no parent directory")?;
    let mut temporary = Builder::new()
        .prefix(".jcode-lsp-rename-")
        .tempfile_in(parent)?;
    temporary.as_file_mut().write_all(contents)?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().set_permissions(permissions.clone())?;
    temporary.as_file().sync_all()?;
    Ok(temporary.into_temp_path())
}

fn publish_all(targets: &mut [RenameTarget]) -> Result<()> {
    for index in 0..targets.len() {
        let staged = targets[index]
            .staged_replacement
            .take()
            .context("missing staged semantic rename")?;
        if let Err(error) = staged.persist(&targets[index].path) {
            let rollback = rollback_prefix(targets, index);
            let suffix = rollback
                .err()
                .map(|rollback| format!("; rollback also failed: {rollback}"))
                .unwrap_or_default();
            bail!(
                "failed to publish semantic rename for {}: {}{}",
                targets[index].relative_path,
                error.error,
                suffix
            );
        }
    }
    Ok(())
}

fn rollback_prefix(targets: &mut [RenameTarget], published: usize) -> Result<()> {
    let mut failures = Vec::new();
    for target in targets[..published].iter_mut().rev() {
        let Some(rollback) = target.staged_rollback.take() else {
            failures.push(format!("{}: missing rollback", target.relative_path));
            continue;
        };
        if let Err(error) = rollback.persist(&target.path) {
            failures.push(format!("{}: {}", target.relative_path, error.error));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

fn modified_ns(metadata: &std::fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ReadCoverage;
    use jcode_tool_core::ToolExecutionMode;

    #[test]
    fn parses_both_workspace_edit_shapes_and_rejects_resource_operations() {
        let changes = collect_workspace_edits(&json!({
            "changes": {"file:///a.rs": [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                "newText": "x"
            }]},
            "documentChanges": [{
                "textDocument": {"uri": "file:///b.rs", "version": 1},
                "edits": [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "newText": "y"
                }]
            }]
        }))
        .unwrap();
        assert_eq!(changes.len(), 2);
        assert!(
            collect_workspace_edits(&json!({
                "documentChanges": [{"kind": "rename", "oldUri": "file:///a", "newUri": "file:///b"}]
            }))
            .is_err()
        );
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext {
            session_id: "rename-session".to_owned(),
            message_id: "message".to_owned(),
            tool_call_id: "rename-call".to_owned(),
            working_dir: Some(root.to_owned()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::Direct,
        }
    }

    async fn record_full_read(ledger: &FileSnapshotLedger, root: &Path, relative: &str) {
        let contents = std::fs::read(root.join(relative)).unwrap();
        ledger
            .record_read(
                "rename-session",
                root,
                relative,
                &contents,
                None,
                ReadCoverage {
                    ranges: Vec::new(),
                    full_file: true,
                },
            )
            .await
            .unwrap();
    }

    fn two_file_edit(root: &Path) -> Value {
        let a = url::Url::from_file_path(root.join("a.rs")).unwrap();
        let b = url::Url::from_file_path(root.join("b.rs")).unwrap();
        json!({
            "changes": {
                a.as_str(): [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                    "newText": "bar"
                }],
                b.as_str(): [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                    "newText": "bar"
                }]
            }
        })
    }

    #[tokio::test]
    async fn applies_fully_read_multi_file_rename_and_records_revisions() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.rs"), "foo one\n").unwrap();
        std::fs::write(root.path().join("b.rs"), "foo two\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.path().join("a.rs"),
                std::fs::Permissions::from_mode(0o640),
            )
            .unwrap();
        }
        let canonical = root.path().canonicalize().unwrap();
        let ledger = FileSnapshotLedger::new();
        record_full_read(&ledger, &canonical, "a.rs").await;
        record_full_read(&ledger, &canonical, "b.rs").await;

        let result = apply_workspace_edit(
            &two_file_edit(&canonical),
            &canonical,
            &context(&canonical),
            ledger,
            "Rename foo to bar".to_owned(),
        )
        .await
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.edit_count, 2);
        assert_eq!(
            std::fs::read_to_string(canonical.join("a.rs")).unwrap(),
            "bar one\n"
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("b.rs")).unwrap(),
            "bar two\n"
        );
        assert_eq!(result.metadata["files"].as_array().unwrap().len(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(canonical.join("a.rs"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
        }
    }

    #[tokio::test]
    async fn applies_code_action_edit_through_the_same_snapshot_transaction() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.rs"), "let value = 1;\n").unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let ledger = FileSnapshotLedger::new();
        record_full_read(&ledger, &canonical, "a.rs").await;
        let uri = url::Url::from_file_path(canonical.join("a.rs")).unwrap();
        let edit = json!({
            "changes": {
                uri.as_str(): [{
                    "range": {
                        "start": {"line": 0, "character": 4},
                        "end": {"line": 0, "character": 9}
                    },
                    "newText": "renamed"
                }]
            }
        });

        let result = apply_workspace_edit_for_operation(
            &edit,
            &canonical,
            &context(&canonical),
            ledger,
            "Apply selected quick fix".to_owned(),
            "code action",
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(canonical.join("a.rs")).unwrap(),
            "let renamed = 1;\n"
        );
        assert_eq!(result.metadata["operation"], "code action");
        assert_eq!(result.metadata["workspace_edit_applied"], true);
        assert!(result.metadata.get("rename_applied").is_none());
    }

    #[tokio::test]
    async fn stale_file_rejects_entire_rename_before_any_write() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.rs"), "foo one\n").unwrap();
        std::fs::write(root.path().join("b.rs"), "foo two\n").unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let ledger = FileSnapshotLedger::new();
        record_full_read(&ledger, &canonical, "a.rs").await;
        record_full_read(&ledger, &canonical, "b.rs").await;
        std::fs::write(canonical.join("b.rs"), "peer change\n").unwrap();

        let error = apply_workspace_edit(
            &two_file_edit(&canonical),
            &canonical,
            &context(&canonical),
            ledger,
            "Rename foo to bar".to_owned(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Overwrite rejected"));
        assert_eq!(
            std::fs::read_to_string(canonical.join("a.rs")).unwrap(),
            "foo one\n"
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("b.rs")).unwrap(),
            "peer change\n"
        );
    }
}
