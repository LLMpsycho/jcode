use thiserror::Error;

pub type Result<T> = std::result::Result<T, LspError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LspError {
    #[error("LSP header exceeds the configured {limit}-byte limit")]
    HeaderTooLarge { limit: usize },
    #[error("LSP header is not valid ASCII")]
    InvalidHeaderEncoding,
    #[error("LSP frame is missing Content-Length")]
    MissingContentLength,
    #[error("LSP frame contains duplicate Content-Length headers")]
    DuplicateContentLength,
    #[error("invalid LSP Content-Length value: {value}")]
    InvalidContentLength { value: String },
    #[error("LSP payload length {observed} exceeds the configured {limit}-byte limit")]
    PayloadTooLarge { observed: usize, limit: usize },
    #[error("malformed LSP JSON payload: {0}")]
    InvalidJson(String),
    #[error("invalid JSON-RPC message: {0}")]
    InvalidMessage(String),
    #[error("LSP transport I/O failed: {0}")]
    Io(String),
    #[error("LSP request {method} timed out")]
    RequestTimeout { method: String },
    #[error("LSP transport closed")]
    TransportClosed,
    #[error("LSP server returned error {code}: {message}")]
    Response { code: i64, message: String },
    #[error("LSP executable `{command}` was not found")]
    ExecutableNotFound { command: String },
    #[error("LSP executable path is not an executable file: {path}")]
    NotExecutable { path: String },
    #[error("invalid LSP configuration: {0}")]
    InvalidConfig(String),
    #[error("LSP child process did not expose its {stream} pipe")]
    MissingProcessPipe { stream: &'static str },
    #[error("workspace path cannot be represented as a file URI: {path}")]
    InvalidWorkspaceUri { path: String },
}

impl From<std::io::Error> for LspError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
