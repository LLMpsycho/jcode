use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::launch::ResolvedProgram;
use crate::{DapClient, DapError, Result};

const DEFAULT_STDERR_LIMIT: usize = 64 * 1024;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct AdapterCommand {
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
    pub(crate) fn with_arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }
    #[cfg(test)]
    pub(crate) fn with_stderr_limit(mut self, limit: usize) -> Self {
        self.stderr_limit = limit;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProcessStatus {
    Running,
    Exited { code: Option<i32> },
}

pub(crate) struct ChildStdio {
    pub stdin: tokio::process::ChildStdin,
    pub stdout: tokio::process::ChildStdout,
    pub stderr: tokio::process::ChildStderr,
}

#[derive(Clone)]
pub(crate) struct OwnedChildProcess {
    state: Arc<OwnedChildState>,
}
#[derive(Clone)]
pub(crate) struct OwnedChildObserver {
    state: Weak<OwnedChildState>,
}
struct OwnedChildState {
    child: tokio::sync::Mutex<Child>,
    pid: AtomicU32,
}

impl Drop for OwnedChildState {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            force_process_group(pid);
        }
    }
}
impl OwnedChildState {
    fn pid(&self) -> Option<u32> {
        match self.pid.load(Ordering::Acquire) {
            0 => None,
            p => Some(p),
        }
    }
    fn mark_reaped(&self) {
        self.pid.store(0, Ordering::Release)
    }
    fn cleanup_reaped_group(&self) -> Result<()> {
        let result = self.pid().map(kill_reaped_process_group).transpose();
        self.mark_reaped();
        result.map(|_| ())
    }
}

