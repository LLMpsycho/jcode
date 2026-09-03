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
}
