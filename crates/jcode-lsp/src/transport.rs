use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::{LspClient, LspError, LspServerConfig, Result, discover_executable};

const DEFAULT_STDERR_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { code: Option<i32> },
}

#[derive(Clone)]
pub struct LspProcess {
    client: LspClient,
    state: Arc<ProcessState>,
    stderr: Arc<Mutex<BoundedOutput>>,
}

struct ProcessState {
    child: tokio::sync::Mutex<Child>,
    pid: Option<u32>,
}

impl Drop for ProcessState {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            terminate_process_group(pid);
        }
    }
}

impl LspProcess {
    pub async fn spawn(config: &LspServerConfig, workspace: &Path) -> Result<Self> {
        Self::spawn_with_path(config, workspace, std::env::var_os("PATH").as_deref()).await
    }

    pub async fn spawn_with_path(
        config: &LspServerConfig,
        workspace: &Path,
        path_env: Option<&OsStr>,
    ) -> Result<Self> {
        let executable = discover_executable(&config.command, path_env, workspace)?;
        let mut command = Command::new(executable);
        command
            .args(&config.args)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        for (key, value) in controlled_environment(path_env) {
            command.env(key, value);
        }
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn()?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or(LspError::MissingProcessPipe { stream: "stdin" })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(LspError::MissingProcessPipe { stream: "stdout" })?;
        let stderr = child
            .stderr
            .take()
            .ok_or(LspError::MissingProcessPipe { stream: "stderr" })?;

        let captured_stderr = Arc::new(Mutex::new(BoundedOutput::new(DEFAULT_STDERR_LIMIT)));
        tokio::spawn(capture_stderr(stderr, Arc::clone(&captured_stderr)));

        Ok(Self {
            client: LspClient::start_split(stdout, stdin),
            state: Arc::new(ProcessState {
                child: tokio::sync::Mutex::new(child),
                pid,
            }),
            stderr: captured_stderr,
        })
    }

    pub fn client(&self) -> &LspClient {
        &self.client
    }

    pub async fn initialize(&self, workspace: &Path, timeout: Duration) -> Result<Value> {
        let root_uri = url::Url::from_directory_path(workspace).map_err(|()| {
            LspError::InvalidWorkspaceUri {
                path: workspace.display().to_string(),
            }
        })?;
        let response = self
            .client
            .request(
                "initialize",
                Some(initialize_params(root_uri.as_str())),
                timeout,
            )
            .await?;
        self.client.notify("initialized", Some(json!({}))).await?;
        Ok(response)
    }

    pub async fn status(&self) -> Result<ProcessStatus> {
        match self.state.child.lock().await.try_wait()? {
            Some(status) => Ok(ProcessStatus::Exited {
                code: status.code(),
            }),
            None => Ok(ProcessStatus::Running),
        }
    }

    pub fn recent_stderr(&self) -> String {
        let output = self
            .stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        String::from_utf8_lossy(&output.bytes.iter().copied().collect::<Vec<_>>()).into_owned()
    }

    pub async fn shutdown(&self, timeout: Duration) {
        if let Err(error) = self.client.request("shutdown", None, timeout).await {
            self.record_shutdown_error("shutdown request", &error);
        }
        if let Err(error) = self.client.notify("exit", None).await {
            self.record_shutdown_error("exit notification", &error);
        }
        let mut child = self.state.child.lock().await;
        let needs_termination = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(_)) => false,
            Ok(Err(error)) => {
                self.record_shutdown_error("wait", &error);
                true
            }
            Err(error) => {
                self.record_shutdown_error("shutdown timeout", &error);
                true
            }
        };
        if needs_termination {
            if let Some(pid) = self.state.pid {
                terminate_process_group(pid);
            }
            if let Err(error) = child.kill().await {
                self.record_shutdown_error("terminate", &error);
            }
        }
    }

    fn record_shutdown_error(&self, operation: &str, error: &impl std::fmt::Display) {
        self.stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("\nLSP {operation} failed: {error}\n").as_bytes());
    }
}

fn initialize_params(root_uri: &str) -> Value {
    json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "workspaceFolders": [{"uri": root_uri, "name": "workspace"}],
        "capabilities": {
            "workspace": {
                "applyEdit": false,
                "configuration": true,
                "workspaceFolders": true,
                "workspaceEdit": {
                    "documentChanges": true,
                    "resourceOperations": ["create", "rename", "delete"]
                },
                "fileOperations": {
                    "dynamicRegistration": true,
                    "didRename": true,
                    "willRename": true
                }
            }
        },
        "clientInfo": {"name": "jcode"}
    })
}

fn controlled_environment(path_env: Option<&OsStr>) -> Vec<(OsString, OsString)> {
    const ALLOWED: &[&str] = &[
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "RUSTC",
        "CARGO",
        "RUSTFLAGS",
        "CARGO_TARGET_DIR",
        "SYSTEMROOT",
        "WINDIR",
    ];
    let mut environment = ALLOWED
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect::<Vec<_>>();
    if let Some(path) = path_env {
        environment.push((OsString::from("PATH"), path.to_owned()));
    }
    environment
}

async fn capture_stderr<R>(mut reader: R, output: Arc<Mutex<BoundedOutput>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => output
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(&buffer[..count]),
        }
    }
}

struct BoundedOutput {
    bytes: VecDeque<u8>,
    limit: usize,
}

impl BoundedOutput {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(
                bytes[bytes.len().saturating_sub(self.limit)..]
                    .iter()
                    .copied(),
            );
            return;
        }
        let excess = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.limit);
        self.bytes.drain(..excess);
        self.bytes.extend(bytes.iter().copied());
    }
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: Negative pid targets only the child-owned process group created at spawn.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_stderr_retains_only_the_newest_bytes() {
        let mut output = BoundedOutput::new(5);
        output.push(b"abc");
        output.push(b"def");
        assert_eq!(output.bytes.into_iter().collect::<Vec<_>>(), b"bcdef");

        let mut output = BoundedOutput::new(3);
        output.push(b"abcdef");
        assert_eq!(output.bytes.into_iter().collect::<Vec<_>>(), b"def");
    }

    #[test]
    fn controlled_environment_does_not_forward_provider_secrets() {
        let environment = controlled_environment(Some(OsStr::new("/bin")));
        assert!(environment.iter().any(|(key, _)| key == "PATH"));
        assert!(
            environment
                .iter()
                .all(|(key, _)| key != "ANTHROPIC_API_KEY")
        );
        assert!(environment.iter().all(|(key, _)| key != "OPENAI_API_KEY"));
    }

    #[test]
    fn initialization_advertises_explicit_file_rename_support_without_implicit_edits() {
        let params = initialize_params("file:///workspace/");
        assert_eq!(params["capabilities"]["workspace"]["applyEdit"], false);
        assert_eq!(
            params["capabilities"]["workspace"]["fileOperations"]["willRename"],
            true
        );
        assert_eq!(
            params["capabilities"]["workspace"]["fileOperations"]["didRename"],
            true
        );
        assert_eq!(params["workspaceFolders"][0]["uri"], "file:///workspace/");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_of_exited_server_retains_failure_diagnostics() {
        let config = LspServerConfig {
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), "exit 0".to_owned()],
            ..Default::default()
        };
        let process = LspProcess::spawn(&config, Path::new("/")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while matches!(process.status().await.unwrap(), ProcessStatus::Running) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        process.shutdown(Duration::from_millis(20)).await;
        assert!(
            process
                .recent_stderr()
                .contains("LSP shutdown request failed:")
        );
        assert!(matches!(
            process.status().await.unwrap(),
            ProcessStatus::Exited { .. }
        ));
    }
}
