use super::*;
use crate::tool::{ToolContext, ToolExecutionMode};
use jcode_edit_core::display_tag_hex;
use serde_json::json;
use std::sync::Arc;

fn make_ctx(working_dir: std::path::PathBuf) -> ToolContext {
    ToolContext {
        session_id: "test-session".to_string(),
        message_id: "test-message".to_string(),
        tool_call_id: "test-call".to_string(),
        working_dir: Some(working_dir),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::Direct,
    }
}

#[tokio::test]
async fn advisor_text_reader_caps_actual_bytes_and_refuses_media_helpers() {
    let dir = tempfile::tempdir().expect("workspace");
    let tool = ReadTool::for_advisor(16);
    let context = make_ctx(dir.path().to_path_buf());
    std::fs::write(dir.path().join("small.rs"), "valid code").expect("small");
    assert!(
        tool.execute(json!({"file_path":"small.rs"}), context.clone())
            .await
            .expect("small read")
            .output
            .contains("valid code")
    );
    // Exercise the byte reader directly: callers' earlier metadata checks are
    // deliberately absent, as with a file that grows between stat and open.
    std::fs::write(dir.path().join("grew.rs"), "x".repeat(1000)).expect("grew");
    assert!(
        tool.execute(json!({"file_path":"grew.rs"}), context.clone())
            .await
            .expect_err("bounded bytes")
            .to_string()
            .contains("byte limit")
    );
    for path in ["image.png", "report.pdf", "icon.ico"] {
        std::fs::write(dir.path().join(path), "media").expect("media");
        assert!(
            tool.execute(json!({"file_path":path}), context.clone())
                .await
                .expect_err("no media processing")
                .to_string()
                .contains("text files only")
        );
    }
}

#[test]
fn normalize_read_range_supports_start_and_end_lines() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 10,
        "end_line": 20
    }))
    .expect("deserialize params");

    let range = normalize_read_range(&params).expect("normalize range");
    assert_eq!(
        range,
        NormalizedReadRange {
            offset: 9,
            limit: 11,
            style: ReadRangeStyle::StartEnd,
        }
    );
}

#[test]
fn normalize_read_range_supports_start_line_and_limit() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 10,
        "limit": 20
    }))
    .expect("deserialize params");

    let range = normalize_read_range(&params).expect("start_line + limit should work");
    assert_eq!(
        range,
        NormalizedReadRange {
            offset: 9,
            limit: 20,
            style: ReadRangeStyle::StartEnd,
        }
    );
}

#[test]
fn normalize_read_range_prefers_end_line_over_limit() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 10,
        "end_line": 20,
        "limit": 999
    }))
    .expect("deserialize params");

    let range = normalize_read_range(&params).expect("end_line should take precedence");
    assert_eq!(
        range,
        NormalizedReadRange {
            offset: 9,
            limit: 11,
            style: ReadRangeStyle::StartEnd,
        }
    );
}

#[test]
fn normalize_read_range_rejects_start_line_and_offset() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 10,
        "offset": 20
    }))
    .expect("deserialize params");

    let err = normalize_read_range(&params).expect_err("mixed range styles should fail");
    assert!(
        err.to_string().contains("Use either start_line/end_line")
            || err.to_string().contains("not both"),
        "unexpected error: {err}"
    );
}

#[test]
fn normalize_read_range_accepts_matching_start_line_and_offset() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 10,
        "offset": 9,
        "limit": 20
    }))
    .expect("deserialize params");

    let range = normalize_read_range(&params).expect("matching range styles should work");
    assert_eq!(
        range,
        NormalizedReadRange {
            offset: 9,
            limit: 20,
            style: ReadRangeStyle::StartEnd,
        }
    );
}

#[test]
fn normalize_read_range_accepts_end_line_with_zero_offset() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "end_line": 20,
        "offset": 0
    }))
    .expect("deserialize params");

    let range = normalize_read_range(&params).expect("redundant zero offset should work");
    assert_eq!(
        range,
        NormalizedReadRange {
            offset: 0,
            limit: 20,
            style: ReadRangeStyle::StartEnd,
        }
    );
}

