#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jcode_dap::{
    DebugAdapterConfig, DebugContinueRequest, DebugLaunchRequest, DebugOperationConfig,
    DebugPauseRequest, DebugRemoveBreakpointRequest, DebugSessionManager,
    DebugSessionManagerConfig, DebugSessionStateKind, DebugSetBreakpointRequest,
    DebugSourceBreakpoint, DebugStepRequest, DebugWorkspaceKey,
};
use tokio::time::{sleep, timeout};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jcode-fake-dap-adapter"))
}
fn workspace() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-30e-process-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
fn target(root: &Path) -> PathBuf {
    let source = if Path::new("/bin/sleep").exists() {
        Path::new("/bin/sleep")
    } else {
        Path::new("/usr/bin/sleep")
    };
    let target = root.join("target program");
    fs::copy(source, &target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    target
}
async fn wait_state(
    manager: &DebugSessionManager,
    id: jcode_dap::DebugSessionId,
    state: DebugSessionStateKind,
) {
    timeout(Duration::from_secs(2), async {
        loop {
            if manager.snapshot("owner", id).unwrap().state.kind() == state {
                break;
            }
            sleep(Duration::from_millis(2)).await
        }
    })
    .await
    .unwrap()
}
fn process_group_exists(pid: u32) -> bool {
    let rc = unsafe { libc::kill(-i32::try_from(pid).unwrap(), 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[tokio::test]
async fn real_subprocess_full_breakpoint_and_control_contract_repeats_cleanly() {
    for iteration in 0..2 {
        let root = workspace();
        fs::write(root.join("stop-on-entry"), b"").unwrap();
        let source = root.join(format!("unicode source {iteration} λ.rs"));
        fs::write(&source, b"fn main() {}\r\n").unwrap();
        let program = target(&root);
        let workspace = DebugWorkspaceKey::new(&root, "process").unwrap();
        let adapter = DebugAdapterConfig::lldb_dap(fixture()).unwrap();
        let manager = DebugSessionManager::new_with_operation_config(
            DebugSessionManagerConfig {
                startup_timeout: Duration::from_secs(2),
                disconnect_timeout: Duration::from_millis(100),
                termination_grace: Duration::from_millis(50),
                process_poll_interval: Duration::from_millis(5),
                ..Default::default()
            },
            DebugOperationConfig {
                operation_timeout: Duration::from_secs(2),
                ..Default::default()
            },
        )
        .unwrap();
        let launched = manager
            .launch(
                "owner",
                workspace,
                &adapter,
                DebugLaunchRequest::new(&program)
                    .with_arg("30")
                    .with_stop_on_entry(true),
            )
            .await
            .unwrap();
        assert_eq!(launched.state.kind(), DebugSessionStateKind::Stopped);
        let first = manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(&source, DebugSourceBreakpoint::new(1)),
            )
            .await
            .unwrap();
        let first_id = match first.mutation {
            jcode_dap::DebugBreakpointMutation::Created { breakpoint_id } => breakpoint_id,
            _ => panic!(),
        };
        manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(
                    &source,
                    DebugSourceBreakpoint::new(2).with_condition("x == 1"),
                ),
            )
            .await
            .unwrap();
        manager
            .remove_breakpoint(
                "owner",
                launched.id,
                DebugRemoveBreakpointRequest::new(first_id),
            )
            .await
            .unwrap();
        let threads = manager.threads("owner", launched.id).await.unwrap();
        assert_eq!(threads.threads.len(), 2);
        manager
            .continue_execution("owner", launched.id, DebugContinueRequest::default())
            .await
            .unwrap();
        assert_eq!(
            manager.snapshot("owner", launched.id).unwrap().state.kind(),
            DebugSessionStateKind::Running
        );
        fs::write(root.join("control-stops"), b"").unwrap();
        manager
            .pause(
                "owner",
                launched.id,
                DebugPauseRequest::default().with_thread_id(threads.threads[0].id),
            )
            .await
            .unwrap();
        wait_state(&manager, launched.id, DebugSessionStateKind::Stopped).await;
        for step in 0..3 {
            match step {
                0 => manager
                    .step_over("owner", launched.id, DebugStepRequest::default())
                    .await
                    .unwrap(),
                1 => manager
                    .step_in("owner", launched.id, DebugStepRequest::default())
                    .await
                    .unwrap(),
                _ => manager
                    .step_out("owner", launched.id, DebugStepRequest::default())
                    .await
                    .unwrap(),
            };
            wait_state(&manager, launched.id, DebugSessionStateKind::Stopped).await;
        }
        let log = fs::read_to_string(root.join("fake-dap.log")).unwrap();
        assert!(log.contains("setBreakpoints\t"));
        assert!(log.contains(source.to_str().unwrap()));
        assert!(log.contains("\"line\":1") && log.contains("\"line\":2"));
        for command in [
            "threads\t",
            "continue\t",
            "pause\t",
            "next\t",
            "stepIn\t",
            "stepOut\t",
        ] {
            assert!(log.contains(command), "missing {command}");
        }
        let adapter_pid = log
            .lines()
            .find_map(|line| line.strip_prefix("adapter_pid\t"))
            .unwrap()
            .parse::<u32>()
            .unwrap();
        manager.terminate("owner", launched.id).await.unwrap();
        timeout(Duration::from_secs(2), async {
            while process_group_exists(adapter_pid) {
                sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        let _ = fs::remove_dir_all(root);
    }
}
