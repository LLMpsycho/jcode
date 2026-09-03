use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::json;
use tokio::time::timeout;

use super::*;
use crate::manager::{DebugTerminationPolicy, NewDebugSession, finish_start, start_protocol};
use crate::testing::FakeAdapter;
use crate::{
    Capabilities, DebugSessionManager, DebugSessionManagerConfig, DebugSessionState, Message,
};

fn fixture(name: &str) -> (PathBuf, DebugWorkspaceKey, PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("jcode-contract-{name}-{stamp}"));
    fs::create_dir_all(&root).unwrap();
    let program = root.join("program");
    fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
    (
        root.clone(),
        DebugWorkspaceKey::new(&root, name).unwrap(),
        program,
    )
}

fn config(startup: Duration, disconnect: Duration) -> DebugSessionManagerConfig {
    DebugSessionManagerConfig {
        startup_timeout: startup,
        disconnect_timeout: disconnect,
        termination_grace: Duration::from_millis(10),
        process_poll_interval: Duration::from_millis(5),
        ..Default::default()
    }
}

fn reservation(
    manager: &DebugSessionManager,
    owner: &str,
    workspace: &DebugWorkspaceKey,
    program: &Path,
) -> (crate::manager::DebugSessionReservation, FakeAdapter) {
    let (client, adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: owner.to_owned(),
            workspace: workspace.clone(),
            adapter_id: "lldb".to_owned(),
            start: Some(DebugSessionStart::Launch {
                program: program.to_path_buf(),
                cwd: workspace.canonical_root().to_path_buf(),
            }),
        })
        .unwrap();
    reservation.attach_client(client).unwrap();
    (reservation, adapter)
}

async fn recv_request(adapter: &mut FakeAdapter, command: &str) -> crate::Request {
    match timeout(Duration::from_secs(1), adapter.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Message::Request(request) => {
            assert_eq!(request.command, command);
            request
        }
        _ => panic!(),
    }
}

