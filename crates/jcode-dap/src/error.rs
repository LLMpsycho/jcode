use std::path::PathBuf;

use thiserror::Error;

use crate::{
    DebugBreakpointId, DebugExecutionRevision, DebugSessionId, DebugSessionStateKind, DebugThreadId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugStartOperation {
    Launch,
    OwnedAttach,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugStartupPhase {
    Initialize,
    StartRequest,
    AwaitInitialized,
    ConfigurationDone,
}

pub type Result<T> = std::result::Result<T, DapError>;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DapError {
    #[error("DAP header exceeds the configured {limit}-byte limit")]
    HeaderTooLarge { limit: usize },
    #[error("DAP header is not valid ASCII")]
    InvalidHeaderEncoding,
    #[error("DAP frame is missing Content-Length")]
    MissingContentLength,
    #[error("DAP frame contains duplicate Content-Length headers")]
    DuplicateContentLength,
    #[error("invalid DAP Content-Length value: {value}")]
    InvalidContentLength { value: String },
    #[error("DAP payload length {observed} exceeds the configured {limit}-byte limit")]
    PayloadTooLarge { observed: usize, limit: usize },
    #[error("malformed DAP JSON payload: {0}")]
    InvalidJson(String),
    #[error("invalid DAP message: {0}")]
    InvalidMessage(String),
    #[error("DAP transport I/O failed: {0}")]
    Io(String),
    #[error("DAP request {command} timed out")]
    RequestTimeout { command: String },
    #[error("DAP request timeout exceeds the supported instant range")]
    InvalidRequestTimeout,
    #[error("DAP transport closed")]
    TransportClosed,
    #[error("DAP adapter rejected {command}: {message}")]
    Response { command: String, message: String },
    #[error("DAP child process did not expose its {stream} pipe")]
    MissingProcessPipe { stream: &'static str },
    #[error("invalid DAP session manager configuration: {message}")]
    InvalidManagerConfiguration { message: String },
    #[error("invalid debug workspace {path:?}: {message}")]
    InvalidWorkspace { path: PathBuf, message: String },
    #[error("DAP session capacity {limit} has been reached")]
    SessionCapacityExceeded { limit: usize },
    #[error("DAP session identifier space is exhausted")]
    SessionIdExhausted,
    #[error("owner session {owner_session_id} already has active debug session {session_id}")]
    OwnerAlreadyHasActiveSession {
        owner_session_id: String,
        session_id: DebugSessionId,
    },
    #[error("debug session {session_id} was not found")]
    SessionNotFound { session_id: DebugSessionId },
    #[error("access to debug session {session_id} is denied")]
    SessionAccessDenied { session_id: DebugSessionId },
    #[error("cannot {operation} debug session {session_id} while it is {state:?}")]
    InvalidSessionTransition {
        session_id: DebugSessionId,
        state: DebugSessionStateKind,
        operation: &'static str,
    },
    #[error("failed to clean debug session {session_id}: {message}")]
    SessionCleanupFailed {
        session_id: DebugSessionId,
        message: String,
    },
    #[error("invalid debug adapter configuration: {message}")]
    InvalidAdapterConfiguration { message: String },
    #[error("debug adapter executable {path:?} is unavailable: {message}")]
    AdapterUnavailable { path: PathBuf, message: String },
    #[error("invalid debug program path {path:?}: {message}")]
    InvalidDebugProgram { path: PathBuf, message: String },
    #[error("debug path {path:?} is outside workspace {workspace:?}")]
    DebugPathOutsideWorkspace { path: PathBuf, workspace: PathBuf },
    #[error("debug working directory {path:?} is invalid: {message}")]
    InvalidDebugWorkingDirectory { path: PathBuf, message: String },
    #[error("{operation:?} cannot provide owned process containment on {platform}")]
    ProcessContainmentUnavailable {
        operation: DebugStartOperation,
        platform: &'static str,
    },
    #[error("debug session startup timed out during {phase:?}")]
    DebugStartupTimeout { phase: DebugStartupPhase },
    #[error("invalid DAP initialize response: {message}")]
    InvalidInitializeResponse { message: String },
    #[error("owned debug target exited before attach with code {exit_code:?}")]
    DebugTargetExitedBeforeAttach { exit_code: Option<i32> },
    #[error("debug session {session_id} ended during startup: {message}")]
    SessionEndedDuringStartup {
        session_id: DebugSessionId,
        message: String,
    },
    #[error("debug startup failed: {message}; adapter stderr: {adapter_stderr}")]
    DebugStartupFailed {
        message: String,
        adapter_stderr: String,
    },
    #[error("invalid debug source {path:?}: {message}")]
    InvalidDebugSource { path: PathBuf, message: String },
    #[error("debug source {path:?} is outside workspace {workspace:?}")]
    DebugSourceOutsideWorkspace { path: PathBuf, workspace: PathBuf },
    #[error("debug source {path:?} has {observed} bytes, exceeding limit {limit}")]
    DebugSourceTooLarge {
        path: PathBuf,
        observed: u64,
        limit: u64,
    },
    #[error("debug source revision mismatch for {path:?}")]
    DebugSourceRevisionMismatch { path: PathBuf },
    #[error("debug source changed during operation: {path:?}")]
    DebugSourceChangedDuringOperation { path: PathBuf },
    #[error("invalid breakpoint: {message}")]
    InvalidBreakpoint { message: String },
    #[error("breakpoint {breakpoint_id} was not found in debug session {session_id}")]
    BreakpointNotFound {
        session_id: DebugSessionId,
        breakpoint_id: DebugBreakpointId,
    },
    #[error("breakpoint {scope} limit {limit} exceeded")]
    BreakpointLimitExceeded { scope: &'static str, limit: usize },
    #[error("DAP operation {operation} requires capability {capability}")]
    UnsupportedDapCapability {
        operation: &'static str,
        capability: &'static str,
    },
    #[error("invalid setBreakpoints response: {message}")]
    InvalidSetBreakpointsResponse { message: String },
    #[error("breakpoint reconciliation for {path:?} is indeterminate: {message}")]
    BreakpointReconciliationIndeterminate { path: PathBuf, message: String },
    #[error("invalid threads response: {message}")]
    InvalidThreadsResponse { message: String },
    #[error("thread response contains {observed} threads, exceeding limit {limit}")]
    ThreadLimitExceeded { observed: usize, limit: usize },
    #[error("thread {thread_id} was not found in debug session {session_id}")]
    ThreadNotFound {
        session_id: DebugSessionId,
        thread_id: DebugThreadId,
    },
    #[error(
        "debug session {session_id} has no unambiguous stopped thread ({observed_threads} observed)"
    )]
    AmbiguousStoppedThread {
        session_id: DebugSessionId,
        observed_threads: usize,
    },
    #[error("debug session {session_id} has no available stopped thread")]
    StoppedThreadUnavailable { session_id: DebugSessionId },
    #[error(
        "debug session {session_id} has ambiguous thread selection ({observed_threads} observed)"
    )]
    AmbiguousThreadSelection {
        session_id: DebugSessionId,
        observed_threads: usize,
    },
    #[error(
        "stale execution revision for session {session_id}: expected {expected}, actual {actual}"
    )]
    StaleExecutionRevision {
        session_id: DebugSessionId,
        expected: DebugExecutionRevision,
        actual: DebugExecutionRevision,
    },
    #[error("debug operation task {operation} failed: {message}")]
    DebugOperationTaskFailed {
        operation: &'static str,
        message: String,
    },
}

impl From<std::io::Error> for DapError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
