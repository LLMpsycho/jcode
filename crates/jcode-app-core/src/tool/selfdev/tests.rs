use super::*;
use crate::bus::BackgroundTaskStatus;
use std::ffi::OsStr;

struct EnvVarGuard {
    key: &'static str,
    original: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let original = std::env::var_os(key);
        crate::env::set_var(key, value);
        Self { key, original }
    }

    fn remove(key: &'static str) -> Self {
        let original = std::env::var_os(key);
        crate::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => crate::env::set_var(self.key, value),
            None => crate::env::remove_var(self.key),
        }
    }
}

fn create_test_context(session_id: &str, working_dir: Option<std::path::PathBuf>) -> ToolContext {
    ToolContext {
        session_id: session_id.to_string(),
        message_id: "test-message".to_string(),
        tool_call_id: "test-tool-call".to_string(),
        working_dir,
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    }
}

fn create_repo_fixture() -> tempfile::TempDir {
    let temp = tempfile::TempDir::new().expect("temp repo");
    std::fs::create_dir_all(temp.path().join(".git")).expect("git dir");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"jcode\"\nversion = \"0.1.0\"\n",
    )
    .expect("cargo toml");
    temp
}

fn test_source_state(repo_dir: &std::path::Path) -> build::SourceState {
    build::SourceState {
        repo_scope: "test-repo-scope".to_string(),
        worktree_scope: build::worktree_scope_key(repo_dir)
            .unwrap_or_else(|_| "test-worktree".to_string()),
        short_hash: "test-build".to_string(),
        full_hash: "test-build-full".to_string(),
        dirty: true,
        fingerprint: "test-fingerprint".to_string(),
        version_label: "test-build".to_string(),
        changed_paths: 0,
    }
}

fn request_fixture(
    request_id: &str,
    state: BuildRequestState,
    requested_at: String,
) -> BuildRequest {
    let source = test_source_state(std::path::Path::new("/tmp/jcode"));
    BuildRequest {
        request_id: request_id.to_string(),
        background_task_id: None,
        session_id: "session-history-test".to_string(),
        session_short_name: None,
        session_title: None,
        reason: request_id.to_string(),
        repo_dir: "/tmp/jcode".to_string(),
        repo_scope: source.repo_scope.clone(),
        worktree_scope: source.worktree_scope.clone(),
        command: "cargo test -p jcode-base".to_string(),
        requested_at,
        started_at: None,
        completed_at: None,
        state,
        version: None,
        dedupe_key: None,
        requested_source: Some(source),
        built_source: None,
        published_version: None,
        last_progress: None,
        validated: false,
        error: None,
        output_file: None,
        status_file: None,
        attached_to_request_id: None,
    }
}

#[test]
fn build_lock_is_removed_on_drop_and_can_be_reacquired() {
    let _env_lock = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp jcode home");
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let scope = format!("lock-drop-{}", std::process::id());
    let path = SelfDevTool::build_lock_path(&scope).expect("lock path");

    let first = SelfDevTool::try_acquire_build_lock(&scope)
        .expect("first lock attempt")
        .expect("first lock acquired");
    assert!(path.exists(), "lock file should exist while held");
    drop(first);
    assert!(!path.exists(), "lock file should be removed on drop");

    let second = SelfDevTool::try_acquire_build_lock(&scope)
        .expect("second lock attempt")
        .expect("lock should be reacquirable after drop");
    drop(second);
    assert!(!path.exists(), "reacquired lock should also clean up");
}