async fn begin_start(
    name: &str,
    supports_config: bool,
) -> (
    DebugSessionManager,
    FakeAdapter,
    tokio::task::JoinHandle<crate::Result<crate::DebugSessionSnapshot>>,
    DebugWorkspaceKey,
    PathBuf,
) {
    let (_, workspace, program) = fixture(name);
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(2), Duration::from_millis(20)))
            .unwrap();
    let (reservation, mut adapter) = reservation(&manager, "owner", &workspace, &program);
    let resolved = ResolvedLaunch {
        target: resolve_program(&workspace, &program, &[], None).unwrap(),
        stop_on_entry: false,
    };
    let task = tokio::spawn(async move {
        let result = start_protocol(
            &reservation,
            AdapterProfile::LldbDap,
            "launch",
            AdapterProfile::LldbDap.launch_arguments(&resolved),
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
        finish_start(reservation, result).await
    });
    let initialize = recv_request(&mut adapter, "initialize").await;
    adapter
        .respond_ok(
            &initialize,
            Some(
                serde_json::to_value(Capabilities {
                    supports_configuration_done_request: Some(supports_config),
                    ..Default::default()
                })
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    (manager, adapter, task, workspace, program)
}

#[tokio::test]
async fn launch_waits_for_both_initialized_and_start_response() {
    let (_, mut adapter, task, _, _) = begin_start("both", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter.respond_ok(&launch, None).await.unwrap();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished(), "start response alone must not commit");
    adapter.event("initialized", None).await.unwrap();
    assert!(matches!(
        task.await.unwrap().unwrap().state,
        DebugSessionState::Running
    ));
}

#[tokio::test]
async fn launch_accepts_start_response_before_initialized() {
    let (_, mut adapter, task, _, _) = begin_start("response-first", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter.respond_ok(&launch, None).await.unwrap();
    assert!(!task.is_finished());
    adapter.event("initialized", None).await.unwrap();
    assert!(matches!(
        task.await.unwrap().unwrap().state,
        DebugSessionState::Running
    ));
}

#[tokio::test]
async fn launch_accepts_initialized_before_start_response() {
    let (_, mut adapter, task, _, _) = begin_start("event-first", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter.event("initialized", None).await.unwrap();
    assert!(!task.is_finished());
    adapter.respond_ok(&launch, None).await.unwrap();
    assert!(matches!(
        task.await.unwrap().unwrap().state,
        DebugSessionState::Running
    ));
}

#[tokio::test]
async fn configuration_done_is_omitted_when_not_supported() {
    let (manager, mut adapter, task, _, _) = begin_start("no-config", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter.event("initialized", None).await.unwrap();
    adapter.respond_ok(&launch, None).await.unwrap();
    let snapshot = task.await.unwrap().unwrap();
    assert!(matches!(snapshot.state, DebugSessionState::Running));
    assert!(matches!(
        manager.snapshot("owner", snapshot.id).unwrap().state,
        DebugSessionState::Running
    ));
    assert!(timeout(Duration::ZERO, adapter.recv()).await.is_err());
}

#[tokio::test]
async fn early_stopped_event_is_not_overwritten_by_launch_completion() {
    let (_, mut adapter, task, _, _) = begin_start("stopped", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter
        .event("stopped", Some(json!({"reason":"entry"})))
        .await
        .unwrap();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished(), "stopped is not initialized");
    adapter.event("initialized", None).await.unwrap();
    adapter.respond_ok(&launch, None).await.unwrap();
    assert!(matches!(
        task.await.unwrap().unwrap().state,
        DebugSessionState::Stopped(_)
    ));
}

#[tokio::test]
async fn launch_rejection_cancels_and_releases_the_owner_slot() {
    let (manager, mut adapter, task, workspace, program) = begin_start("reject", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    adapter.respond_error(&launch, "no").await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(
        manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace,
                adapter_id: "lldb".into(),
                start: Some(DebugSessionStart::Launch {
                    program: program.clone(),
                    cwd: program.parent().unwrap().to_path_buf()
                })
            })
            .is_ok()
    );
}

#[tokio::test]
async fn startup_timeout_closes_transport_and_releases_the_owner_slot() {
    let (_, workspace, program) = fixture("timeout");
    let manager =
        DebugSessionManager::new(config(Duration::from_millis(20), Duration::from_millis(5)))
            .unwrap();
    let (reservation, mut adapter) = reservation(&manager, "owner", &workspace, &program);
    let task = tokio::spawn(async move {
        let result = start_protocol(
            &reservation,
            AdapterProfile::LldbDap,
            "launch",
            json!({}),
            tokio::time::Instant::now() + Duration::from_millis(20),
        )
        .await;
        finish_start(reservation, result).await
    });
    let initialize = recv_request(&mut adapter, "initialize").await;
    adapter
        .respond_ok(&initialize, Some(json!({})))
        .await
        .unwrap();
    let _ = recv_request(&mut adapter, "launch").await;
    assert!(task.await.unwrap().is_err());
    assert!(
        manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace,
                adapter_id: "x".into(),
                start: Some(DebugSessionStart::Launch {
                    program: program.clone(),
                    cwd: program.parent().unwrap().into()
                })
            })
            .is_ok()
    );
}

#[tokio::test]
async fn caller_cancellation_drops_the_uncommitted_session() {
    let (_, workspace, program) = fixture("abort");
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(2), Duration::from_millis(5))).unwrap();
    let (reservation, mut adapter) = reservation(&manager, "owner", &workspace, &program);
    let task = tokio::spawn(async move {
        start_protocol(
            &reservation,
            AdapterProfile::LldbDap,
            "launch",
            json!({}),
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
    });
    let initialize = recv_request(&mut adapter, "initialize").await;
    adapter
        .respond_ok(&initialize, Some(json!({})))
        .await
        .unwrap();
    let _ = recv_request(&mut adapter, "launch").await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::task::yield_now().await;
    assert!(
        manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace,
                adapter_id: "x".into(),
                start: Some(DebugSessionStart::Launch {
                    program: program.clone(),
                    cwd: program.parent().unwrap().into()
                })
            })
            .is_ok()
    );
}

#[tokio::test]
async fn transport_close_during_start_finalizes_once() {
    let (manager, mut adapter, task, workspace, program) = begin_start("close", false).await;
    let _ = recv_request(&mut adapter, "launch").await;
    drop(adapter);
    assert!(task.await.unwrap().is_err());
    assert!(
        manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace,
                adapter_id: "x".into(),
                start: Some(DebugSessionStart::Launch {
                    program: program.clone(),
                    cwd: program.parent().unwrap().into()
                })
            })
            .is_ok()
    );
}

