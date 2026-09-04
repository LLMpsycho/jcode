#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jcode_dap::{
    DebugAdapterConfig, DebugLaunchRequest, DebugOwnedAttachRequest, DebugSessionManager,
    DebugSessionManagerConfig, DebugSessionStart, DebugSessionState, DebugWorkspaceKey,
};
use tokio::time::timeout;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jcode-fake-dap-adapter"))
}
fn workspace(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("jcode-dap-{name}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn copy_target(root: &Path) -> PathBuf {
    let source = if Path::new("/bin/sleep").exists() {
        Path::new("/bin/sleep")
    } else {
        Path::new("/usr/bin/sleep")
    };
    let target = root.join("target-program");
    fs::copy(source, &target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    target
}
fn manager(disconnect: Duration) -> DebugSessionManager {
    DebugSessionManager::new(DebugSessionManagerConfig {
        startup_timeout: Duration::from_millis(500),
        disconnect_timeout: disconnect,
        termination_grace: Duration::from_millis(30),
        process_poll_interval: Duration::from_millis(5),
        ..Default::default()
    })
    .unwrap()
}
fn setup(name: &str) -> (PathBuf, DebugWorkspaceKey, DebugAdapterConfig) {
    let root = workspace(name);
    let ws = DebugWorkspaceKey::new(&root, name).unwrap();
    let adapter = DebugAdapterConfig::lldb_dap(fixture()).unwrap();
    (root, ws, adapter)
}
fn log(root: &Path) -> String {
    fs::read_to_string(root.join("fake-dap.log")).unwrap_or_default()
}
async fn wait_log(root: &Path, needle: &str) {
    timeout(Duration::from_secs(2), async {
        loop {
            if log(root).contains(needle) {
                break;
            }
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap()
}
fn logged_pid(root: &Path, key: &str) -> u32 {
    log(root)
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}\t")))
        .unwrap()
        .parse()
        .unwrap()
}
fn group_exists(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap();
    let rc = unsafe { libc::kill(-pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
fn process_exists(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap();
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
async fn wait_group_gone(pid: u32) {
    timeout(Duration::from_secs(2), async {
        while group_exists(pid) {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap()
}
async fn wait_process_gone(pid: u32) {
    timeout(Duration::from_secs(2), async {
        while process_exists(pid) {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_process_launch_initializes_runs_and_terminates_adapter_tree() {
    let (root, ws, adapter) = setup("launch");
    fs::write(root.join("launch-descendant"), b"").unwrap();
    fs::write(root.join("response-first"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    let snapshot = manager
        .launch(
            "owner",
            ws,
            &adapter,
            DebugLaunchRequest::new(&program).with_arg("30"),
        )
        .await
        .unwrap();
    assert!(matches!(snapshot.state, DebugSessionState::Running));
    wait_log(&root, "descendant_pid\t").await;
    let adapter_pid = logged_pid(&root, "adapter_pid");
    let descendant_pid = logged_pid(&root, "descendant_pid");
    assert!(group_exists(adapter_pid));
    manager.terminate("owner", snapshot.id).await.unwrap();
    wait_group_gone(adapter_pid).await;
    wait_process_gone(descendant_pid).await;
    let text = log(&root);
    assert!(text.contains("initialize\t"));
    assert!(text.contains("launch\t"));
    assert!(text.contains("configurationDone\t"));
    assert!(text.contains("\"terminateDebuggee\":true"));
}

#[tokio::test]
async fn real_process_lifecycle_system_pid_is_informational_only() {
    let (root, ws, adapter) = setup("lifecycle");
    fs::write(root.join("emit-lifecycle"), b"").unwrap();
    let mut sentinel = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    fs::write(root.join("system-process-id"), sentinel.id().to_string()).unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    let result = manager
        .launch("owner", ws, &adapter, DebugLaunchRequest::new(&program))
        .await;
    assert!(result.is_err());
    let snapshot = manager.sessions("owner").into_iter().next().unwrap();
    assert!(matches!(snapshot.state, DebugSessionState::Ended(_)));
    let output = manager.output("owner", snapshot.id, None, 10).unwrap();
    assert!(
        output
            .records
            .iter()
            .any(|item| item.output.contains("fixture output"))
    );
    assert!(
        process_exists(sentinel.id()),
        "DAP systemProcessId must not become kill authority"
    );
    let adapter_pid = logged_pid(&root, "adapter_pid");
    wait_group_gone(adapter_pid).await;
    sentinel.kill().unwrap();
    sentinel.wait().unwrap();
}

#[tokio::test]
async fn real_process_owned_attach_uses_the_manager_spawned_pid() {
    let (root, ws, adapter) = setup("attach-pid");
    let program = root.join("target-probe");
    fs::copy(fixture(), &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let marker = root.join("target-pid");
    let manager = manager(Duration::from_millis(100));
    let snapshot = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program)
                .with_arg("--target-probe")
                .with_arg(marker.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
    let pid = match snapshot.start {
        DebugSessionStart::OwnedAttach { pid, .. } => pid,
        _ => panic!(),
    };
    wait_log(&root, &format!("verified_attach_pid\t{pid}")).await;
    let target_recorded_pid = timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(pid) = fs::read_to_string(&marker)
                .unwrap_or_default()
                .parse::<u32>()
            {
                break pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let adapter_received_pid = logged_pid(&root, "verified_attach_pid");
    assert_eq!(pid, target_recorded_pid);
    assert_eq!(pid, adapter_received_pid);
    assert!(group_exists(pid));
    manager.terminate("owner", snapshot.id).await.unwrap();
    wait_group_gone(pid).await;
    assert!(log(&root).contains("\"terminateDebuggee\":false"));
}

#[tokio::test]
async fn real_process_owned_attach_terminates_target_and_adapter_groups() {
    let (root, ws, adapter) = setup("attach-clean");
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    let snapshot = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program).with_arg("30"),
        )
        .await
        .unwrap();
    let target_pid = match snapshot.start {
        DebugSessionStart::OwnedAttach { pid, .. } => pid,
        _ => panic!(),
    };
    let adapter_pid = logged_pid(&root, "adapter_pid");
    manager.terminate("owner", snapshot.id).await.unwrap();
    wait_group_gone(target_pid).await;
    wait_group_gone(adapter_pid).await;
    assert!(matches!(
        manager.snapshot("owner", snapshot.id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
}

#[tokio::test]
async fn real_process_startup_failure_leaves_no_adapter_or_target() {
    let (root, ws, adapter) = setup("reject");
    fs::write(root.join("reject-start"), b"").unwrap();
    fs::write(root.join("reject-stderr"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(500));
    let error = manager
        .launch(
            "owner",
            ws.clone(),
            &adapter,
            DebugLaunchRequest::new(&program),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("fixture rejection"));
    assert!(error.to_string().contains("authorization denied"));
    let adapter_pid = logged_pid(&root, "adapter_pid");
    wait_group_gone(adapter_pid).await;
    fs::remove_file(root.join("reject-start")).unwrap();
    fs::remove_file(root.join("reject-stderr")).unwrap();
    let retry = manager
        .launch("owner", ws, &adapter, DebugLaunchRequest::new(&program))
        .await
        .unwrap();
    assert!(matches!(retry.state, DebugSessionState::Running));
    manager.terminate("owner", retry.id).await.unwrap();
}

#[tokio::test]
async fn real_process_owned_attach_rejection_cleans_both_groups_and_retries() {
    let (root, ws, adapter) = setup("attach-reject");
    fs::write(root.join("reject-start"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    assert!(
        manager
            .spawn_and_attach(
                "owner",
                ws.clone(),
                &adapter,
                DebugOwnedAttachRequest::new(&program).with_arg("30")
            )
            .await
            .is_err()
    );
    let adapter_pid = logged_pid(&root, "adapter_pid");
    let attach_line = log(&root)
        .lines()
        .find(|line| line.starts_with("attach\t"))
        .unwrap()
        .to_owned();
    let arguments: serde_json::Value =
        serde_json::from_str(attach_line.split_once('\t').unwrap().1).unwrap();
    let target_pid = arguments["pid"].as_u64().unwrap() as u32;
    wait_group_gone(adapter_pid).await;
    wait_group_gone(target_pid).await;
    fs::remove_file(root.join("reject-start")).unwrap();
    let retry = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program).with_arg("30"),
        )
        .await
        .unwrap();
    manager.terminate("owner", retry.id).await.unwrap();
}

#[tokio::test]
async fn real_process_cancelled_start_leaves_no_descendants() {
    let (root, ws, adapter) = setup("cancel");
    fs::write(root.join("hang-start"), b"").unwrap();
    fs::write(root.join("launch-descendant"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_secs(1));
    let task = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .launch("owner", ws, &adapter, DebugLaunchRequest::new(&program))
                .await
        }
    });
    wait_log(&root, "descendant_pid\t").await;
    let adapter_pid = logged_pid(&root, "adapter_pid");
    let descendant_pid = logged_pid(&root, "descendant_pid");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_group_gone(adapter_pid).await;
    wait_process_gone(descendant_pid).await;
    assert!(manager.sessions("owner").is_empty());
}

#[tokio::test]
async fn real_process_disconnect_hang_escalates_to_forced_cleanup() {
    let (root, ws, adapter) = setup("hang");
    fs::write(root.join("hang-disconnect"), b"").unwrap();
    fs::write(root.join("launch-descendant"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(20));
    let snapshot = manager
        .launch("owner", ws, &adapter, DebugLaunchRequest::new(&program))
        .await
        .unwrap();
    let adapter_pid = logged_pid(&root, "adapter_pid");
    let descendant_pid = logged_pid(&root, "descendant_pid");
    assert!(manager.terminate("owner", snapshot.id).await.is_err());
    wait_group_gone(adapter_pid).await;
    wait_process_gone(descendant_pid).await;
    assert!(matches!(
        manager.snapshot("owner", snapshot.id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
}

#[tokio::test]
async fn real_process_target_exit_during_attach_finalizes_once() {
    let (root, ws, adapter) = setup("exit");
    fs::write(root.join("signal-target-exit"), b"").unwrap();
    let program = root.join("target-exit-probe");
    fs::copy(fixture(), &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let pid_marker = root.join("target-pid");
    let exit_marker = root.join("attach-seen");
    let manager = manager(Duration::from_millis(500));
    let result = manager
        .spawn_and_attach(
            "owner",
            ws.clone(),
            &adapter,
            DebugOwnedAttachRequest::new(&program)
                .with_arg("--target-exit-after")
                .with_arg(pid_marker.to_string_lossy().into_owned())
                .with_arg(exit_marker.to_string_lossy().into_owned()),
        )
        .await;
    assert!(result.is_err());
    let target_pid = timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(pid) = fs::read_to_string(&pid_marker)
                .unwrap_or_default()
                .parse::<u32>()
            {
                break pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    wait_group_gone(target_pid).await;
    let adapter_pid = logged_pid(&root, "adapter_pid");
    wait_group_gone(adapter_pid).await;
    assert!(matches!(
        manager.sessions("owner")[0].state,
        DebugSessionState::Ended(_)
    ));
    assert_eq!(manager.sessions("owner").len(), 1);
    fs::remove_file(root.join("signal-target-exit")).unwrap();
    let program = copy_target(&root);
    let retry = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program).with_arg("30"),
        )
        .await
        .unwrap();
    manager.terminate("owner", retry.id).await.unwrap();
}

#[tokio::test]
async fn real_process_dead_adapter_with_open_pipes_cannot_spawn_owned_target() {
    let (root, ws, adapter) = setup("dead-adapter");
    fs::write(root.join("exit-after-initialize-open-pipes"), b"").unwrap();
    let program = root.join("target-probe");
    fs::copy(fixture(), &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let marker = root.join("target-started");
    let manager = manager(Duration::from_millis(100));
    let error = manager
        .spawn_and_attach(
            "owner",
            ws.clone(),
            &adapter,
            DebugOwnedAttachRequest::new(&program)
                .with_arg("--target-probe")
                .with_arg(marker.to_string_lossy().into_owned()),
        )
        .await
        .unwrap_err();
    assert!(!error.to_string().is_empty());
    let adapter_pid = logged_pid(&root, "adapter_pid");
    let holder_pid = logged_pid(&root, "pipe_holder_pid");
    wait_group_gone(adapter_pid).await;
    wait_process_gone(holder_pid).await;
    assert!(!log(&root).contains("attach\t"));
    assert!(!marker.exists());

    fs::remove_file(root.join("exit-after-initialize-open-pipes")).unwrap();
    let program = copy_target(&root);
    let retry = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program).with_arg("30"),
        )
        .await
        .unwrap();
    manager.terminate("owner", retry.id).await.unwrap();
}

#[tokio::test]
async fn real_process_invalid_inputs_are_rejected_before_adapter_spawn() {
    let (root, ws, adapter) = setup("pre-spawn-denial");
    let manager = manager(Duration::from_millis(100));
    let outside = workspace("outside-target");
    let outside_program = copy_target(&outside);
    let program_link = root.join("program-link");
    std::os::unix::fs::symlink(&outside_program, &program_link).unwrap();
    assert!(
        manager
            .launch(
                "owner",
                ws.clone(),
                &adapter,
                DebugLaunchRequest::new(&program_link)
            )
            .await
            .is_err()
    );

    let program = copy_target(&root);
    let cwd_link = root.join("cwd-link");
    std::os::unix::fs::symlink(&outside, &cwd_link).unwrap();
    assert!(
        manager
            .launch(
                "owner",
                ws.clone(),
                &adapter,
                DebugLaunchRequest::new(&program).with_cwd(&cwd_link)
            )
            .await
            .is_err()
    );

    fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        manager
            .launch(
                "owner",
                ws.clone(),
                &adapter,
                DebugLaunchRequest::new(&program)
            )
            .await
            .is_err()
    );
    fs::set_permissions(&program, fs::Permissions::from_mode(0o4755)).unwrap();
    assert!(
        manager
            .spawn_and_attach(
                "owner",
                ws,
                &adapter,
                DebugOwnedAttachRequest::new(&program)
            )
            .await
            .is_err()
    );

    assert!(!root.join("fake-dap.log").exists());
    assert!(manager.sessions("owner").is_empty());
}