#[test]
fn terminal_request_history_is_archived_without_touching_active_requests() {
    // One shared env lock only: `lock_test_env` is a plain non-reentrant mutex,
    // so taking a second env guard here would self-deadlock (issue #593).
    let _storage_guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("temp jcode home");
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _limit = EnvVarGuard::set("JCODE_SELFDEV_REQUEST_HISTORY_LIMIT", "2");

    let base = Utc::now() - chrono::Duration::minutes(10);
    for index in 0..4 {
        request_fixture(
            &format!("terminal-{index}"),
            BuildRequestState::Completed,
            (base + chrono::Duration::minutes(index)).to_rfc3339(),
        )
        .save()
        .expect("save terminal request");
    }
    request_fixture(
        "active-request",
        BuildRequestState::Queued,
        Utc::now().to_rfc3339(),
    )
    .save()
    .expect("save active request");

    let live = BuildRequest::load_all().expect("load live requests");
    let live_ids = live
        .iter()
        .map(|request| request.request_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(live.len(), 3);
    assert!(live_ids.contains("terminal-2"));
    assert!(live_ids.contains("terminal-3"));
    assert!(live_ids.contains("active-request"));

    let archive = BuildRequest::requests_dir()
        .expect("requests dir")
        .join("archive");
    assert!(archive.join("terminal-0.json").exists());
    assert!(archive.join("terminal-1.json").exists());
}

#[test]
fn optimized_test_shell_command_routes_compile_subcommands_only() {
    let shell = SelfDevTool::optimized_test_shell_command(
        "cargo test -p jcode-base && cargo fmt --all -- --check",
    );

    assert!(shell.contains("test|check|build|clippy|bench"));
    assert!(shell.contains("JCODE_DEV_CARGO_SCRIPT"));
    assert!(shell.contains("JCODE_IN_DEV_CARGO=1"));
    assert!(shell.contains("*) command cargo \"$@\" ;;"));
    assert!(shell.ends_with("cargo test -p jcode-base && cargo fmt --all -- --check"));
}

#[cfg(unix)]
#[test]
fn optimized_test_shell_command_executes_raw_cargo_test_through_wrapper() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("temp command wrapper");
    let wrapper = temp.path().join("dev_cargo.sh");
    let capture = temp.path().join("args.txt");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$JCODE_TEST_CAPTURE\"\n",
    )
    .expect("write wrapper");
    let mut permissions = std::fs::metadata(&wrapper)
        .expect("wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("make wrapper executable");

    let status = std::process::Command::new("bash")
        .args([
            "-lc",
            &SelfDevTool::optimized_test_shell_command("cargo test -p demo --lib"),
        ])
        .env("JCODE_DEV_CARGO_SCRIPT", &wrapper)
        .env("JCODE_TEST_CAPTURE", &capture)
        .env_remove("JCODE_IN_DEV_CARGO")
        .status()
        .expect("run optimized shell command");

    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(capture).expect("captured args"),
        "test -p demo --lib\n"
    );
}