#[test]
fn normalize_read_range_rejects_invalid_end_before_start() {
    let params: ReadInput = serde_json::from_value(json!({
        "file_path": "src/lib.rs",
        "start_line": 20,
        "end_line": 10
    }))
    .expect("deserialize params");

    let err = normalize_read_range(&params).expect_err("invalid range should fail");
    assert!(
        err.to_string()
            .contains("greater than or equal to start_line"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_tool_schema_avoids_openai_incompatible_combinators() {
    let schema = ReadTool::new().parameters_schema();

    assert_eq!(schema.get("type"), Some(&json!("object")));
    assert!(schema.get("allOf").is_none());
    assert!(schema.get("not").is_none());
}

#[test]
fn read_tool_schema_advertises_only_canonical_public_fields() {
    let schema = ReadTool::new().parameters_schema();
    let properties = schema["properties"]
        .as_object()
        .expect("read schema properties should be an object");

    assert!(properties.contains_key("file_path"));
    assert!(properties.contains_key("start_line"));
    assert!(properties.contains_key("limit"));
    assert!(!properties.contains_key("end_line"));
    assert!(!properties.contains_key("offset"));
}

#[test]
fn read_tool_description_advertises_supported_file_types() {
    let tool = ReadTool::new();
    let description = tool.description().to_lowercase();
    assert!(description.contains("text"), "description={description}");
    assert!(description.contains("image"), "description={description}");
    assert!(description.contains("pdf"), "description={description}");

    let schema = tool.parameters_schema();
    let file_path_description = schema["properties"]["file_path"]["description"]
        .as_str()
        .expect("file_path should have a description");
    assert_eq!(file_path_description, "Path to a file.");
}

#[tokio::test]
async fn read_tool_supports_start_line_and_end_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("sample.txt");
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").expect("write sample file");

    let tool = ReadTool::new();
    let output = tool
        .execute(
            json!({
                "file_path": "sample.txt",
                "start_line": 2,
                "end_line": 4
            }),
            make_ctx(temp.path().to_path_buf()),
        )
        .await
        .expect("read execution should succeed");

    assert!(
        output.output.contains("2\ttwo"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("3\tthree"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("4\tfour"),
        "output={:?}",
        output.output
    );
    assert!(
        !output.output.contains("1\tone"),
        "output={:?}",
        output.output
    );
    assert!(
        !output.output.contains("5\tfive"),
        "output={:?}",
        output.output
    );
}

#[tokio::test]
async fn read_tool_continuation_hint_matches_start_line_style() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("sample.txt");
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").expect("write sample file");

    let tool = ReadTool::new();
    let output = tool
        .execute(
            json!({
                "file_path": "sample.txt",
                "start_line": 2,
                "end_line": 3
            }),
            make_ctx(temp.path().to_path_buf()),
        )
        .await
        .expect("read execution should succeed");

    assert!(
        output.output.contains("use start_line=4 to continue"),
        "output={:?}",
        output.output
    );
}

#[tokio::test]
async fn read_tool_supports_start_line_with_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("sample.txt");
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").expect("write sample file");

    let tool = ReadTool::new();
    let output = tool
        .execute(
            json!({
                "file_path": "sample.txt",
                "start_line": 2,
                "limit": 2
            }),
            make_ctx(temp.path().to_path_buf()),
        )
        .await
        .expect("read execution should succeed");

    assert!(
        output.output.contains("2\ttwo"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("3\tthree"),
        "output={:?}",
        output.output
    );
    assert!(
        !output.output.contains("4\tfour"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("use start_line=4 to continue"),
        "output={:?}",
        output.output
    );
}

#[tokio::test]
async fn read_tool_prefers_end_line_over_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("sample.txt");
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").expect("write sample file");

    let tool = ReadTool::new();
    let output = tool
        .execute(
            json!({
                "file_path": "sample.txt",
                "start_line": 2,
                "end_line": 3,
                "limit": 50
            }),
            make_ctx(temp.path().to_path_buf()),
        )
        .await
        .expect("read execution should succeed");

    assert!(
        output.output.contains("2\ttwo"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("3\tthree"),
        "output={:?}",
        output.output
    );
    assert!(
        !output.output.contains("4\tfour"),
        "output={:?}",
        output.output
    );
    assert!(
        output.output.contains("use start_line=4 to continue"),
        "output={:?}",
        output.output
    );
}

struct NoopProvider;

#[async_trait::async_trait]
impl crate::provider::Provider for NoopProvider {
    async fn complete(
        &self,
        _messages: &[crate::message::Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> anyhow::Result<crate::provider::EventStream> {
        anyhow::bail!("NoopProvider should not be called by read tests")
    }

    fn name(&self) -> &str {
        "noop"
    }

    fn fork(&self) -> Arc<dyn crate::provider::Provider> {
        Arc::new(Self)
    }
}

#[tokio::test]
async fn registry_injected_ledger_headers_full_reads_and_records_exact_coverage() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::create_dir_all(workspace.path().join("src")).expect("create src");
    std::fs::write(workspace.path().join("src/lib.rs"), "one\ntwo\nthree\n")
        .expect("write text file");
    let ledger = FileSnapshotLedger::new();
    let provider: Arc<dyn crate::provider::Provider> = Arc::new(NoopProvider);
    let registry = crate::tool::Registry::new_with_file_snapshots(provider, ledger.clone()).await;

    let output = registry
        .execute(
            "read",
            json!({"file_path": "src/lib.rs"}),
            make_ctx(workspace.path().to_path_buf()),
        )
        .await
        .expect("registry read should succeed");
    let read = ledger
        .session_read("test-session", workspace.path(), "src/lib.rs")
        .await
        .expect("query session read")
        .expect("read should be recorded");

    assert_eq!(read.revision.revision, 1);
    assert!(read.full_file);
    assert!(read.ranges.is_empty());
    assert!(output.output.starts_with(&format!(
        "[src/lib.rs#{} rev=1]\n",
        display_tag_hex(read.revision.display_tag)
    )));
}

#[tokio::test]
async fn partial_text_read_records_only_returned_lines() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(
        workspace.path().join("sample.txt"),
        "one\ntwo\nthree\nfour\nfive\n",
    )
    .expect("write text file");
    let ledger = FileSnapshotLedger::new();
    let tool = ReadTool::with_file_snapshots(ledger.clone());

    let output = tool
        .execute(
            json!({"file_path": "sample.txt", "start_line": 2, "limit": 2}),
            make_ctx(workspace.path().to_path_buf()),
        )
        .await
        .expect("partial read should succeed");
    let read = ledger
        .session_read("test-session", workspace.path(), "sample.txt")
        .await
        .expect("query session read")
        .expect("read should be recorded");

    assert_eq!(read.ranges, vec![LineRange { start: 2, end: 3 }]);
    assert!(!read.full_file);
    assert!(output.output.starts_with(&format!(
        "[sample.txt#{} rev=1]\n",
        display_tag_hex(read.revision.display_tag)
    )));
}

#[tokio::test]
async fn read_after_observed_write_uses_the_new_revision() {
    let workspace = tempfile::tempdir().expect("workspace");
    let path = workspace.path().join("sample.txt");
    std::fs::write(&path, "before\n").expect("write initial text");
    let ledger = FileSnapshotLedger::new();
    let tool = ReadTool::with_file_snapshots(ledger.clone());
    let ctx = make_ctx(workspace.path().to_path_buf());

    let first = tool
        .execute(json!({"file_path": "sample.txt"}), ctx.clone())
        .await
        .expect("initial read should succeed");
    assert!(first.output.contains("rev=1]"));

    std::fs::write(&path, "after\n").expect("write changed text");
    let write = ledger
        .record_write("writer", workspace.path(), "sample.txt", b"after\n", None)
        .await
        .expect("record observed write");
    assert_eq!(write.revision.revision, 2);

    let second = tool
        .execute(json!({"file_path": "sample.txt"}), ctx)
        .await
        .expect("read after write should succeed");
    assert!(second.output.starts_with(&format!(
        "[sample.txt#{} rev=2]\n",
        display_tag_hex(write.revision.display_tag)
    )));
}

#[tokio::test]
async fn same_relative_path_in_separate_worktrees_has_independent_snapshots() {
    let left = tempfile::tempdir().expect("left workspace");
    let right = tempfile::tempdir().expect("right workspace");
    std::fs::write(left.path().join("same.txt"), "left\n").expect("write left");
    std::fs::write(right.path().join("same.txt"), "right\n").expect("write right");
    let ledger = FileSnapshotLedger::new();
    let tool = ReadTool::with_file_snapshots(ledger.clone());

    let left_output = tool
        .execute(
            json!({"file_path": "same.txt"}),
            make_ctx(left.path().to_path_buf()),
        )
        .await
        .expect("read left");
    let right_output = tool
        .execute(
            json!({"file_path": "same.txt"}),
            make_ctx(right.path().to_path_buf()),
        )
        .await
        .expect("read right");
    let left_snapshot = ledger
        .snapshot(left.path(), "same.txt")
        .await
        .expect("query left")
        .expect("left snapshot");
    let right_snapshot = ledger
        .snapshot(right.path(), "same.txt")
        .await
        .expect("query right")
        .expect("right snapshot");

    assert_eq!(left_snapshot.revision.revision, 1);
    assert_eq!(right_snapshot.revision.revision, 1);
    assert_ne!(
        left_snapshot.revision.content_digest,
        right_snapshot.revision.content_digest
    );
    assert!(left_output.output.contains("left"));
    assert!(right_output.output.contains("right"));
}

#[tokio::test]
async fn binary_reads_do_not_get_snapshot_headers_or_ledger_entries() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join("payload.bin"), [0, 1, 2, 3]).expect("write binary file");
    let ledger = FileSnapshotLedger::new();
    let tool = ReadTool::with_file_snapshots(ledger.clone());

    let output = tool
        .execute(
            json!({"file_path": "payload.bin"}),
            make_ctx(workspace.path().to_path_buf()),
        )
        .await
        .expect("binary read should succeed");

    assert_eq!(
        output.output,
        "Binary file detected: payload.bin\nUse appropriate tools to handle binary files."
    );
    assert!(
        ledger
            .snapshot(workspace.path(), "payload.bin")
            .await
            .expect("query ledger")
            .is_none()
    );
}

#[tokio::test]
async fn read_without_ledger_preserves_existing_output() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join("sample.txt"), "one\n").expect("write text file");

    let output = ReadTool::new()
        .execute(
            json!({"file_path": "sample.txt"}),
            make_ctx(workspace.path().to_path_buf()),
        )
        .await
        .expect("read should succeed");

    assert_eq!(output.output, "    1\tone\n");
}
