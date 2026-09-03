//! Dependency-light Language Server Protocol foundations.

mod client;
mod config;
mod diagnostics;
mod discovery;
mod document_sync;
mod error;
mod framing;
mod protocol;
pub mod testing;
mod transport;
mod workspace;

pub use client::LspClient;
pub use config::config_digest;
pub use diagnostics::{DiagnosticSnapshot, DiagnosticsCache, diagnostic_delta};
pub use discovery::discover_executable;
pub use document_sync::{DocumentState, DocumentSync};
pub use error::{LspError, Result};
pub use framing::{
    DEFAULT_MAX_HEADER_BYTES, DEFAULT_MAX_PAYLOAD_BYTES, FrameDecoder, encode_frame,
};
pub use jcode_lsp_types::*;
pub use protocol::{IncomingMessage, decode_message, encode_message};
pub use transport::{LspProcess, ProcessStatus};
pub use workspace::{LspServicePool, LspWorkspace, LspWorkspaceKey};
