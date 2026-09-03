use std::path::PathBuf;

use thiserror::Error;

use crate::{DebugSessionId, DebugSessionStateKind};

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
}

impl From<std::io::Error> for DapError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