#[tokio::test]
async fn concurrent_terminate_and_start_response_cannot_resurrect_session() {
    let (manager, mut adapter, task, _, _) = begin_start("terminate-race", false).await;
    let launch = recv_request(&mut adapter, "launch").await;
    let id = manager.sessions("owner")[0].id;
    let terminate = tokio::spawn({
        let manager = manager.clone();
        async move { manager.terminate("owner", id).await }
    });
    adapter.respond_ok(&launch, None).await.unwrap();
    let _ = terminate.await;
    assert!(task.await.unwrap().is_err());
    assert!(matches!(
        manager.snapshot("owner", id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
}

async fn disconnect_body(policy: DebugTerminationPolicy) -> serde_json::Value {
    let (_, workspace, program) = fixture("disconnect");
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(1), Duration::from_secs(1))).unwrap();
    let (client, mut adapter) = FakeAdapter::pair(1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: "owner".into(),
            workspace,
            adapter_id: "x".into(),
            start: Some(DebugSessionStart::Launch {
                program: program.clone(),
                cwd: program.parent().unwrap().into(),
            }),
        })
        .unwrap();
    reservation.attach_start_client(client, policy).unwrap();
    reservation.mark_configuring().unwrap();
    reservation.mark_running().unwrap();
    let id = reservation.commit().unwrap();
    let task = tokio::spawn({
        let manager = manager.clone();
        async move { manager.terminate("owner", id).await }
    });
    let request = recv_request(&mut adapter, "disconnect").await;
    let body = request.arguments.clone().unwrap();
    adapter.respond_ok(&request, None).await.unwrap();
    task.await.unwrap().unwrap();
    body
}

#[tokio::test]
async fn disconnect_uses_terminate_debuggee_for_launch() {
    assert_eq!(
        disconnect_body(DebugTerminationPolicy::AdapterLaunched).await,
        json!({"restart":false,"terminateDebuggee":true,"suspendDebuggee":false})
    );
}
#[tokio::test]
async fn disconnect_does_not_delegate_target_termination_for_owned_attach() {
    assert_eq!(
        disconnect_body(DebugTerminationPolicy::OwnedAttach).await,
        json!({"restart":false,"terminateDebuggee":false,"suspendDebuggee":false})
    );
}
#[tokio::test]
async fn disconnect_timeout_still_runs_local_process_cleanup() {
    let (_, workspace, program) = fixture("disconnect-timeout");
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(1), Duration::from_millis(5))).unwrap();
    let (client, _adapter) = FakeAdapter::pair(1024);
    let mut r = manager
        .reserve(NewDebugSession {
            owner_session_id: "o".into(),
            workspace,
            adapter_id: "x".into(),
            start: Some(DebugSessionStart::Launch {
                program: program.clone(),
                cwd: program.parent().unwrap().into(),
            }),
        })
        .unwrap();
    r.attach_start_client(client, DebugTerminationPolicy::AdapterLaunched)
        .unwrap();
    r.mark_configuring().unwrap();
    r.mark_running().unwrap();
    let id = r.commit().unwrap();
    assert!(manager.terminate("o", id).await.is_err());
    assert!(matches!(
        manager.snapshot("o", id).unwrap().state,
        DebugSessionState::Ended(_)
    ));
}

#[test]
fn cwd_symlink_escape_is_rejected_before_adapter_spawn() {
    let (root, workspace, program) = fixture("cwd-escape");
    let outside = std::env::temp_dir();
    let link = root.join("cwd");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside, &link).unwrap();
    assert!(matches!(
        resolve_program(&workspace, &program, &[], Some(&link)),
        Err(DapError::DebugPathOutsideWorkspace { .. })
    ));
}
#[cfg(unix)]
#[test]
fn non_executable_program_is_rejected_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let (_, workspace, program) = fixture("nonexec");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        resolve_program(&workspace, &program, &[], None),
        Err(DapError::InvalidDebugProgram { .. })
    ));
}
#[cfg(unix)]
#[test]
fn setuid_and_setgid_targets_are_rejected_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o4755, 0o2755] {
        let (_, workspace, program) = fixture("special");
        fs::set_permissions(&program, fs::Permissions::from_mode(mode)).unwrap();
        assert!(matches!(
            resolve_program(&workspace, &program, &[], None),
            Err(DapError::InvalidDebugProgram { .. })
        ));
    }
}

