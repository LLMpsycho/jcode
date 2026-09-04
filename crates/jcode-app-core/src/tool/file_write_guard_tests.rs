use super::file_write_guard::FileWriteGuard;
use super::{Tool, ToolContext, ToolExecutionMode};
use crate::config::{ReadGuardConfig, ReadGuardMode};
use crate::server::{FileSnapshotLedger, ReadCoverage};
use jcode_agent_runtime::InterruptSignal;
use serde_json::{Value, json};
use std::path::Path;

fn context(workspace: &Path, session_id: &str) -> ToolContext {
    ToolContext {
        session_id: session_id.to_owned(),
        message_id: "message".into(),
        tool_call_id: "call".into(),
        working_dir: Some(workspace.to_path_buf()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None::<InterruptSignal>,
        execution_mode: ToolExecutionMode::Direct,
    }
}

fn policy(mode: ReadGuardMode) -> ReadGuardConfig {
    ReadGuardConfig {
        mode,
        allow_full_file_write: true,
        ..ReadGuardConfig::default()
    }
}

fn assert_revision_metadata(metadata: Option<Value>, path: &str, before_is_null: bool) {
    let metadata = metadata.expect("write tools should return ledger metadata");
    let file = &metadata["files"][0];
    assert_eq!(file["path"], path);
    assert_eq!(file["revision_before"].is_null(), before_is_null);
    assert!(file["revision_after"]["revision"].as_u64().is_some());
    assert_eq!(file["writer_session_id"], "writer");
}

#[tokio::test]
async fn block_mode_rejects_peer_stale_edit_without_writing() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("shared.txt");
    std::fs::write(&path, "base\n").unwrap();
    let ledger = FileSnapshotLedger::new();
    ledger
        .record_read(
            "writer",
            workspace.path(),
            "shared.txt",
            b"base\n",
            None,
            ReadCoverage {
                ranges: Vec::new(),
                full_file: true,
            },
        )
        .await
        .unwrap();
    std::fs::write(&path, "peer\n").unwrap();
    ledger
        .record_write("peer", workspace.path(), "shared.txt", b"peer\n", None)
        .await
        .unwrap();
    let tool = super::edit::EditTool::with_write_guard(FileWriteGuard::with_policy(
        ledger,
        policy(ReadGuardMode::Block),
    ));

    let error = tool
        .execute(
            json!({"file_path": "shared.txt", "old_string": "peer", "new_string": "writer"}),
            context(workspace.path(), "writer"),
        )
        .await
        .expect_err("stale peer edit should be blocked");

    assert_eq!(std::fs::read_to_string(path).unwrap(), "peer\n");
    assert!(error.to_string().contains("peer peer"));
    assert!(error.to_string().contains("No bytes were written"));
}

#[tokio::test]
async fn block_mode_rejects_edit_outside_read_coverage() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("partial.txt");
    std::fs::write(&path, "one\ntwo\n").unwrap();
    let ledger = FileSnapshotLedger::new();
    ledger
        .record_read(
            "writer",
            workspace.path(),
            "partial.txt",
            b"one\ntwo\n",
            None,
            ReadCoverage {
                ranges: vec![jcode_edit_core::LineRange { start: 1, end: 1 }],
                full_file: false,
            },
        )
        .await
        .unwrap();
    let tool = super::edit::EditTool::with_write_guard(FileWriteGuard::with_policy(
        ledger,
        policy(ReadGuardMode::Block),
    ));

    let error = tool
        .execute(
            json!({"file_path": "partial.txt", "old_string": "two", "new_string": "changed"}),
            context(workspace.path(), "writer"),
        )
        .await
        .expect_err("uncovered edit should be blocked");

    assert!(error.to_string().contains("lines 2-2 were not covered"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "one\ntwo\n");
}

