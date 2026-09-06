#[tokio::test]
async fn advisor_gate_applies_to_risky_calls_nested_inside_batch() {
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = Registry::new(provider).await;
    let temp = tempfile::TempDir::new().expect("temp dir");
    let target = temp.path().join("must-not-exist.txt");
    let session_id = "test-advisor-gates-batch-subcall";
    let manager = crate::advisor::advisor_manager();
    manager.remove(session_id);
    manager
        .set_enabled(session_id, true)
        .expect("save advisor control");
    assert!(manager.schedule_turn(
        session_id.to_string(),
        Arc::new(BlockingNoteProvider),
        Arc::new(std::sync::Mutex::new(Vec::new())),
        crate::advisor::AdvisorTurnInput::default(),
        crate::config::AdvisorConfig {
            enabled: true,
            mode: crate::config::AdvisorMode::SelfdevGuardian,
            ..crate::config::AdvisorConfig::default()
        },
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if manager
                .snapshot(session_id)
                .is_some_and(|snapshot| snapshot.status == crate::advisor::AdvisorStatus::Ready)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("advisor blocker should become ready");

    let result = registry
        .execute(
            "batch",
            serde_json::json!({
                "tool_calls": [{
                    "tool": "write",
                    "file_path": target,
                    "content": "this must be blocked"
                }]
            }),
            ToolContext {
                session_id: session_id.to_string(),
                message_id: "test".to_string(),
                tool_call_id: "test".to_string(),
                working_dir: Some(temp.path().to_path_buf()),
                stdin_request_tx: None,
                graceful_shutdown_signal: None,
                execution_mode: ToolExecutionMode::Direct,
            },
        )
        .await
        .expect("batch itself should complete with a failed subcall");

    // Registry keys do not confer capabilities: renaming a tool to look safe
    // cannot bypass the gate, and undeclared plugin tools fail closed.
    registry
        .register("innocent_lookup".into(), Arc::new(BareSchemaTool))
        .await;
    registry
        .register(
            "renamed_reader".into(),
            Arc::new(super::read::ReadTool::new()),
        )
        .await;
    let context = ToolContext {
        session_id: session_id.to_string(),
        message_id: "test".into(),
        tool_call_id: "renamed".into(),
        working_dir: Some(temp.path().to_path_buf()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::Direct,
    };
    assert!(
        registry
            .execute("innocent_lookup", serde_json::json!({}), context.clone())
            .await
            .is_err()
    );
    std::fs::write(temp.path().join("readable"), "safe to inspect").expect("fixture");
    let read = registry
        .execute(
            "renamed_reader",
            serde_json::json!({"file_path": "readable"}),
            context,
        )
        .await
        .expect("declared reader");
    assert!(read.output.contains("safe to inspect"));

    manager.remove(session_id);
    assert!(
        result
            .output
            .contains("advisor blocked future risky tool `write`")
    );
    assert!(result.output.contains("Completed: 0 succeeded, 1 failed"));
    assert!(
        !target.exists(),
        "blocked batch subcall must not write a file"
    );
}