async fn wait_for_task_completion(task_id: &str) -> background::TaskStatusFile {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = background::global().status(task_id).await
            && status.status != BackgroundTaskStatus::Running
        {
            return status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for background task {}",
            task_id
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[test]
fn test_reload_context_serialization() {
    // Create test context with task info
    let ctx = ReloadContext {
        task_context: Some("Testing the reload feature".to_string()),
        version_before: "v0.1.100".to_string(),
        version_after: "abc1234".to_string(),
        session_id: "test-session-123".to_string(),
        timestamp: "2025-01-20T00:00:00Z".to_string(),
    };

    // Serialize and deserialize
    let json = serde_json::to_string(&ctx).unwrap();
    let loaded: ReloadContext = serde_json::from_str(&json).unwrap();

    assert_eq!(
        loaded.task_context,
        Some("Testing the reload feature".to_string())
    );
    assert_eq!(loaded.version_before, "v0.1.100");
    assert_eq!(loaded.version_after, "abc1234");
    assert_eq!(loaded.session_id, "test-session-123");
}

#[test]
fn test_reload_context_path() {
    // Just verify the session-scoped path function works
    let path = ReloadContext::path_for_session("test-session-123");
    assert!(path.is_ok());
    let path = path.unwrap();
    let path_str = path.to_string_lossy();
    assert!(path_str.contains("reload-context-test-session-123.json"));
}

#[test]
fn test_reload_context_save_and_load_for_session_uses_session_scoped_file() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let ctx = ReloadContext {
        task_context: Some("Testing scoped reload context".to_string()),
        version_before: "v0.1.100".to_string(),
        version_after: "abc1234".to_string(),
        session_id: "test-session-123".to_string(),
        timestamp: "2025-01-20T00:00:00Z".to_string(),
    };

    ctx.save().expect("save reload context");

    let path = ReloadContext::path_for_session("test-session-123").expect("context path");
    assert!(
        path.exists(),
        "session-scoped reload context file should exist"
    );

    let peeked = ReloadContext::peek_for_session("test-session-123")
        .expect("peek should succeed")
        .expect("context should exist");
    assert_eq!(peeked.session_id, "test-session-123");

    let loaded = ReloadContext::load_for_session("test-session-123")
        .expect("load should succeed")
        .expect("context should exist");
    assert_eq!(loaded.session_id, "test-session-123");
    assert!(
        !path.exists(),
        "load_for_session should consume the context file"
    );
}

#[test]
fn test_recovery_directive_prefers_reload_context_when_present() {
    let ctx = ReloadContext {
        task_context: Some("Resume a self-dev reload".to_string()),
        version_before: "old-build".to_string(),
        version_after: "new-build".to_string(),
        session_id: "session-123".to_string(),
        timestamp: "2026-04-19T00:00:00Z".to_string(),
    };

    let directive = ReloadContext::recovery_directive(
        Some(&ctx),
        true,
        "\nPersisted background task(s) detected.",
        Some(12),
    )
    .expect("directive should exist");

    assert_eq!(
        directive.reconnect_notice.as_deref(),
        Some("Reloaded with build new-build")
    );
    assert!(directive.continuation_message.contains("Reload succeeded"));
    assert!(
        directive
            .continuation_message
            .contains("Persisted background task(s)")
    );
    assert!(
        directive
            .continuation_message
            .contains("Session restored with 12 turns")
    );
}

#[test]
fn test_recovery_directive_uses_interrupted_message_without_reload_context() {
    let directive = ReloadContext::recovery_directive(None, true, "", None)
        .expect("interrupted sessions should get a directive");

    assert!(directive.reconnect_notice.is_none());
    assert!(
        directive
            .continuation_message
            .contains("interrupted by a server reload while a tool was running")
    );
}

#[test]
fn test_recovery_directive_returns_none_when_no_reload_recovery_needed() {
    assert!(ReloadContext::recovery_directive(None, false, "", None).is_none());
}

#[test]
fn reload_timeout_secs_defaults_to_15() {
    let _storage_guard = crate::storage::lock_test_env();
    let _guard = EnvVarGuard::remove("JCODE_SELFDEV_RELOAD_TIMEOUT_SECS");
    assert_eq!(SelfDevTool::reload_timeout_secs(), 15);
}

#[test]
fn reload_timeout_secs_honors_valid_env_override() {
    let _storage_guard = crate::storage::lock_test_env();
    let _guard = EnvVarGuard::set("JCODE_SELFDEV_RELOAD_TIMEOUT_SECS", "27");
    assert_eq!(SelfDevTool::reload_timeout_secs(), 27);
}

#[test]
fn reload_timeout_secs_ignores_empty_invalid_and_zero_values() {
    let _storage_guard = crate::storage::lock_test_env();
    let _guard = EnvVarGuard::set("JCODE_SELFDEV_RELOAD_TIMEOUT_SECS", "   ");
    assert_eq!(SelfDevTool::reload_timeout_secs(), 15);
    drop(_guard);

    let _guard = EnvVarGuard::set("JCODE_SELFDEV_RELOAD_TIMEOUT_SECS", "abc");
    assert_eq!(SelfDevTool::reload_timeout_secs(), 15);
    drop(_guard);

    let _guard = EnvVarGuard::set("JCODE_SELFDEV_RELOAD_TIMEOUT_SECS", "0");
    assert_eq!(SelfDevTool::reload_timeout_secs(), 15);
}

#[test]
fn schema_only_advertises_core_selfdev_fields() {
    // The full (self-dev) schema exposes the build/test/reload surface.
    let schema = SelfDevTool::schema_for(true);
    let props = schema["properties"]
        .as_object()
        .expect("selfdev schema should have properties");

    assert!(props.contains_key("action"));
    assert!(props.contains_key("prompt"));
    assert!(props.contains_key("context"));
    assert!(props.contains_key("reason"));
    assert!(props.contains_key("target"));
    assert!(props.contains_key("command"));
    assert!(props.contains_key("request_id"));
    assert!(props.contains_key("task_id"));
    assert!(!props.contains_key("notify"));
    assert!(!props.contains_key("wake"));

    let actions: Vec<&str> = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for expected in [
        "enter",
        "setup",
        "build",
        "build-reload",
        "test",
        "cancel-build",
        "reload",
        "status",
        "find-config",
        "socket-info",
        "socket-help",
    ] {
        assert!(actions.contains(&expected), "missing action {expected}");
    }
}

#[test]
fn non_selfdev_schema_only_exposes_onramp_actions() {
    // The default schema (what a regular session advertises) is the on-ramp
    // surface: no build/test/socket actions, only enter/setup/reload/status/
    // find-config.
    let default_schema = SelfDevTool::new().parameters_schema();
    let onramp_schema = SelfDevTool::schema_for(false);
    assert_eq!(default_schema, onramp_schema);

    let props = onramp_schema["properties"]
        .as_object()
        .expect("schema properties");
    assert!(props.contains_key("action"));
    assert!(props.contains_key("prompt"));
    // Build/test-only fields are hidden outside self-dev mode.
    assert!(!props.contains_key("reason"));
    assert!(!props.contains_key("target"));
    assert!(!props.contains_key("command"));
    assert!(!props.contains_key("request_id"));
    assert!(!props.contains_key("task_id"));

    let actions: Vec<&str> = onramp_schema["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let mut sorted = actions.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec!["enter", "find-config", "reload", "setup", "status"]
    );
    for hidden in [
        "build",
        "build-reload",
        "test",
        "cancel-build",
        "socket-info",
        "socket-help",
    ] {
        assert!(
            !actions.contains(&hidden),
            "on-ramp schema should not expose {hidden}"
        );
    }
}

#[tokio::test]
async fn test_action_queues_command_in_test_mode() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let tool = SelfDevTool::new();
    let ctx = create_test_context(
        "session-selfdev-test-action",
        Some(repo.path().to_path_buf()),
    );
    let output = tool
        .execute(
            json!({
                "action": "test",
                "command": "cargo test -p jcode selfdev_build_command",
                "reason": "verify selfdev test queue"
            }),
            ctx,
        )
        .await
        .expect("selfdev test should queue");

    assert!(output.output.contains("Self-dev test queued"));
    assert!(
        output
            .output
            .contains("cargo test -p jcode selfdev_build_command")
    );
}

