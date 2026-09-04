#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jcode_dap::{
    DebugAdapterConfig, DebugLaunchRequest, DebugOperationConfig, DebugOwnedAttachRequest,
    DebugSessionId, DebugSessionManager, DebugSessionManagerConfig, DebugSessionSnapshot,
    DebugSessionStart, DebugSessionStateKind, DebugWorkspaceKey,
};
use tokio::time::{sleep, timeout};

pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jcode-fake-dap-adapter"))
}

pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "jcode-dap-30e-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn marker(&self, name: &str) {
        fs::write(self.root.join(name), b"").unwrap();
    }

    pub fn source(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, b"fn main() {}\r\n").unwrap();
        path
    }

    pub fn key(&self, name: &str) -> DebugWorkspaceKey {
        DebugWorkspaceKey::new(&self.root, name).unwrap()
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.root.join("fake-dap.log")).unwrap_or_default()
    }

    pub async fn wait_log(&self, needle: &str) {
        self.wait_log_count(needle, 1).await;
    }

    pub async fn wait_log_count(&self, needle: &str, expected: usize) {
        timeout(Duration::from_secs(3), async {
            loop {
                if self.log().matches(needle).count() >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {expected} occurrences of {needle:?}; log:\n{}",
                self.log()
            )
        });
    }

    pub fn logged_pid(&self, key: &str) -> u32 {
        self.log()
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}\t")))
            .unwrap_or_else(|| panic!("missing PID {key}; log:\n{}", self.log()))
            .parse()
            .unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn manager(operation_timeout: Duration) -> DebugSessionManager {
    DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig {
            startup_timeout: Duration::from_secs(2),
            disconnect_timeout: Duration::from_millis(100),
            termination_grace: Duration::from_millis(50),
            process_poll_interval: Duration::from_millis(5),
            ..Default::default()
        },
        DebugOperationConfig {
            operation_timeout,
            ..Default::default()
        },
    )
    .unwrap()
}

pub fn adapter() -> DebugAdapterConfig {
    DebugAdapterConfig::lldb_dap(fixture()).unwrap()
}

pub fn sleep_target(root: &Path) -> PathBuf {
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

pub fn tree_target(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let target = root.join("target tree probe");
    fs::copy(fixture(), &target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    (
        target,
        root.join("target.pid"),
        root.join("target-descendant.pid"),
    )
}

pub async fn launch_stopped(
    workspace: &Workspace,
    manager: &DebugSessionManager,
) -> DebugSessionSnapshot {
    workspace.marker("stop-on-entry");
    manager
        .launch(
            "owner",
            workspace.key("process"),
            &adapter(),
            DebugLaunchRequest::new(sleep_target(&workspace.root))
                .with_arg("30")
                .with_stop_on_entry(true),
        )
        .await
        .unwrap()
}

pub async fn attach_tree(
    workspace: &Workspace,
    manager: &DebugSessionManager,
) -> (DebugSessionSnapshot, u32, u32) {
    workspace.marker("stop-on-entry");
    let (target, target_marker, descendant_marker) = tree_target(&workspace.root);
    let snapshot = manager
        .spawn_and_attach(
            "owner",
            workspace.key("tree"),
            &adapter(),
            DebugOwnedAttachRequest::new(target)
                .with_arg("--target-tree-probe")
                .with_arg(target_marker.to_string_lossy().into_owned())
                .with_arg(descendant_marker.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
    let target_pid = match snapshot.start {
        DebugSessionStart::OwnedAttach { pid, .. } => pid,
        _ => panic!("expected owned attach"),
    };
    let recorded_target = wait_pid_file(&target_marker).await;
    let descendant = wait_pid_file(&descendant_marker).await;
    assert_eq!(target_pid, recorded_target);
    (snapshot, target_pid, descendant)
}

pub async fn wait_state(
    manager: &DebugSessionManager,
    id: DebugSessionId,
    state: DebugSessionStateKind,
) {
    timeout(Duration::from_secs(3), async {
        loop {
            if manager
                .snapshot("owner", id)
                .is_ok_and(|snapshot| snapshot.state.kind() == state)
            {
                break;
            }
            sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

pub async fn wait_pid_file(path: &Path) -> u32 {
    timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(pid) = fs::read_to_string(path).unwrap_or_default().trim().parse() {
                break pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

pub fn process_exists(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap();
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn group_exists(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap();
    let rc = unsafe { libc::kill(-pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub async fn wait_tree_gone(
    adapter_pid: u32,
    target_pid: Option<u32>,
    descendant_pid: Option<u32>,
) {
    timeout(Duration::from_secs(3), async {
        loop {
            let adapter_alive = group_exists(adapter_pid);
            let target_alive = target_pid.is_some_and(group_exists);
            let descendant_alive = descendant_pid.is_some_and(process_exists);
            if !adapter_alive && !target_alive && !descendant_alive {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "process tree survived: adapter={adapter_pid} target={target_pid:?} descendant={descendant_pid:?}"
        )
    });
}

pub fn command_lines(log: &str) -> Vec<(&str, serde_json::Value)> {
    log.lines()
        .filter_map(|line| {
            let (command, body) = line.split_once('\t')?;
            if matches!(
                command,
                "initialize"
                    | "launch"
                    | "attach"
                    | "configurationDone"
                    | "setBreakpoints"
                    | "threads"
                    | "continue"
                    | "pause"
                    | "next"
                    | "stepIn"
                    | "stepOut"
                    | "cancel"
                    | "disconnect"
            ) {
                Some((command, serde_json::from_str(body).unwrap()))
            } else {
                None
            }
        })
        .collect()
}
