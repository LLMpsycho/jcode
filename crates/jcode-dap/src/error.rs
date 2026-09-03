use thiserror::Error;

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
    #[error("DAP transport closed")]
    TransportClosed,
    #[error("DAP adapter rejected {command}: {message}")]
    Response { command: String, message: String },
    #[error("DAP child process did not expose its {stream} pipe")]
    MissingProcessPipe { stream: &'static str },
}

impl From<std::io::Error> for DapError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
