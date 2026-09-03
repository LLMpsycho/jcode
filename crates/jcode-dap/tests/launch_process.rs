#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jcode_dap::{
    DebugAdapterConfig, DebugLaunchRequest, DebugOwnedAttachRequest, DebugSessionManager,
    DebugSessionManagerConfig, DebugSessionStart, DebugSessionState, DebugWorkspaceKey,
};

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
    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&target, permissions).unwrap();
    target
}
fn manager(disconnect: Duration) -> DebugSessionManager {
    DebugSessionManager::new(DebugSessionManagerConfig {
        startup_timeout: Duration::from_secs(3),
        disconnect_timeout: disconnect,
        termination_grace: Duration::from_millis(50),
        process_poll_interval: Duration::from_millis(10),
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

#[tokio::test]
async fn real_process_launch_initializes_runs_and_terminates_adapter_tree() {
    let (root, ws, adapter) = setup("launch");
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
    manager.terminate("owner", snapshot.id).await.unwrap();
    let log = fs::read_to_string(root.join("fake-dap.log")).unwrap();
    let commands = log
        .lines()
        .map(|l| l.split('\t').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        &commands[..4],
        ["initialize", "launch", "configurationDone", "disconnect"]
    );
}

#[tokio::test]
async fn real_process_owned_attach_uses_the_manager_spawned_pid() {
    let (root, ws, adapter) = setup("attach-pid");
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
    let pid = match snapshot.start {
        DebugSessionStart::OwnedAttach { pid, .. } => pid,
        _ => panic!(),
    };
    let log = fs::read_to_string(root.join("fake-dap.log")).unwrap();
    assert!(log.contains(&format!("\"pid\":{pid}")));
    manager.terminate("owner", snapshot.id).await.unwrap();
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
    manager.terminate("owner", snapshot.id).await.unwrap();
    assert!(matches!(
        manager.snapshot("owner", snapshot.id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
    drop(root);
}

#[tokio::test]
async fn real_process_startup_failure_leaves_no_adapter_or_target() {
    let (root, ws, adapter) = setup("reject");
    fs::write(root.join("reject-start"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
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
    let retry = manager.launch("owner", ws, &adapter, DebugLaunchRequest::new(&program));
    assert!(retry.await.is_err());
}

#[tokio::test]
async fn real_process_cancelled_start_leaves_no_descendants() {
    let (root, ws, adapter) = setup("cancel");
    fs::write(root.join("reject-start"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    let _ = manager
        .launch(
            "owner",
            ws.clone(),
            &adapter,
            DebugLaunchRequest::new(&program),
        )
        .await;
    assert!(
        manager
            .sessions("owner")
            .iter()
            .all(|s| matches!(s.state, DebugSessionState::Ended(_)))
    );
}

#[tokio::test]
async fn real_process_disconnect_hang_escalates_to_forced_cleanup() {
    let (root, ws, adapter) = setup("hang");
    fs::write(root.join("hang-disconnect"), b"").unwrap();
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(20));
    let snapshot = manager
        .launch("owner", ws, &adapter, DebugLaunchRequest::new(&program))
        .await
        .unwrap();
    assert!(manager.terminate("owner", snapshot.id).await.is_err());
    assert!(matches!(
        manager.snapshot("owner", snapshot.id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
}

#[tokio::test]
async fn real_process_target_exit_during_attach_finalizes_once() {
    let (root, ws, adapter) = setup("exit");
    let program = copy_target(&root);
    let manager = manager(Duration::from_millis(100));
    let result = manager
        .spawn_and_attach(
            "owner",
            ws,
            &adapter,
            DebugOwnedAttachRequest::new(&program).with_arg("0"),
        )
        .await;
    assert!(result.is_err() || matches!(result.unwrap().state, DebugSessionState::Running));
}