impl OwnedChildProcess {
    pub(crate) fn observer(&self) -> OwnedChildObserver {
        OwnedChildObserver {
            state: Arc::downgrade(&self.state),
        }
    }
    pub(crate) async fn spawn_adapter(config: &AdapterCommand) -> Result<(Self, ChildStdio)> {
        if !config.command.is_absolute() {
            return Err(DapError::InvalidMessage(
                "adapter command must be an absolute path".into(),
            ));
        }
        if !config.cwd.is_absolute() {
            return Err(DapError::InvalidMessage(
                "adapter cwd must be an absolute path".into(),
            ));
        }
        let command_identity = config.command.canonicalize()?;
        if command_identity != config.command || !command_identity.is_file() {
            return Err(DapError::InvalidMessage(
                "adapter executable identity changed before spawn".into(),
            ));
        }
        validate_spawn_executable(&command_identity)?;
        let cwd_identity = config.cwd.canonicalize()?;
        if cwd_identity != config.cwd || !cwd_identity.is_dir() {
            return Err(DapError::InvalidMessage(
                "adapter working directory identity changed before spawn".into(),
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
        let pid = child.id().unwrap_or(0);
        let stdio = ChildStdio {
            stdin: child
                .stdin
                .take()
                .ok_or(DapError::MissingProcessPipe { stream: "stdin" })?,
            stdout: child
                .stdout
                .take()
                .ok_or(DapError::MissingProcessPipe { stream: "stdout" })?,
            stderr: child
                .stderr
                .take()
                .ok_or(DapError::MissingProcessPipe { stream: "stderr" })?,
        };
        Ok((
            Self {
                state: Arc::new(OwnedChildState {
                    child: tokio::sync::Mutex::new(child),
                    pid: AtomicU32::new(pid),
                }),
            },
            stdio,
        ))
    }

    pub(crate) async fn spawn_debug_target(
        target: &ResolvedProgram,
        _allowed_tracer_pid: Option<u32>,
    ) -> Result<Self> {
        let program_identity = target.program.canonicalize()?;
        if program_identity != target.program || !program_identity.is_file() {
            return Err(DapError::InvalidDebugProgram {
                path: target.program.clone(),
                message: "program identity changed before spawn".into(),
            });
        }
        validate_spawn_executable(&program_identity)?;
        let cwd_identity = target.cwd.canonicalize()?;
        if cwd_identity != target.cwd || !cwd_identity.is_dir() {
            return Err(DapError::InvalidDebugWorkingDirectory {
                path: target.cwd.clone(),
                message: "working directory identity changed before spawn".into(),
            });
        }
        let mut command = Command::new(&program_identity);
        command
            .args(&target.args)
            .current_dir(&target.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .env_clear();
        command.envs(controlled_environment(std::env::var_os("PATH").as_deref()));
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(target_os = "linux")]
        if let Some(adapter_pid) = _allowed_tracer_pid {
            // SAFETY: pre_exec invokes only the async-signal-safe prctl syscall and creates no allocations.
            unsafe {
                command.pre_exec(move || set_ptracer(adapter_pid));
            }
        }
        let child = command.spawn()?;
        let pid = child.id().unwrap_or(0);
        Ok(Self {
            state: Arc::new(OwnedChildState {
                child: tokio::sync::Mutex::new(child),
                pid: AtomicU32::new(pid),
            }),
        })
    }
    pub(crate) fn pid(&self) -> Option<u32> {
        self.state.pid()
    }
    pub(crate) async fn status(&self) -> Result<ProcessStatus> {
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
    pub(crate) async fn terminate(&self, grace: Duration) -> Result<ProcessStatus> {
        let mut child = self.state.child.lock().await;
        if let Some(status) = child.try_wait()? {
            self.state.cleanup_reaped_group()?;
            return Ok(ProcessStatus::Exited {
                code: status.code(),
            });
        }
        if let Some(pid) = self.state.pid()
            && let Err(signal_error) = terminate_process_group(pid)
        {
            return match tokio::time::timeout(grace, child.wait()).await {
                Ok(status) => {
                    let status = status?;
                    self.state.cleanup_reaped_group()?;
                    Ok(ProcessStatus::Exited {
                        code: status.code(),
                    })
                }
                Err(_) => Err(signal_error),
            };
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

fn validate_spawn_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = path.metadata()?.permissions().mode();
        if mode & 0o111 == 0 || mode & 0o6000 != 0 {
            return Err(DapError::InvalidMessage(
                "executable permissions changed before spawn".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn set_ptracer(adapter_pid: u32) -> std::io::Result<()> {
    set_ptracer_with(adapter_pid, |option, value| unsafe {
        libc::prctl(option, value, 0, 0, 0)
    })
}

#[cfg(target_os = "linux")]
fn set_ptracer_with(
    adapter_pid: u32,
    call: impl FnOnce(libc::c_int, libc::c_ulong) -> libc::c_int,
) -> std::io::Result<()> {
    if adapter_pid == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "tracer PID must be nonzero",
        ));
    }
    if call(libc::PR_SET_PTRACER, adapter_pid as libc::c_ulong) == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn test_set_ptracer_with(
    adapter_pid: u32,
    call: impl FnOnce(libc::c_int, libc::c_ulong) -> libc::c_int,
) -> std::io::Result<()> {
    set_ptracer_with(adapter_pid, call)
}

impl OwnedChildObserver {
    pub(crate) async fn status(&self) -> Result<Option<ProcessStatus>> {
        let Some(state) = self.state.upgrade() else {
            return Ok(None);
        };
        match state.child.lock().await.try_wait()? {
            Some(status) => {
                state.cleanup_reaped_group()?;
                Ok(Some(ProcessStatus::Exited {
                    code: status.code(),
                }))
            }
            None => Ok(Some(ProcessStatus::Running)),
        }
    }
}

pub(crate) struct AdapterProcess {
    client: DapClient,
    process: OwnedChildProcess,
    stderr: Arc<Mutex<BoundedOutput>>,
}
impl AdapterProcess {
    pub async fn spawn(config: &AdapterCommand) -> Result<Self> {
        let (process, stdio) = OwnedChildProcess::spawn_adapter(config).await?;
        let captured = Arc::new(Mutex::new(BoundedOutput::new(config.stderr_limit)));
        tokio::spawn(capture_stderr(stdio.stderr, Arc::clone(&captured)));
        Ok(Self {
            client: DapClient::start_split(stdio.stdout, stdio.stdin),
            process,
            stderr: captured,
        })
    }
    pub fn client(&self) -> &DapClient {
        &self.client
    }
    pub(crate) fn pid(&self) -> Option<u32> {
        self.process.pid()
    }
    pub(crate) fn observer(&self) -> OwnedChildObserver {
        self.process.observer()
    }
    #[cfg(test)]
    pub(crate) async fn status(&self) -> Result<ProcessStatus> {
        self.process.status().await
    }
    #[cfg(test)]
    pub(crate) fn stderr_capture_error(&self) -> Option<String> {
        self.stderr
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .error
            .clone()
    }
    pub fn recent_stderr(&self) -> Vec<u8> {
        self.stderr
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .bytes
            .iter()
            .copied()
            .collect()
    }
    pub async fn terminate(&self, grace: Duration) -> Result<ProcessStatus> {
        self.process.terminate(grace).await
    }
}

#[derive(Clone)]
pub(crate) struct OwnedTargetProcess {
    process: OwnedChildProcess,
}
impl OwnedTargetProcess {
    pub(crate) async fn spawn(
        target: &ResolvedProgram,
        allowed_tracer_pid: Option<u32>,
    ) -> Result<Self> {
        Ok(Self {
            process: OwnedChildProcess::spawn_debug_target(target, allowed_tracer_pid).await?,
        })
    }
    pub(crate) fn pid(&self) -> Option<u32> {
        self.process.pid()
    }
    pub(crate) fn observer(&self) -> OwnedChildObserver {
        self.process.observer()
    }
    pub(crate) async fn status(&self) -> Result<ProcessStatus> {
        self.process.status().await
    }
    pub(crate) async fn terminate(&self, grace: Duration) -> Result<ProcessStatus> {
        self.process.terminate(grace).await
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
    let mut env: BTreeMap<OsString, OsString> = ALLOWED
        .iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (OsString::from(k), v)))
        .collect();
    if let Some(path) = path {
        env.insert(OsString::from("PATH"), path.to_owned());
    }
    env
}
async fn capture_stderr<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    output: Arc<Mutex<BoundedOutput>>,
) {
    let mut buffer = [0; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return,
            Err(e) => {
                output.lock().unwrap_or_else(|p| p.into_inner()).error = Some(e.to_string());
                return;
            }
            Ok(n) => output
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(&buffer[..n]),
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
        self.bytes.extend(bytes)
    }
}
#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> Result<()> {
    let pid = i32::try_from(pid).map_err(|_| DapError::Io("process id exceeds i32".into()))?;
    if unsafe { libc::kill(-pid, signal) } == 0 {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(e.into())
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
fn kill_reaped_process_group(pid: u32) -> Result<()> {
    match signal_group(pid, libc::SIGKILL) {
        Err(DapError::Io(message))
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
                || message.contains("Operation not permitted") =>
        {
            Ok(())
        }
        result => result,
    }
}
#[cfg(unix)]
fn force_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}
#[cfg(not(unix))]
fn terminate_process_group(_: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn kill_process_group(_: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn kill_reaped_process_group(_: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn force_process_group(_: u32) {}
