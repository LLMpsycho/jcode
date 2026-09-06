#[test]
fn test_parse_progress_marker_handles_percent_payloads() {
    let progress = parse_progress_marker(
        r#"JCODE_PROGRESS {"percent":25,"message":"Downloading dependencies"}"#,
    )
    .expect("marker should parse");

    assert_eq!(progress.percent, Some(25.0));
    assert_eq!(
        progress.message.as_deref(),
        Some("Downloading dependencies")
    );
    assert_eq!(progress.kind, BackgroundTaskProgressKind::Determinate);
    assert_eq!(progress.source, BackgroundTaskProgressSource::Reported);
}
#[test]
fn test_parse_heuristic_progress_handles_ratio_output() {
    let progress = parse_heuristic_progress("Running test 3/10 tests")
        .expect("heuristic parser should not fail")
        .expect("heuristic ratio progress should parse");

    assert_eq!(progress.current, Some(3));
    assert_eq!(progress.total, Some(10));
    assert_eq!(progress.percent, Some(30.0));
    assert_eq!(progress.unit.as_deref(), Some("tests"));
    assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
}
#[test]
fn test_parse_heuristic_progress_handles_percent_output() {
    let progress = parse_heuristic_progress("download progress 42% complete")
        .expect("heuristic parser should not fail")
        .expect("heuristic percent progress should parse");

    assert_eq!(progress.percent, Some(42.0));
    assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
    assert_eq!(
        progress.message.as_deref(),
        Some("download progress 42% complete")
    );
}
#[test]
fn test_parse_heuristic_progress_handles_phase_output() {
    let progress = parse_heuristic_progress("Compiling jcode v0.10.2")
        .expect("heuristic parser should not fail")
        .expect("phase progress should parse");

    assert_eq!(progress.kind, BackgroundTaskProgressKind::Indeterminate);
    assert_eq!(progress.percent, None);
    assert_eq!(progress.message.as_deref(), Some("Compiling jcode v0.10.2"));
    assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
}
#[test]
fn test_parse_heuristic_progress_handles_of_output() {
    let progress = parse_heuristic_progress("Downloaded 3 of 12 crates")
        .expect("heuristic parser should not fail")
        .expect("heuristic of progress should parse");

    assert_eq!(progress.current, Some(3));
    assert_eq!(progress.total, Some(12));
    assert_eq!(progress.percent, Some(25.0));
    assert_eq!(progress.unit.as_deref(), Some("crates"));
}
#[test]
fn test_parse_heuristic_progress_handles_byte_ratio_output() {
    let progress = parse_heuristic_progress("Downloaded 1.5/3.0 GiB")
        .expect("heuristic parser should not fail")
        .expect("heuristic byte ratio progress should parse");

    assert_eq!(progress.percent, Some(50.0));
    assert_eq!(progress.unit.as_deref(), Some("gib"));
    assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
}
#[tokio::test]
async fn test_background_command_progress_marker_updates_status_and_stays_out_of_output() {
    let tool = BashTool::new();
    let ctx = make_ctx(None);

    let result = tool
            .execute(
                json!({
                    "command": "printf '%s\n' 'JCODE_PROGRESS {\"current\":3,\"total\":10,\"unit\":\"steps\",\"message\":\"Building\"}'; sleep 0.1; echo done",
                    "run_in_background": true,
                    "notify": false,
                    "wake": false,
                }),
                ctx,
            )
            .await
            .expect("background command should start");

    let metadata = result.metadata.expect("expected metadata");
    let task_id = metadata["task_id"]
        .as_str()
        .expect("task id should be present")
        .to_string();

    let mut saw_progress = false;
    // Wall-clock deadline: observing emitted progress depends on scheduler
    // latency, so a fixed 50-iteration budget starved under parallel load
    // (issue #593). The assertions inside stay exact.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = crate::background::global()
            .status(&task_id)
            .await
            .expect("status should exist");
        if let Some(progress) = status.progress {
            saw_progress = true;
            assert_eq!(progress.current, Some(3));
            assert_eq!(progress.total, Some(10));
            assert_eq!(progress.unit.as_deref(), Some("steps"));
            assert_eq!(progress.message.as_deref(), Some("Building"));
            assert_eq!(progress.percent, Some(30.0));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        saw_progress,
        "expected progress to be recorded for {task_id}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let output = crate::background::global()
        .output(&task_id)
        .await
        .expect("output should exist");
    assert!(output.contains("done"), "output was: {output}");
    assert!(
        !output.contains("JCODE_PROGRESS"),
        "progress marker should be hidden from output: {output}"
    );
}
#[tokio::test]
async fn test_background_command_ratio_output_updates_progress() {
    let tool = BashTool::new();
    let ctx = make_ctx(None);

    let result = tool
        .execute(
            json!({
                "command": "printf '%s\n' 'Running test 4/8 tests'; sleep 0.1; echo done",
                "run_in_background": true,
                "notify": false,
                "wake": false,
            }),
            ctx,
        )
        .await
        .expect("background command should start");

    let metadata = result.metadata.expect("expected metadata");
    let task_id = metadata["task_id"]
        .as_str()
        .expect("task id should be present")
        .to_string();

    let mut saw_progress = false;
    // Wall-clock deadline: observing emitted progress depends on scheduler
    // latency, so a fixed 50-iteration budget starved under parallel load
    // (issue #593). The assertions inside stay exact.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = crate::background::global()
            .status(&task_id)
            .await
            .expect("status should exist");
        if let Some(progress) = status.progress {
            saw_progress = true;
            assert_eq!(progress.current, Some(4));
            assert_eq!(progress.total, Some(8));
            assert_eq!(progress.percent, Some(50.0));
            assert_eq!(progress.unit.as_deref(), Some("tests"));
            assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(
        saw_progress,
        "expected heuristic progress to be recorded for {task_id}"
    );
}
#[tokio::test]
async fn test_background_command_byte_ratio_output_updates_progress() {
    let tool = BashTool::new();
    let ctx = make_ctx(None);

    let result = tool
        .execute(
            json!({
                "command": "printf '%s\n' 'Downloaded 1.5/3.0 GiB'; sleep 0.1; echo done",
                "run_in_background": true,
                "notify": false,
                "wake": false,
            }),
            ctx,
        )
        .await
        .expect("background command should start");

    let metadata = result.metadata.expect("expected metadata");
    let task_id = metadata["task_id"]
        .as_str()
        .expect("task id should be present")
        .to_string();

    let mut saw_progress = false;
    // Wall-clock deadline: observing emitted progress depends on scheduler
    // latency, so a fixed 50-iteration budget starved under parallel load
    // (issue #593). The assertions inside stay exact.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = crate::background::global()
            .status(&task_id)
            .await
            .expect("status should exist");
        if let Some(progress) = status.progress {
            saw_progress = true;
            assert_eq!(progress.percent, Some(50.0));
            assert_eq!(progress.unit.as_deref(), Some("gib"));
            assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(
        saw_progress,
        "expected byte-ratio progress to be recorded for {task_id}"
    );
}
#[test]
fn parse_progress_line_classifies_markers_checkpoints_and_heuristics() {
    let update = parse_progress_line(r#"JCODE_PROGRESS {"percent":40,"message":"Working"}"#)
        .expect("parser should not fail")
        .expect("progress marker should parse");
    match update {
        ProgressLineUpdate::Progress(progress) => assert_eq!(progress.percent, Some(40.0)),
        other => panic!("expected a progress update, got {other:?}"),
    }

    let update = parse_progress_line(r#"JCODE_CHECKPOINT {"message":"Tests passed"}"#)
        .expect("parser should not fail")
        .expect("checkpoint marker should parse");
    match update {
        ProgressLineUpdate::Checkpoint(progress) => {
            assert_eq!(progress.message.as_deref(), Some("Tests passed"))
        }
        other => panic!("expected a checkpoint update, got {other:?}"),
    }

    let update = parse_progress_line("Copied 7/10 files")
        .expect("parser should not fail")
        .expect("heuristic ratio should parse");
    match update {
        ProgressLineUpdate::Progress(progress) => {
            assert_eq!(progress.percent, Some(70.0));
            assert_eq!(progress.source, BackgroundTaskProgressSource::ParsedOutput);
        }
        other => panic!("expected a progress update, got {other:?}"),
    }

    assert!(
        parse_progress_line("plain log line with no progress")
            .expect("parser should not fail")
            .is_none(),
        "non-progress output must not produce updates"
    );
}
/// The bug this guards against: a foreground command promoted to background at
/// the timeout showed 0% until it completed, because nothing parsed its output
/// for progress. Both the update emitted *before* promotion and updates
/// emitted *after* promotion must reach the background task's status.
#[tokio::test]
async fn test_timeout_promoted_command_reports_intermediate_progress() {
    let tool = BashTool::new();
    // Emits 10% before the 300ms foreground timeout, then 80% about 2s in.
    let input = json!({
        "command": "echo 'progress 10% done'; sleep 2; echo 'progress 80% done'; sleep 1",
        "timeout": 300,
    });
    let ctx = make_ctx(None);

    let result = tool
        .execute(input, ctx)
        .await
        .expect("timeout should promote to background");
    let metadata = result.metadata.expect("expected background metadata");
    assert_eq!(metadata["timeout_promoted"], true);
    let task_id = metadata["task_id"]
        .as_str()
        .expect("task_id should be present")
        .to_string();

    // The pre-promotion update (10%) must be attached at promotion time, and
    // the post-promotion update (80%) must stream in while still running.
    let mut observed: Vec<f32> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = crate::background::global()
            .status(&task_id)
            .await
            .expect("status should exist");
        if let Some(percent) = status.progress.as_ref().and_then(|p| p.percent)
            && observed.last() != Some(&percent)
        {
            observed.push(percent);
        }
        if observed.contains(&80.0) {
            break;
        }
        if status.status != BackgroundTaskStatus::Running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert!(
        observed.contains(&80.0),
        "promoted task should reach 80% via parsed output, saw {observed:?}"
    );
    assert!(
        observed.contains(&10.0),
        "the pre-promotion 10% update should be flushed at promotion, saw {observed:?}"
    );

    let _ = crate::background::global().cancel(&task_id).await;
}
/// Same guarantee for the reload-persistable (detached) path: the command
/// writes straight to its output file, so a follower must translate progress
/// lines into status updates while the task is still running.
#[tokio::test]
async fn test_detached_promoted_command_reports_intermediate_progress() {
    let tool = BashTool::new();
    let signal = jcode_agent_runtime::InterruptSignal::new();
    let ctx = make_agent_ctx(signal);

    let result = tool
        .execute(
            json!({
                "command": "sleep 0.5; echo 'done 3/10 steps'; sleep 2; echo 'done 8/10 steps'; sleep 1",
                "timeout": 200,
            }),
            ctx,
        )
        .await
        .expect("timeout should promote the detached command to background");
    let metadata = result.metadata.expect("expected background metadata");
    assert_eq!(metadata["timeout_promoted"], true);
    let task_id = metadata["task_id"]
        .as_str()
        .expect("task_id should be present")
        .to_string();

    let mut observed: Vec<f32> = Vec::new();
    let mut saw_intermediate_while_running = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = crate::background::global()
            .status(&task_id)
            .await
            .expect("status should exist");
        if let Some(percent) = status.progress.as_ref().and_then(|p| p.percent) {
            if observed.last() != Some(&percent) {
                observed.push(percent);
            }
            if status.status == BackgroundTaskStatus::Running && percent < 100.0 {
                saw_intermediate_while_running = true;
            }
        }
        if observed.contains(&80.0) {
            break;
        }
        if status.status != BackgroundTaskStatus::Running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(
        observed.contains(&30.0) && observed.contains(&80.0),
        "detached task should report 30% then 80% from parsed output, saw {observed:?}"
    );
    assert!(
        saw_intermediate_while_running,
        "intermediate progress must be visible while the task is still running"
    );

    let output_file = std::path::PathBuf::from(
        metadata["output_file"]
            .as_str()
            .expect("output_file should be present"),
    );
    let status_file = std::path::PathBuf::from(
        metadata["status_file"]
            .as_str()
            .expect("status_file should be present"),
    );
    let _ = crate::background::global().cancel(&task_id).await;
    let _ = tokio::fs::remove_file(output_file).await;
    let _ = tokio::fs::remove_file(status_file).await;
}