#[tokio::test]
async fn new_file_write_records_revision_without_read_guard_warning() {
    let workspace = tempfile::tempdir().unwrap();
    let ledger = FileSnapshotLedger::new();
    let tool = super::write::WriteTool::with_write_guard(FileWriteGuard::with_policy(
        ledger.clone(),
        policy(ReadGuardMode::Block),
    ));

    let output = tool
        .execute(
            json!({"file_path": "new.txt", "content": "created\n"}),
            context(workspace.path(), "writer"),
        )
        .await
        .unwrap();

    assert!(!output.output.contains("overwrite guard"));
    assert_revision_metadata(output.metadata, "new.txt", true);
    let snapshot = ledger
        .snapshot(workspace.path(), "new.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.revision.revision, 1);
    assert_eq!(snapshot.writer_session_id.as_deref(), Some("writer"));
}

#[tokio::test]
async fn exact_edit_fallback_remains_operational_in_default_warn_rollout() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("exact.txt");
    std::fs::write(&path, "before\n").unwrap();
    let tool = super::edit::EditTool::with_write_guard(FileWriteGuard::with_policy(
        FileSnapshotLedger::new(),
        policy(ReadGuardMode::Warn),
    ));

    let output = tool
        .execute(
            json!({"file_path": "exact.txt", "old_string": "before", "new_string": "after"}),
            context(workspace.path(), "writer"),
        )
        .await
        .unwrap();

    assert_eq!(std::fs::read_to_string(path).unwrap(), "after\n");
    assert!(output.output.contains("was not read in this session"));
    assert_revision_metadata(output.metadata, "exact.txt", false);
}

#[tokio::test]
async fn legacy_mutation_tools_return_consistent_revision_metadata() {
    let workspace = tempfile::tempdir().unwrap();
    for (path, contents) in [
        ("edit.txt", "old\n"),
        ("multi.txt", "one\ntwo\n"),
        ("patch.txt", "old\n"),
        ("apply.txt", "old\n"),
    ] {
        std::fs::write(workspace.path().join(path), contents).unwrap();
    }
    let guard = FileWriteGuard::with_policy(FileSnapshotLedger::new(), policy(ReadGuardMode::Warn));

    let edit = super::edit::EditTool::with_write_guard(guard.clone());
    let multi = super::multiedit::MultiEditTool::with_write_guard(guard.clone());
    let patch = super::patch::PatchTool::with_write_guard(guard.clone());
    let apply = super::apply_patch::ApplyPatchTool::with_write_guard(guard);

    let outputs = [
        edit.execute(
            json!({"file_path": "edit.txt", "old_string": "old", "new_string": "new"}),
            context(workspace.path(), "writer"),
        )
        .await
        .unwrap(),
        multi
            .execute(
                json!({"file_path": "multi.txt", "edits": [{"old_string": "one", "new_string": "ONE"}]}),
                context(workspace.path(), "writer"),
            )
            .await
            .unwrap(),
        patch
            .execute(
                json!({"patch_text": "--- a/patch.txt\n+++ b/patch.txt\n@@ -1,1 +1,1 @@\n-old\n+new"}),
                context(workspace.path(), "writer"),
            )
            .await
            .unwrap(),
        apply
            .execute(
                json!({"patch_text": "*** Begin Patch\n*** Update File: apply.txt\n@@\n-old\n+new\n*** End Patch"}),
                context(workspace.path(), "writer"),
            )
            .await
            .unwrap(),
    ];

    for (output, path) in
        outputs
            .into_iter()
            .zip(["edit.txt", "multi.txt", "patch.txt", "apply.txt"])
    {
        assert_revision_metadata(output.metadata, path, false);
    }
}

#[tokio::test]
async fn apply_patch_preflight_failure_leaves_earlier_file_unchanged() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("first.txt"), "old\n").unwrap();
    std::fs::write(workspace.path().join("second.txt"), "actual\n").unwrap();
    let tool = super::apply_patch::ApplyPatchTool::with_write_guard(FileWriteGuard::with_policy(
        FileSnapshotLedger::new(),
        policy(ReadGuardMode::Warn),
    ));
    let error = tool
        .execute(
            json!({"patch_text": "*** Begin Patch\n*** Update File: first.txt\n@@\n-old\n+new\n*** Update File: second.txt\n@@\n-missing\n+new\n*** End Patch"}),
            context(workspace.path(), "writer"),
        )
        .await
        .expect_err("all hunks should preflight before publication");

    assert!(error.to_string().contains("Failed to find expected lines"));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("first.txt")).unwrap(),
        "old\n"
    );
}

#[tokio::test]
async fn unified_patch_preflight_failure_leaves_earlier_file_unchanged() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("first.txt"), "old\n").unwrap();
    let tool = super::patch::PatchTool::with_write_guard(FileWriteGuard::with_policy(
        FileSnapshotLedger::new(),
        policy(ReadGuardMode::Warn),
    ));
    let error = tool
        .execute(
            json!({"patch_text": "--- a/first.txt\n+++ b/first.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n--- a/missing.txt\n+++ b/missing.txt\n@@ -1,1 +1,1 @@\n-old\n+new"}),
            context(workspace.path(), "writer"),
        )
        .await
        .expect_err("all files should preflight before publication");

    assert!(error.to_string().contains("file does not exist"));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("first.txt")).unwrap(),
        "old\n"
    );
}