#[tokio::test]
async fn do_reload_returns_after_ack_in_direct_mode() {
    let request_id = server::send_reload_signal("direct-hash".to_string(), None, true);
    let waiter = tokio::spawn({
        let request_id = request_id.clone();
        async move { server::wait_for_reload_ack(&request_id, std::time::Duration::from_secs(1)).await }
    });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    server::acknowledge_reload_signal(&crate::server::ReloadSignal {
        hash: "direct-hash".to_string(),
        triggering_session: None,
        prefer_selfdev_binary: true,
        request_id: "ignored".to_string(),
    });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    server::acknowledge_reload_signal(&crate::server::ReloadSignal {
        hash: "direct-hash".to_string(),
        triggering_session: None,
        prefer_selfdev_binary: true,
        request_id,
    });

    let ack = waiter
        .await
        .expect("waiter task should complete")
        .expect("ack should be received");
    assert_eq!(ack.hash, "direct-hash");
}

#[test]
fn reload_repo_resolver_uses_working_dir_when_primary_detection_fails() {
    let repo = create_repo_fixture();
    let nested = repo.path().join("crates").join("jcode-build-support");
    std::fs::create_dir_all(&nested).expect("nested dir");

    let resolved = reload::resolve_selfdev_reload_repo_dir_from(None, Some(&nested));
    assert_eq!(resolved.as_deref(), Some(repo.path()));
}

#[tokio::test]
async fn enter_creates_selfdev_session_in_test_mode() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut parent = session::Session::create(None, Some("Origin Session".to_string()));
    parent.working_dir = Some("/tmp/origin-project".to_string());
    parent.model = Some("gpt-test".to_string());
    parent.provider_key = Some("openai".to_string());
    parent.subagent_model = Some("gpt-subagent".to_string());
    parent.add_message(
        crate::message::Role::User,
        vec![crate::message::ContentBlock::Text {
            text: "hello from parent".to_string(),
            cache_control: None,
        }],
    );
    parent.compaction = Some(session::StoredCompactionState {
        summary_text: "summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 1,
        original_turn_count: 1,
        compacted_count: 1,
    });
    parent.record_replay_display_message("system", None, "remember this context");
    parent.save().expect("save parent session");

    let tool = SelfDevTool::new();
    let ctx = create_test_context(&parent.id, Some(repo.path().to_path_buf()));
    let output = tool
        .execute(
            json!({"action": "enter", "prompt": "Work on jcode itself"}),
            ctx,
        )
        .await
        .expect("selfdev enter should succeed in test mode");

    assert!(output.output.contains("Created self-dev session"));
    assert!(
        output
            .output
            .contains("Test mode skipped launching a new terminal")
    );
    assert!(
        output.output.contains("Seed prompt captured"),
        "test-mode enter should still report captured prompt"
    );

    let metadata = output.metadata.expect("metadata");
    let session_id = metadata["session_id"]
        .as_str()
        .expect("session id metadata");
    assert_eq!(metadata["inherited_context"].as_bool(), Some(true));
    let session = session::Session::load(session_id).expect("load spawned session");
    assert!(
        session.is_canary,
        "spawned session should be canary/self-dev"
    );
    assert_eq!(session.testing_build.as_deref(), Some("self-dev"));
    assert_eq!(
        session.working_dir.as_deref(),
        Some(repo.path().to_string_lossy().as_ref())
    );
    assert_eq!(session.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(session.messages.len(), parent.messages.len());
    assert_eq!(session.messages[0].content_preview(), "hello from parent");
    assert_eq!(session.compaction, parent.compaction);
    assert_eq!(session.model, parent.model);
    assert_eq!(session.provider_key, parent.provider_key);
    assert_eq!(session.subagent_model, parent.subagent_model);
    assert_eq!(session.replay_events, parent.replay_events);
}

