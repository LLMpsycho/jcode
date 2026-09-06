#[tokio::test]
async fn build_requires_reason() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let tool = SelfDevTool::new();
    let ctx = create_test_context("build-session", Some(repo.path().to_path_buf()));
    let err = tool
        .execute(json!({"action": "build"}), ctx)
        .await
        .expect_err("build without reason should fail");

    assert!(err.to_string().contains("requires a non-empty `reason`"));
}
#[tokio::test]
async fn build_queues_background_tasks_and_reports_queue_status() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut session_one = session::Session::create(None, Some("First build session".to_string()));
    session_one.short_name = Some("alpha".to_string());
    session_one.save().expect("save session one");

    let mut session_two = session::Session::create(None, Some("Second build session".to_string()));
    session_two.short_name = Some("beta".to_string());
    session_two.save().expect("save session two");

    let tool = SelfDevTool::new();
    let first = tool
        .execute(
            json!({"action": "build", "reason": "first reason"}),
            create_test_context(&session_one.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("first build should queue");
    let second = tool
        .execute(
            json!({"action": "build", "reason": "second reason"}),
            create_test_context(&session_two.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("second build should queue");

    let first_meta = first.metadata.expect("first metadata");
    let second_meta = second.metadata.expect("second metadata");
    let first_task_id = first_meta["task_id"].as_str().expect("first task id");
    let second_task_id = second_meta["task_id"].as_str().expect("second task id");

    assert_eq!(first_meta["queue_position"].as_u64(), Some(1));
    assert_eq!(second_meta["deduped"].as_bool(), Some(true));
    assert!(
        second
            .output
            .contains("attached instead of spawning a duplicate build")
    );

    let status_output = selfdev_status_output().expect("status output");
    assert!(status_output.output.contains("## Build Queue"));
    assert!(status_output.output.contains("first reason"));
    assert!(status_output.output.contains("Attached watchers: 1"));
    assert!(
        status_output
            .output
            .contains("Target version: `test-build`")
    );

    let first_status = wait_for_task_completion(first_task_id).await;
    let second_status = wait_for_task_completion(second_task_id).await;
    assert_eq!(first_status.status, BackgroundTaskStatus::Completed);
    assert_eq!(second_status.status, BackgroundTaskStatus::Completed);

    let request_one =
        BuildRequest::load(first_meta["request_id"].as_str().expect("first request id"))
            .expect("load request one")
            .expect("request one exists");
    let request_two = BuildRequest::load(
        second_meta["request_id"]
            .as_str()
            .expect("second request id"),
    )
    .expect("load request two")
    .expect("request two exists");
    assert_eq!(request_one.state, BuildRequestState::Completed);
    assert_eq!(request_two.state, BuildRequestState::Completed);
}
#[tokio::test]
async fn build_reload_waits_for_build_then_reloads() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut session = session::Session::create(None, Some("Build+reload session".to_string()));
    session.is_canary = true;
    session.short_name = Some("gamma".to_string());
    session.save().expect("save session");

    // The reload phase blocks on a server ack. Spawn a watcher that mirrors the
    // server: it observes reload signals and acknowledges them so the inline
    // reload can complete deterministically in test mode. It keeps acking every
    // signal it sees (the RELOAD_SIGNAL channel is a process-global shared by
    // parallel tests, and `wait_for_reload_ack` matches by request id, so acking
    // unrelated/stale signals is harmless).
    let mut signal_rx = server::subscribe_reload_signal_for_tests();
    let acker = tokio::spawn(async move {
        if let Some(signal) = signal_rx.borrow_and_update().clone() {
            server::acknowledge_reload_signal(&signal);
        }
        while signal_rx.changed().await.is_ok() {
            if let Some(signal) = signal_rx.borrow_and_update().clone() {
                server::acknowledge_reload_signal(&signal);
            }
        }
    });

    let tool = SelfDevTool::new();
    let output = tool
        .execute(
            json!({"action": "build-reload", "reason": "combined build and reload"}),
            create_test_context(&session.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("build-reload should succeed");

    acker.abort();

    assert!(
        output.output.contains("Build completed successfully"),
        "unexpected output: {}",
        output.output
    );
    let meta = output.metadata.expect("build-reload metadata");
    assert_eq!(meta["phase"].as_str(), Some("reload"));
    assert_eq!(meta["build_finished"].as_bool(), Some(true));
    assert_eq!(meta["build_succeeded"].as_bool(), Some(true));

    let request_id = meta["request_id"].as_str().expect("request id in metadata");
    let request = BuildRequest::load(request_id)
        .expect("load request")
        .expect("request exists");
    assert_eq!(request.state, BuildRequestState::Completed);
}
#[tokio::test]
async fn build_dedupes_identical_reason_and_version_with_attached_watcher() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut session_one = session::Session::create(None, Some("Build A".to_string()));
    session_one.short_name = Some("alpha".to_string());
    session_one.save().expect("save session one");

    let mut session_two = session::Session::create(None, Some("Build B".to_string()));
    session_two.short_name = Some("beta".to_string());
    session_two.save().expect("save session two");

    let tool = SelfDevTool::new();
    let first = tool
        .execute(
            json!({"action": "build", "reason": "same reason"}),
            create_test_context(&session_one.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("first build should queue");
    let second = tool
        .execute(
            json!({"action": "build", "reason": "same reason"}),
            create_test_context(&session_two.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("second build should attach");

    let first_meta = first.metadata.expect("first metadata");
    let second_meta = second.metadata.expect("second metadata");
    assert_eq!(second_meta["deduped"].as_bool(), Some(true));
    assert_eq!(
        second_meta["duplicate_of"]["request_id"].as_str(),
        first_meta["request_id"].as_str()
    );

    let status_output = selfdev_status_output().expect("status output");
    assert!(status_output.output.contains("Attached watchers: 1"));
    assert!(status_output.output.contains("alpha"));
    assert!(status_output.output.contains("beta"));

    let first_status = wait_for_task_completion(first_meta["task_id"].as_str().unwrap()).await;
    let second_status = wait_for_task_completion(second_meta["task_id"].as_str().unwrap()).await;
    assert_eq!(first_status.status, BackgroundTaskStatus::Completed);
    assert_eq!(second_status.status, BackgroundTaskStatus::Completed);

    let watcher_request = BuildRequest::load(second_meta["request_id"].as_str().unwrap())
        .expect("load watcher request")
        .expect("watcher request exists");
    assert_eq!(watcher_request.state, BuildRequestState::Completed);
    assert_eq!(
        watcher_request.attached_to_request_id.as_deref(),
        first_meta["request_id"].as_str()
    );
}
#[tokio::test]
async fn cancel_build_marks_request_cancelled_and_removes_it_from_queue() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut session_one = session::Session::create(None, Some("Build A".to_string()));
    session_one.short_name = Some("alpha".to_string());
    session_one.save().expect("save session one");

    let mut session_two = session::Session::create(None, Some("Build B".to_string()));
    session_two.short_name = Some("beta".to_string());
    session_two.save().expect("save session two");

    let tool = SelfDevTool::new();
    let first = tool
        .execute(
            json!({"action": "build", "reason": "keep building"}),
            create_test_context(&session_one.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("first build should queue");
    let second = tool
        .execute(
            json!({"action": "build", "reason": "cancel me"}),
            create_test_context(&session_two.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("second build should queue");

    let second_meta = second.metadata.expect("second metadata");
    let cancel = tool
        .execute(
            json!({
                "action": "cancel-build",
                "request_id": second_meta["request_id"].as_str().unwrap()
            }),
            create_test_context(&session_two.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("cancel should succeed");

    assert!(cancel.output.contains("Cancelled self-dev build request"));

    let second_status = wait_for_task_completion(second_meta["task_id"].as_str().unwrap()).await;
    assert_eq!(second_status.status, BackgroundTaskStatus::Failed);

    let cancelled_request = BuildRequest::load(second_meta["request_id"].as_str().unwrap())
        .expect("load cancelled request")
        .expect("cancelled request exists");
    assert_eq!(cancelled_request.state, BuildRequestState::Cancelled);

    let status_output = selfdev_status_output().expect("status output");
    assert!(status_output.output.contains("keep building"));
    assert!(!status_output.output.contains("cancel me"));

    let first_meta = first.metadata.expect("first metadata");
    let first_status = wait_for_task_completion(first_meta["task_id"].as_str().unwrap()).await;
    assert_eq!(first_status.status, BackgroundTaskStatus::Completed);
}
#[test]
fn status_output_prunes_stale_pending_requests() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Stale Build".to_string()));
    session.short_name = Some("ghost".to_string());
    session.save().expect("save session");

    let stale_status_path = temp_home.path().join("missing-selfdev.status.json");
    let source = test_source_state(std::path::Path::new("/tmp/jcode"));
    let request = BuildRequest {
        request_id: "stale-request".to_string(),
        background_task_id: Some("missing-task".to_string()),
        session_id: session.id.clone(),
        session_short_name: session.short_name.clone(),
        session_title: Some("Stale Build".to_string()),
        reason: "stale reason".to_string(),
        repo_dir: "/tmp/jcode".to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode".to_string(),
        // Outside the bootstrap grace window: a request with a missing status
        // file is only pruned once it is old enough that the queue handler
        // cannot still be mid-spawn.
        requested_at: (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339(),
        started_at: Some(Utc::now().to_rfc3339()),
        completed_at: None,
        state: BuildRequestState::Building,
        version: Some("stale-build".to_string()),
        dedupe_key: Some("stale-dedupe".to_string()),
        requested_source: Some(source),
        built_source: None,
        published_version: None,
        last_progress: Some("building".to_string()),
        validated: false,
        error: None,
        output_file: None,
        status_file: Some(stale_status_path.display().to_string()),
        attached_to_request_id: None,
    };
    request.save().expect("save stale request");

    let status_output = selfdev_status_output().expect("status output");
    assert!(
        !status_output.output.contains("stale reason"),
        "stale request should be pruned from queue output"
    );

    let request = BuildRequest::load("stale-request")
        .expect("load stale request")
        .expect("stale request exists");
    assert_eq!(request.state, BuildRequestState::Failed);
    assert!(
        request
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("pruning stale self-dev build request"),
        "stale request should record why it was pruned"
    );
}
#[test]
fn freshly_queued_request_survives_reconcile_before_task_metadata_exists() {
    // Regression: the queue handler saves the request *before* spawning its
    // background task, so for a moment it has no task id / status file. A
    // concurrent reconcile (status output, another agent's queue poll, or the
    // task's own first wait_for_turn iteration) used to prune it as stale,
    // killing the build instantly with "Queued build request disappeared".
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Fresh Build".to_string()));
    session.save().expect("save session");

    let source = test_source_state(std::path::Path::new("/tmp/jcode"));
    let request = BuildRequest {
        request_id: "fresh-request".to_string(),
        // No background task metadata yet: mid-bootstrap.
        background_task_id: None,
        session_id: session.id.clone(),
        session_short_name: session.short_name.clone(),
        session_title: Some("Fresh Build".to_string()),
        reason: "fresh reason".to_string(),
        repo_dir: "/tmp/jcode".to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode".to_string(),
        requested_at: Utc::now().to_rfc3339(),
        started_at: None,
        completed_at: None,
        state: BuildRequestState::Queued,
        version: Some("fresh-build".to_string()),
        dedupe_key: Some("fresh-dedupe".to_string()),
        requested_source: Some(source.clone()),
        built_source: None,
        published_version: None,
        last_progress: Some("queued".to_string()),
        validated: false,
        error: None,
        output_file: None,
        status_file: None,
        attached_to_request_id: None,
    };
    request.save().expect("save fresh request");

    let pending =
        BuildRequest::pending_requests_for_scope(&source.worktree_scope).expect("pending requests");
    assert!(
        pending
            .iter()
            .any(|request| request.request_id == "fresh-request"),
        "freshly queued request must stay pending during the bootstrap grace window"
    );

    let reloaded = BuildRequest::load("fresh-request")
        .expect("load fresh request")
        .expect("fresh request exists");
    assert_eq!(reloaded.state, BuildRequestState::Queued);
    assert!(reloaded.error.is_none());
}
#[tokio::test]
async fn build_ignores_stale_pending_requests_when_computing_queue_position() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut stale_session = session::Session::create(None, Some("Stale Build".to_string()));
    stale_session.short_name = Some("ghost".to_string());
    stale_session.save().expect("save stale session");

    let stale_status_path = temp_home.path().join("stale-running.status.json");
    storage::write_json(
        &stale_status_path,
        &background::TaskStatusFile {
            task_id: "stale-task".to_string(),
            tool_name: "selfdev-build".to_string(),
            display_name: Some("selfdev build".to_string()),
            session_id: stale_session.id.clone(),
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            error: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            duration_secs: None,
            pid: None,
            owner_pid: None,
            owner_instance: None,
            detached: false,
            notify: true,
            wake: true,
            progress: None,
            event_history: Vec::new(),
            stall_wake_seconds: None,
        },
    )
    .expect("write stale status file");

    let source = test_source_state(repo.path());
    let stale_request = BuildRequest {
        request_id: "stale-queued-request".to_string(),
        background_task_id: Some("stale-task".to_string()),
        session_id: stale_session.id.clone(),
        session_short_name: stale_session.short_name.clone(),
        session_title: Some("Stale Build".to_string()),
        reason: "stale blocker".to_string(),
        repo_dir: repo.path().display().to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode".to_string(),
        // Backdated beyond the 30s bootstrap grace so reconciliation treats the
        // dead-task request as genuinely stale (a fresh timestamp would keep it
        // alive and Queued, which is the bootstrap-race protection, not the
        // staleness path under test).
        requested_at: (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339(),
        started_at: Some(Utc::now().to_rfc3339()),
        completed_at: None,
        state: BuildRequestState::Queued,
        version: Some("test-build".to_string()),
        dedupe_key: Some("stale-dedupe".to_string()),
        requested_source: Some(source),
        built_source: None,
        published_version: None,
        last_progress: Some("queued".to_string()),
        validated: false,
        error: None,
        output_file: None,
        status_file: Some(stale_status_path.display().to_string()),
        attached_to_request_id: None,
    };
    stale_request.save().expect("save stale queued request");

    let mut live_session = session::Session::create(None, Some("Live Build".to_string()));
    live_session.short_name = Some("alpha".to_string());
    live_session.save().expect("save live session");

    let tool = SelfDevTool::new();
    let output = tool
        .execute(
            json!({"action": "build", "reason": "fresh build"}),
            create_test_context(&live_session.id, Some(repo.path().to_path_buf())),
        )
        .await
        .expect("build should queue");

    let metadata = output.metadata.expect("build metadata");
    assert_eq!(metadata["queue_position"].as_u64(), Some(1));
    assert!(
        !output.output.contains("Currently blocked by"),
        "stale queued requests should not block new builds"
    );

    let stale_request = BuildRequest::load("stale-queued-request")
        .expect("load stale queued request")
        .expect("stale queued request exists");
    assert_eq!(stale_request.state, BuildRequestState::Failed);

    let task_id = metadata["task_id"].as_str().expect("task id");
    let status = wait_for_task_completion(task_id).await;
    assert_eq!(status.status, BackgroundTaskStatus::Completed);
}
#[test]
fn reconcile_pending_state_maps_superseded_background_status() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Superseded Build".to_string()));
    session.short_name = Some("alpha".to_string());
    session.save().expect("save session");

    let status_path = temp_home.path().join("superseded.status.json");
    storage::write_json(
        &status_path,
        &background::TaskStatusFile {
            task_id: "superseded-task".to_string(),
            tool_name: "selfdev-build".to_string(),
            display_name: Some("selfdev build".to_string()),
            session_id: session.id.clone(),
            status: BackgroundTaskStatus::Superseded,
            exit_code: Some(0),
            error: Some("Build completed, but source changed before activation".to_string()),
            started_at: Utc::now().to_rfc3339(),
            completed_at: Some(Utc::now().to_rfc3339()),
            duration_secs: Some(1.0),
            pid: None,
            owner_pid: None,
            owner_instance: None,
            detached: false,
            notify: true,
            wake: true,
            progress: None,
            event_history: Vec::new(),
            stall_wake_seconds: None,
        },
    )
    .expect("write superseded status file");

    let source = test_source_state(std::path::Path::new("/tmp/jcode"));
    let request = BuildRequest {
        request_id: "superseded-request".to_string(),
        background_task_id: Some("superseded-task".to_string()),
        session_id: session.id.clone(),
        session_short_name: session.short_name.clone(),
        session_title: Some("Superseded Build".to_string()),
        reason: "superseded reason".to_string(),
        repo_dir: "/tmp/jcode".to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode".to_string(),
        requested_at: Utc::now().to_rfc3339(),
        started_at: Some(Utc::now().to_rfc3339()),
        completed_at: None,
        state: BuildRequestState::Building,
        version: Some("superseded-build".to_string()),
        dedupe_key: Some("superseded-dedupe".to_string()),
        requested_source: Some(source),
        built_source: None,
        published_version: None,
        last_progress: Some("building".to_string()),
        validated: false,
        error: None,
        output_file: None,
        status_file: Some(status_path.display().to_string()),
        attached_to_request_id: None,
    };
    request.save().expect("save superseded request");

    let mut request = BuildRequest::load("superseded-request")
        .expect("load superseded request")
        .expect("request exists");
    assert!(
        !request
            .reconcile_pending_state()
            .expect("reconcile superseded request")
    );
    assert_eq!(request.state, BuildRequestState::Superseded);
    assert!(
        request
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("source changed before activation")
    );
}
#[test]
fn reconcile_keeps_running_request_not_yet_registered_in_live_task_map() {
    // Regression: spawn_with_notify writes the Running status file and starts
    // the build future *before* inserting the task into the in-process task
    // map. The build's own first wait_for_turn iteration (or another agent's
    // queue poll) could then see status=Running + is_live_task=false and prune
    // the request instantly: "Queued build request disappeared". Within the
    // bootstrap grace window a Running-but-unregistered task must survive.
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Racing Build".to_string()));
    session.save().expect("save session");

    let status_path = temp_home.path().join("racing.status.json");
    storage::write_json(
        &status_path,
        &background::TaskStatusFile {
            task_id: "racing-task-not-in-live-map".to_string(),
            tool_name: "selfdev-build".to_string(),
            display_name: Some("selfdev build".to_string()),
            session_id: session.id.clone(),
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            error: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            duration_secs: None,
            pid: None,
            owner_pid: None,
            owner_instance: None,
            detached: false,
            notify: true,
            wake: true,
            progress: None,
            event_history: Vec::new(),
            stall_wake_seconds: None,
        },
    )
    .expect("write running status file");

    let source = test_source_state(std::path::Path::new("/tmp/jcode"));
    let request = BuildRequest {
        request_id: "racing-request".to_string(),
        background_task_id: Some("racing-task-not-in-live-map".to_string()),
        session_id: session.id.clone(),
        session_short_name: session.short_name.clone(),
        session_title: Some("Racing Build".to_string()),
        reason: "racing reason".to_string(),
        repo_dir: "/tmp/jcode".to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode".to_string(),
        requested_at: Utc::now().to_rfc3339(),
        started_at: None,
        completed_at: None,
        state: BuildRequestState::Queued,
        version: Some("racing-build".to_string()),
        dedupe_key: Some("racing-dedupe".to_string()),
        requested_source: Some(source.clone()),
        built_source: None,
        published_version: None,
        last_progress: Some("queued".to_string()),
        validated: false,
        error: None,
        output_file: None,
        status_file: Some(status_path.display().to_string()),
        attached_to_request_id: None,
    };
    request.save().expect("save racing request");

    let pending =
        BuildRequest::pending_requests_for_scope(&source.worktree_scope).expect("pending requests");
    assert!(
        pending
            .iter()
            .any(|request| request.request_id == "racing-request"),
        "running-but-unregistered request must stay pending during bootstrap grace"
    );

    let reloaded = BuildRequest::load("racing-request")
        .expect("load racing request")
        .expect("racing request exists");
    assert_eq!(reloaded.state, BuildRequestState::Queued);
    assert!(reloaded.error.is_none());
}
