use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::{DapClient, DapError, Result};

const DEFAULT_STDERR_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct AdapterCommand {
    command: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    stderr_limit: usize,
}

impl AdapterCommand {
    pub fn new(command: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            environment: controlled_environment(std::env::var_os("PATH").as_deref()),
            stderr_limit: DEFAULT_STDERR_LIMIT,
        }
    }

    pub fn with_arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }
    pub fn with_stderr_limit(mut self, limit: usize) -> Self {
        self.stderr_limit = limit;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { code: Option<i32> },
}

pub struct AdapterProcess {
    client: DapClient,
    state: Arc<ProcessState>,
    stderr: Arc<Mutex<BoundedOutput>>,
}

struct ProcessState {
    child: tokio::sync::Mutex<Child>,
    pid: AtomicU32,
}

impl Drop for ProcessState {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            force_process_group(pid);
        }
    }
}

impl ProcessState {
    fn pid(&self) -> Option<u32> {
        match self.pid.load(Ordering::Acquire) {
            0 => None,
            pid => Some(pid),
        }
    }

    fn mark_reaped(&self) {
        self.pid.store(0, Ordering::Release);
    }

    fn cleanup_reaped_group(&self) -> Result<()> {
        if let Some(pid) = self.pid() {
            kill_process_group(pid)?;
        }
        self.mark_reaped();
        Ok(())
    }
}

impl AdapterProcess {
    pub async fn spawn(config: &AdapterCommand) -> Result<Self> {
        if !config.command.is_absolute() {
            return Err(DapError::InvalidMessage(
                "adapter command must be an absolute path".to_owned(),
            ));
        }
        if !config.cwd.is_absolute() {
            return Err(DapError::InvalidMessage(
                "adapter cwd must be an absolute path".to_owned(),
            ));
        }
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        command.envs(&config.environment);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn()?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or(DapError::MissingProcessPipe { stream: "stdin" })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(DapError::MissingProcessPipe { stream: "stdout" })?;
        let stderr = child
            .stderr
            .take()
            .ok_or(DapError::MissingProcessPipe { stream: "stderr" })?;
        let captured = Arc::new(Mutex::new(BoundedOutput::new(config.stderr_limit)));
        tokio::spawn(capture_stderr(stderr, Arc::clone(&captured)));
        Ok(Self {
            client: DapClient::start_split(stdout, stdin),
            state: Arc::new(ProcessState {
                child: tokio::sync::Mutex::new(child),
                pid: AtomicU32::new(pid.unwrap_or(0)),
            }),
            stderr: captured,
        })
    }

    pub fn client(&self) -> &DapClient {
        &self.client
    }

    pub async fn status(&self) -> Result<ProcessStatus> {
        match self.state.child.lock().await.try_wait()? {
            Some(status) => {
                self.state.cleanup_reaped_group()?;
                Ok(ProcessStatus::Exited {
                    code: status.code(),
                })
            }
            None => Ok(ProcessStatus::Running),
        }
    }

    pub fn stderr_capture_error(&self) -> Option<String> {
        self.stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .error
            .clone()
    }

    pub fn recent_stderr(&self) -> Vec<u8> {
        self.stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .bytes
            .iter()
            .copied()
            .collect()
    }

    pub async fn terminate(&self, grace: Duration) -> Result<ProcessStatus> {
        let mut child = self.state.child.lock().await;
        if let Some(status) = child.try_wait()? {
            self.state.cleanup_reaped_group()?;
            return Ok(ProcessStatus::Exited {
                code: status.code(),
            });
        }
        if let Some(pid) = self.state.pid() {
            terminate_process_group(pid)?;
        }
        match tokio::time::timeout(grace, child.wait()).await {
            Ok(status) => {
                let status = status?;
                self.state.cleanup_reaped_group()?;
                Ok(ProcessStatus::Exited {
                    code: status.code(),
                })
            }
            Err(_) => {
                if let Some(pid) = self.state.pid() {
                    kill_process_group(pid)?;
                }
                #[cfg(not(unix))]
                child.start_kill()?;
                let status = child.wait().await?;
                self.state.cleanup_reaped_group()?;
                Ok(ProcessStatus::Exited {
                    code: status.code(),
                })
            }
        }
    }
}

pub fn controlled_environment(path: Option<&OsStr>) -> BTreeMap<OsString, OsString> {
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
        "SYSTEMROOT",
        "WINDIR",
    ];
    let mut environment: BTreeMap<OsString, OsString> = ALLOWED
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect();
    if let Some(path) = path {
        environment.insert(OsString::from("PATH"), path.to_owned());
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
            Ok(0) => return,
            Err(error) => {
                output
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .error = Some(error.to_string());
                return;
            }
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
    error: Option<String>,
}
impl BoundedOutput {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
            error: None,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        if self.limit == 0 {
            return;
        }
        if bytes.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(&bytes[bytes.len() - self.limit..]);
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.limit);
        self.bytes.drain(..overflow);
        self.bytes.extend(bytes);
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> Result<()> {
    let pid = i32::try_from(pid).map_err(|_| DapError::Io("process id exceeds i32".to_owned()))?;
    // SAFETY: kill is called with a valid negative process-group id and signal.
    if unsafe { libc::kill(-pid, signal) } == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error.into())
        }
    }
}
#[cfg(unix)]
fn terminate_process_group(pid: u32) -> Result<()> {
    signal_group(pid, libc::SIGTERM)
}
#[cfg(unix)]
fn kill_process_group(pid: u32) -> Result<()> {
    signal_group(pid, libc::SIGKILL)
}
#[cfg(unix)]
fn force_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: best-effort Drop cleanup of the owned process group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn kill_process_group(_pid: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn force_process_group(_pid: u32) {}