#[tokio::test]
async fn enter_falls_back_to_fresh_session_when_parent_missing() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let tool = SelfDevTool::new();
    let ctx = create_test_context("missing-parent", Some(repo.path().to_path_buf()));
    let output = tool
        .execute(json!({"action": "enter"}), ctx)
        .await
        .expect("selfdev enter should succeed without a persisted parent session");

    let metadata = output.metadata.expect("metadata");
    let session_id = metadata["session_id"]
        .as_str()
        .expect("session id metadata");
    assert_eq!(metadata["inherited_context"].as_bool(), Some(false));

    let session = session::Session::load(session_id).expect("load spawned session");
    assert!(session.messages.is_empty());
    assert!(session.parent_id.is_none());
    assert_eq!(
        session.working_dir.as_deref(),
        Some(repo.path().to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn reload_in_non_selfdev_session_is_upgrade_in_place() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    // Test mode short-circuits the actual server reload signal.
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");

    let mut session = session::Session::create(None, Some("Normal Session".to_string()));
    session.save().expect("save session");

    let tool = SelfDevTool::new();
    let ctx = create_test_context(&session.id, session.working_dir.clone().map(Into::into));
    let output = tool
        .execute(json!({"action": "reload"}), ctx)
        .await
        .expect("reload should route to upgrade-in-place");

    // It must NOT be the old "only available inside a self-dev session" error;
    // a regular session can reload into a newer installed build.
    assert!(
        !output
            .output
            .contains("only available inside a self-dev session")
    );
    assert!(output.output.contains("Test mode"));
}

#[tokio::test]
async fn socket_actions_require_selfdev_session() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Normal Session".to_string()));
    session.save().expect("save session");

    let tool = SelfDevTool::new();
    for action in ["socket-info", "socket-help"] {
        let ctx = create_test_context(&session.id, session.working_dir.clone().map(Into::into));
        let output = tool
            .execute(json!({"action": action}), ctx)
            .await
            .expect("socket action should return guidance instead of failing");
        assert!(
            output
                .output
                .contains("only available inside a self-dev session"),
            "{action} should be gated"
        );
        assert!(output.output.contains("selfdev enter"));
    }
}

#[tokio::test]
async fn find_config_reports_key_paths() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());

    let mut session = session::Session::create(None, Some("Normal Session".to_string()));
    session.save().expect("save session");

    let tool = SelfDevTool::new();
    let ctx = create_test_context(&session.id, None);
    let output = tool
        .execute(json!({"action": "find-config"}), ctx)
        .await
        .expect("find-config should succeed");

    assert!(output.output.contains("Config file:"));
    assert!(output.output.contains("config.toml"));
    assert!(output.output.contains("Build channels"));
    let metadata = output.metadata.expect("find-config metadata");
    assert!(metadata["config_path"].as_str().is_some());
}

#[tokio::test]
async fn setup_reports_dependency_checks() {
    let _storage_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", temp_home.path());
    // Test mode avoids attempting a real git clone when no repo is detected.
    let _test_guard = EnvVarGuard::set("JCODE_TEST_SESSION", "1");
    let repo = create_repo_fixture();

    let mut session = session::Session::create(None, Some("Normal Session".to_string()));
    session.save().expect("save session");

    let tool = SelfDevTool::new();
    let ctx = create_test_context(&session.id, Some(repo.path().to_path_buf()));
    let output = tool
        .execute(json!({"action": "setup"}), ctx)
        .await
        .expect("setup should succeed");

    assert!(output.output.contains("Self-dev setup"));
    assert!(output.output.contains("**cargo**") || output.output.contains("cargo"));
    assert!(output.output.contains("repository"));
    let metadata = output.metadata.expect("setup metadata");
    assert!(metadata["checks"].as_array().is_some());
    // The fixture repo should be detected as the repository.
    assert_eq!(
        metadata["repo_dir"].as_str(),
        Some(repo.path().to_string_lossy().as_ref())
    );
}

include!("tests/build_queue.rs");
