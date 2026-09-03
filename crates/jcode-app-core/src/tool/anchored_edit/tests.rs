use super::*;
use crate::message::{Message, ToolDefinition};
use crate::provider::{EventStream, Provider};
use crate::tool::Registry;
use jcode_agent_runtime::InterruptSignal;
use std::sync::Arc;

struct NoopProvider;

#[async_trait::async_trait]
impl Provider for NoopProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        anyhow::bail!("NoopProvider should not be called by anchored edit tests")
    }

    fn name(&self) -> &str {
        "noop"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self)
    }
}

fn context(workspace: &Path, session_id: &str) -> ToolContext {
    ToolContext {
        session_id: session_id.to_owned(),
        message_id: "message".into(),
        tool_call_id: "call".into(),
        working_dir: Some(workspace.to_path_buf()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None::<InterruptSignal>,
        execution_mode: super::super::ToolExecutionMode::Direct,
    }
}

async fn registry(ledger: FileSnapshotLedger) -> Registry {
    let provider: Arc<dyn Provider> = Arc::new(NoopProvider);
    Registry::new_with_file_snapshots(provider, ledger).await
}

async fn read_tag(registry: &Registry, workspace: &Path, session: &str, path: &str) -> String {
    let output = registry
        .execute(
            "read",
            json!({"file_path": path}),
            context(workspace, session),
        )
        .await
        .expect("read should succeed")
        .output;
    output
        .lines()
        .next()
        .and_then(|header| header.rsplit_once('#'))
        .and_then(|(_, suffix)| suffix.split_once(' '))
        .map(|(tag, _)| tag.to_owned())
        .expect("read should return an anchored snapshot header")
}

async fn apply(
    registry: &Registry,
    workspace: &Path,
    session: &str,
    document: String,
) -> anyhow::Result<ToolOutput> {
    registry
        .execute(
            "anchored_edit",
            json!({"intent": "test anchored edit", "input": document}),
            context(workspace, session),
        )
        .await
}

#[tokio::test]
async fn read_anchored_edit_reread_changes_tag_and_attributes_writer() {
    let workspace = tempfile::tempdir().expect("workspace");
    let path = workspace.path().join("sample.txt");
    std::fs::write(&path, "before\n").expect("initial file");
    let ledger = FileSnapshotLedger::new();
    let registry = registry(ledger.clone()).await;

    let before_tag = read_tag(&registry, workspace.path(), "writer", "sample.txt").await;
    let output = apply(
        &registry,
        workspace.path(),
        "writer",
        format!("[sample.txt#{before_tag}]\nPUT 1.=1:\n+after"),
    )
    .await
    .expect("anchored edit should succeed");
    let after_tag = read_tag(&registry, workspace.path(), "writer", "sample.txt").await;
    let snapshot = ledger
        .snapshot(workspace.path(), "sample.txt")
        .await
        .expect("snapshot query")
        .expect("snapshot exists");

    assert_eq!(std::fs::read_to_string(path).unwrap(), "after\n");
    assert_ne!(before_tag, after_tag);
    assert_eq!(snapshot.revision.revision, 2);
    assert_eq!(snapshot.writer_session_id.as_deref(), Some("writer"));
    assert!(output.output.contains("rev 1"));
    assert!(output.output.contains("rev 2"));
}

#[tokio::test]
async fn peer_stale_edit_is_rejected_without_writing() {
    let workspace = tempfile::tempdir().expect("workspace");
    let path = workspace.path().join("sample.txt");
    std::fs::write(&path, "base\n").expect("initial file");
    let ledger = FileSnapshotLedger::new();
    let registry = registry(ledger.clone()).await;

    let stale_tag = read_tag(&registry, workspace.path(), "stale", "sample.txt").await;
    let peer_tag = read_tag(&registry, workspace.path(), "peer", "sample.txt").await;
    apply(
        &registry,
        workspace.path(),
        "peer",
        format!("[sample.txt#{peer_tag}]\nPUT 1.=1:\n+peer"),
    )
    .await
    .expect("peer edit should succeed");

    let error = apply(
        &registry,
        workspace.path(),
        "stale",
        format!("[sample.txt#{stale_tag}]\nPUT 1.=1:\n+stale"),
    )
    .await
    .expect_err("stale edit should fail");

    assert_eq!(std::fs::read_to_string(path).unwrap(), "peer\n");
    assert!(error.to_string().contains("no bytes were written"));
    let snapshot = ledger
        .snapshot(workspace.path(), "sample.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.writer_session_id.as_deref(), Some("peer"));
}

#[tokio::test]
async fn partial_read_blocks_uncovered_edit_without_writing() {
    let workspace = tempfile::tempdir().expect("workspace");
    let path = workspace.path().join("sample.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").expect("initial file");
    let ledger = FileSnapshotLedger::new();
    let registry = registry(ledger).await;
    let output = registry
        .execute(
            "read",
            json!({"file_path": "sample.txt", "start_line": 1, "limit": 1}),
            context(workspace.path(), "reader"),
        )
        .await
        .expect("partial read");
    let tag = output
        .output
        .lines()
        .next()
        .and_then(|header| header.rsplit_once('#'))
        .and_then(|(_, suffix)| suffix.split_once(' '))
        .map(|(tag, _)| tag.to_owned())
        .unwrap();

    let error = apply(
        &registry,
        workspace.path(),
        "reader",
        format!("[sample.txt#{tag}]\nPUT 2.=2:\n+changed"),
    )
    .await
    .expect_err("uncovered edit should fail");

    assert_eq!(std::fs::read_to_string(path).unwrap(), "one\ntwo\nthree\n");
    assert!(error.to_string().contains("not covered"));
}

#[tokio::test]
async fn stale_file_in_multi_file_edit_leaves_every_file_unchanged() {
    let workspace = tempfile::tempdir().expect("workspace");
    let first = workspace.path().join("first.txt");
    let second = workspace.path().join("second.txt");
    std::fs::write(&first, "first\n").unwrap();
    std::fs::write(&second, "second\n").unwrap();
    let ledger = FileSnapshotLedger::new();
    let registry = registry(ledger).await;

    let first_tag = read_tag(&registry, workspace.path(), "stale", "first.txt").await;
    let second_tag = read_tag(&registry, workspace.path(), "stale", "second.txt").await;
    let peer_tag = read_tag(&registry, workspace.path(), "peer", "second.txt").await;
    apply(
        &registry,
        workspace.path(),
        "peer",
        format!("[second.txt#{peer_tag}]\nPUT 1.=1:\n+peer"),
    )
    .await
    .unwrap();

    let document = format!(
        "[first.txt#{first_tag}]\nPUT 1.=1:\n+changed-first\n[second.txt#{second_tag}]\nPUT 1.=1:\n+changed-second"
    );
    apply(&registry, workspace.path(), "stale", document)
        .await
        .expect_err("multi-file stale edit should fail");

    assert_eq!(std::fs::read_to_string(first).unwrap(), "first\n");
    assert_eq!(std::fs::read_to_string(second).unwrap(), "peer\n");
}

#[cfg(unix)]
#[tokio::test]
async fn anchored_edit_preserves_file_permissions() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let workspace = tempfile::tempdir().expect("workspace");
    let path = workspace.path().join("script.sh");
    std::fs::write(&path, "echo before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
    let registry = registry(FileSnapshotLedger::new()).await;
    let tag = read_tag(&registry, workspace.path(), "writer", "script.sh").await;

    apply(
        &registry,
        workspace.path(),
        "writer",
        format!("[script.sh#{tag}]\nPUT 1.=1:\n+echo after"),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::metadata(path).unwrap().mode() & 0o777, 0o751);
}

#[tokio::test]
async fn injected_registry_keeps_exact_edit_available() {
    let registry = registry(FileSnapshotLedger::new()).await;
    let names = registry.tool_names().await;

    assert!(names.iter().any(|name| name == "anchored_edit"));
    assert!(names.iter().any(|name| name == "edit"));
}
