use std::collections::BTreeMap;
#[cfg(unix)]
use std::ffi::CString;
use std::fs::Permissions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use jcode_lsp::{TextEdit, apply_text_edits};
use serde_json::{Value, json};
use tempfile::{Builder, TempPath};

use super::ToolContext;
use super::file_write_guard::{FileWriteGuard, GuardedFile, RequiredCoverage};
use crate::bus::{Bus, BusEvent, FileOp, FileTouch};
use crate::server::{FileSnapshotLedger, SnapshotMove, SnapshotWrite};

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
    expected_revision: jcode_edit_types::FileRevision,
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
        let expected_revision = guarded
            .revision_before
            .clone()
            .context("existing rename target has no recorded revision; no bytes were written")?;
        targets.push(RenameTarget {
            relative_path,
            path: path.clone(),
            original: original.clone(),
            replacement: replacement.clone(),
            guarded,
            expected_revision,
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
            expected_revision: target.expected_revision.clone(),
            contents: target.replacement.clone(),
            mtime_ns: std::fs::metadata(&target.path).map_or_else(
                |_| {
                    crate::logging::warn(
                        "Renamed file metadata unavailable; retaining content revision",
                    );
                    None
                },
                |metadata| modified_ns(&metadata),
            ),
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
                .unwrap_or_else(String::new);
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

pub(crate) async fn apply_workspace_edit_and_file_rename(
    edit: &Value,
    source_path: &Path,
    destination_path: &Path,
    root: &Path,
    ctx: &ToolContext,
    ledger: FileSnapshotLedger,
    intent: String,
) -> Result<RenameApplyResult> {
    let risk_context = jcode_command_risk::RiskContext::from_env(ctx.working_dir.clone());
    if jcode_command_risk::is_catastrophic_target(source_path, &risk_context) {
        bail!(
            "file rename source is protected and must never be removed by an agent: {}",
            source_path.display()
        );
    }
    if path_entry_exists(destination_path)? {
        bail!(
            "file rename destination already exists: {}",
            destination_path.display()
        );
    }
    let destination_parent = destination_path
        .parent()
        .context("file rename destination has no parent directory")?
        .canonicalize()?;
    if !destination_parent.starts_with(root) {
        bail!("file rename destination escapes the workspace");
    }

    let strict_policy = crate::config::ReadGuardConfig {
        mode: crate::config::ReadGuardMode::Block,
        require_same_revision: true,
        require_covered_ranges: true,
        allow_full_file_write: true,
    };
    let write_guard = FileWriteGuard::with_policy(ledger.clone(), strict_policy);
    let transaction = write_guard.begin(ctx).await?;
    let source_metadata = std::fs::metadata(source_path)
        .with_context(|| format!("failed to inspect rename source {}", source_path.display()))?;
    if !source_metadata.is_file() {
        bail!(
            "file rename source is not a file: {}",
            source_path.display()
        );
    }
    let source_original = std::fs::read(source_path)?;
    let source_guard = transaction
        .preflight_existing(source_path, &source_original, RequiredCoverage::FullFile)
        .await?;
    let source_revision = source_guard
        .revision_before
        .clone()
        .context("existing file rename source has no recorded revision; no bytes were written")?;
    let destination_guard = transaction.prepare_new(destination_path)?;

    let mut targets = Vec::new();
    for (uri, edits) in collect_workspace_edits(edit)? {
        let path = uri_to_workspace_path(&uri, root)?;
        let relative_path = path
            .strip_prefix(root)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("failed to inspect file rename edit {relative_path}"))?;
        if !metadata.is_file() {
            bail!("file rename edit target is not a file: {relative_path}");
        }
        let original = std::fs::read(&path)?;
        let original_text = std::str::from_utf8(&original)
            .with_context(|| format!("file rename edit target is not UTF-8: {relative_path}"))?;
        let guarded = if path == source_path {
            source_guard.clone()
        } else {
            transaction
                .preflight_existing(&path, &original, RequiredCoverage::FullFile)
                .await?
        };
        let plan = apply_text_edits(original_text, &edits)?;
        let replacement = plan.updated.into_bytes();
        let expected_revision = guarded
            .revision_before
            .clone()
            .context("existing rename target has no recorded revision; no bytes were written")?;
        targets.push(RenameTarget {
            relative_path,
            path: path.clone(),
            original: original.clone(),
            replacement: replacement.clone(),
            guarded,
            expected_revision,
            edit_count: plan.edit_count,
            staged_replacement: Some(stage_file(&path, &replacement, &metadata.permissions())?),
            staged_rollback: Some(stage_file(&path, &original, &metadata.permissions())?),
        });
    }

    let source_replacement = targets
        .iter()
        .find(|target| target.path == source_path)
        .map(|target| target.replacement.clone())
        .unwrap_or_else(|| source_original.clone());
    if std::fs::read(source_path)? != source_original {
        bail!("file rename source changed after LSP preview\nNo bytes were written.");
    }
    for target in &targets {
        if std::fs::read(&target.path)? != target.original {
            bail!(
                "file rename edit target changed after LSP preview: {}\nNo bytes were written.",
                target.relative_path
            );
        }
    }
    if path_entry_exists(destination_path)? {
        bail!("file rename destination appeared after preview\nNo bytes were written.");
    }

    publish_all(&mut targets)?;
    if let Err(error) = rename_no_replace(source_path, destination_path) {
        let published = targets.len();
        let rollback = rollback_prefix(&mut targets, published);
        let suffix = rollback
            .err()
            .map(|rollback| format!("; rollback also failed: {rollback}"))
            .unwrap_or_else(String::new);
        bail!("failed to rename file: {error}{suffix}");
    }

    let related_targets = targets
        .iter()
        .filter(|target| target.path != source_path)
        .collect::<Vec<_>>();
    let writes = related_targets
        .iter()
        .map(|target| SnapshotWrite {
            relative_path: target.relative_path.clone(),
            expected_revision: target.expected_revision.clone(),
            contents: target.replacement.clone(),
            mtime_ns: std::fs::metadata(&target.path).map_or_else(
                |_| {
                    crate::logging::warn(
                        "Renamed file metadata unavailable; retaining content revision",
                    );
                    None
                },
                |metadata| modified_ns(&metadata),
            ),
        })
        .collect();
    let movement = SnapshotMove {
        source_relative_path: source_guard.relative_path.clone(),
        expected_revision: source_revision,
        destination_relative_path: destination_guard.relative_path.clone(),
        contents: source_replacement,
        mtime_ns: std::fs::metadata(destination_path).map_or_else(
            |_| {
                crate::logging::warn(
                    "Rename destination metadata unavailable; retaining content revision",
                );
                None
            },
            |metadata| modified_ns(&metadata),
        ),
    };
    let (destination_record, write_records) = match ledger
        .record_move_with_writes(&ctx.session_id, root, movement, writes)
        .await
    {
        Ok(records) => records,
        Err(error) => {
            let rename_back = std::fs::rename(destination_path, source_path);
            let published = targets.len();
            let rollback = rename_back
                .as_ref()
                .map(|_| rollback_prefix(&mut targets, published))
                .unwrap_or_else(|rename_error| {
                    Err(anyhow::anyhow!(
                        "failed to restore source path: {rename_error}"
                    ))
                });
            let suffix = rollback
                .err()
                .map(|rollback| format!("; rollback also failed: {rollback}"))
                .unwrap_or_else(String::new);
            bail!("file rename ledger update failed: {error}{suffix}");
        }
    };

    let intent = Some(intent).filter(|value| !value.trim().is_empty());
    let source_relative = source_guard.relative_path;
    let destination_relative = destination_guard.relative_path;
    Bus::global().publish(BusEvent::FileTouch(FileTouch {
        session_id: ctx.session_id.clone(),
        path: source_path.to_owned(),
        op: FileOp::Edit,
        intent: intent.clone(),
        summary: Some(format!("LSP file rename to {destination_relative}")),
        detail: None,
    }));
    Bus::global().publish(BusEvent::FileTouch(FileTouch {
        session_id: ctx.session_id.clone(),
        path: destination_path.to_owned(),
        op: FileOp::Write,
        intent: intent.clone(),
        summary: Some(format!("LSP file rename from {source_relative}")),
        detail: None,
    }));
    let related_files = related_targets
        .iter()
        .zip(write_records.iter())
        .map(|(target, record)| {
            Bus::global().publish(BusEvent::FileTouch(FileTouch {
                session_id: ctx.session_id.clone(),
                path: target.path.clone(),
                op: FileOp::Edit,
                intent: intent.clone(),
                summary: Some(format!(
                    "LSP file rename import update ({} edits)",
                    target.edit_count
                )),
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
    let mut files = vec![json!({
        "path": destination_relative,
        "moved_from": source_relative,
        "revision_before": source_guard.revision_before,
        "revision_after": destination_record.revision,
        "writer_session_id": destination_record.writer_session_id,
        "edit_count": targets
            .iter()
            .find(|target| target.path == source_path)
            .map(|target| target.edit_count)
            .unwrap_or(0),
    })];
    files.extend(related_files);
    let edit_count = targets.iter().map(|target| target.edit_count).sum();
    Ok(RenameApplyResult {
        file_count: files.len(),
        edit_count,
        metadata: json!({
            "workspace_edit_applied": !targets.is_empty(),
            "file_rename_applied": true,
            "operation": "file rename",
            "source": {
                "path": source_relative,
                "revision_before": source_guard.revision_before,
            },
            "destination": {
                "path": destination_relative,
                "revision_after": destination_record.revision,
                "writer_session_id": destination_record.writer_session_id,
            },
            "files": files,
        }),
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
                .unwrap_or_else(String::new);
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

fn path_entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source = path_cstring(source)?;
    let destination = path_cstring(destination)?;
    // SAFETY: both paths are NUL-terminated C strings that remain alive for the call.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source = path_cstring(source)?;
    let destination = path_cstring(destination)?;
    // SAFETY: both paths are NUL-terminated C strings that remain alive for the syscall.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn path_cstring(path: &Path) -> std::io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file rename path contains an interior NUL byte",
        )
    })
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, destination)?;
    if let Err(error) = std::fs::remove_file(source) {
        if let Err(rollback_error) = std::fs::remove_file(destination) {
            return Err(std::io::Error::new(
                error.kind(),
                format!(
                    "file rename failed: {error}; destination rollback also failed: {rollback_error}"
                ),
            ));
        }
        return Err(error);
    }
    Ok(())
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

    #[cfg(unix)]
    #[tokio::test]
    async fn file_rename_rejects_a_protected_source_before_inspection() {
        let destination_parent = tempfile::tempdir().unwrap();
        let error = apply_workspace_edit_and_file_rename(
            &json!({}),
            Path::new("/"),
            &destination_parent.path().join("renamed-root"),
            Path::new("/"),
            &context(Path::new("/")),
            FileSnapshotLedger::new(),
            "Must not rename root".to_owned(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("protected"));
    }

    #[test]
    fn no_replace_rename_preserves_a_concurrent_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.rs");
        let destination = root.path().join("destination.rs");
        std::fs::write(&source, "source\n").unwrap();
        std::fs::write(&destination, "concurrent\n").unwrap();

        assert!(rename_no_replace(&source, &destination).is_err());
        assert_eq!(std::fs::read_to_string(source).unwrap(), "source\n");
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "concurrent\n"
        );
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

    #[tokio::test]
    async fn applies_file_rename_and_related_edits_with_revision_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/value.ts"),
            "export const value = 1;\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("src/main.ts"),
            "import { value } from './value';\n",
        )
        .unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let source = canonical.join("src/value.ts");
        let destination = canonical.join("src/renamed.ts");
        let ledger = FileSnapshotLedger::new();
        record_full_read(&ledger, &canonical, "src/value.ts").await;
        record_full_read(&ledger, &canonical, "src/main.ts").await;
        let importer_uri = url::Url::from_file_path(canonical.join("src/main.ts")).unwrap();
        let edit = json!({
            "changes": {
                importer_uri.as_str(): [{
                    "range": {
                        "start": {"line": 0, "character": 25},
                        "end": {"line": 0, "character": 30}
                    },
                    "newText": "renamed"
                }]
            }
        });

        let result = apply_workspace_edit_and_file_rename(
            &edit,
            &source,
            &destination,
            &canonical,
            &context(&canonical),
            ledger.clone(),
            "Rename the TypeScript module".to_owned(),
        )
        .await
        .unwrap();

        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "export const value = 1;\n"
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("src/main.ts")).unwrap(),
            "import { value } from './renamed';\n"
        );
        assert_eq!(result.file_count, 2);
        assert_eq!(result.edit_count, 1);
        assert_eq!(result.metadata["file_rename_applied"], true);
        assert_eq!(result.metadata["files"].as_array().unwrap().len(), 2);
        assert!(
            ledger
                .snapshot(&canonical, "src/value.ts")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            ledger
                .snapshot(&canonical, "src/renamed.ts")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn stale_related_edit_rejects_file_rename_before_any_mutation() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/value.ts"),
            "export const value = 1;\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("src/main.ts"),
            "import { value } from './value';\n",
        )
        .unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let source = canonical.join("src/value.ts");
        let destination = canonical.join("src/renamed.ts");
        let ledger = FileSnapshotLedger::new();
        record_full_read(&ledger, &canonical, "src/value.ts").await;
        record_full_read(&ledger, &canonical, "src/main.ts").await;
        std::fs::write(canonical.join("src/main.ts"), "// peer change\n").unwrap();
        let importer_uri = url::Url::from_file_path(canonical.join("src/main.ts")).unwrap();
        let edit = json!({
            "changes": {
                importer_uri.as_str(): [{
                    "range": {
                        "start": {"line": 0, "character": 25},
                        "end": {"line": 0, "character": 30}
                    },
                    "newText": "renamed"
                }]
            }
        });

        let error = apply_workspace_edit_and_file_rename(
            &edit,
            &source,
            &destination,
            &canonical,
            &context(&canonical),
            ledger,
            "Rename the TypeScript module".to_owned(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("Overwrite rejected"));
        assert!(source.exists());
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read_to_string(canonical.join("src/main.ts")).unwrap(),
            "// peer change\n"
        );
    }
}