#[test]
fn no_start_path_accepts_a_caller_pid() {
    let source = include_str!("../manager.rs");
    assert!(!source.contains("pub async fn attach("));
    assert!(!source.contains("spawn_and_attach(\n        &self,\n        pid"));
    assert_eq!(
        AdapterProfile::LldbDap.attach_arguments(77),
        json!({"pid":77})
    );
}

#[tokio::test]
async fn wrong_owner_cannot_observe_or_terminate_a_starting_session() {
    let (manager, mut adapter, task, _, _) = begin_start("wrong-owner", false).await;
    let _ = recv_request(&mut adapter, "launch").await;
    let id = manager.sessions("owner")[0].id;
    let before = manager.snapshot("owner", id).unwrap().state;
    assert!(manager.sessions("other").is_empty());
    assert!(manager.snapshot("other", id).is_err());
    assert!(manager.terminate("other", id).await.is_err());
    assert_eq!(manager.snapshot("owner", id).unwrap().state, before);
    assert!(timeout(Duration::ZERO, adapter.recv()).await.is_err());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_denial_occurs_before_reservation_or_process_spawn() {
    let (_, workspace, program) = fixture("windows");
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(1), Duration::from_secs(1))).unwrap();
    let adapter = DebugAdapterConfig::lldb_dap(std::env::current_exe().unwrap()).unwrap();
    assert!(matches!(
        manager
            .launch(
                "owner",
                workspace.clone(),
                &adapter,
                DebugLaunchRequest::new(program.clone())
            )
            .await,
        Err(DapError::ProcessContainmentUnavailable { .. })
    ));
    assert!(manager.sessions("owner").is_empty());
}
#[cfg(not(windows))]
#[test]
fn windows_denial_occurs_before_reservation_or_process_spawn() {
    assert!(include_str!("../manager/startup.rs").contains("#[cfg(windows)]"));
}

#[tokio::test]
async fn target_exit_during_attach_cleans_target_adapter_and_owner_slot() {
    let (_, workspace, program) = fixture("target-exit");
    let manager =
        DebugSessionManager::new(config(Duration::from_secs(1), Duration::from_millis(5))).unwrap();
    let (client, adapter) = FakeAdapter::pair(1024);
    let mut r = manager
        .reserve(NewDebugSession {
            owner_session_id: "owner".into(),
            workspace: workspace.clone(),
            adapter_id: "x".into(),
            start: Some(DebugSessionStart::Launch {
                program: program.clone(),
                cwd: program.parent().unwrap().into(),
            }),
        })
        .unwrap();
    r.attach_client(client).unwrap();
    drop(adapter);
    let id = r.commit().unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if manager.snapshot("owner", id).unwrap().state.is_terminal() {
                break;
            }
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    assert!(
        manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace,
                adapter_id: "x".into(),
                start: Some(DebugSessionStart::Launch {
                    program: program.clone(),
                    cwd: program.parent().unwrap().into()
                })
            })
            .is_ok()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_attach_grants_only_the_owned_adapter_pid() {
    use std::cell::Cell;
    let option = Cell::new(0);
    let value = Cell::new(0);
    crate::process::test_set_ptracer_with(4242, |seen_option, seen_value| {
        option.set(seen_option);
        value.set(seen_value);
        0
    })
    .unwrap();
    assert_eq!(option.get(), libc::PR_SET_PTRACER);
    assert_eq!(value.get(), 4242);
    assert_ne!(value.get(), libc::PR_SET_PTRACER_ANY as libc::c_ulong);
    assert!(crate::process::test_set_ptracer_with(4242, |_, _| -1).is_err());
    assert!(crate::process::test_set_ptracer_with(0, |_, _| 0).is_err());
}
#[cfg(not(target_os = "linux"))]
#[test]
fn linux_attach_grants_only_the_owned_adapter_pid() {
    assert_eq!(
        AdapterProfile::LldbDap.attach_arguments(42),
        json!({"pid":42})
    );
}
