//! Dependency-light Language Server Protocol foundations.

mod error;
mod framing;
mod protocol;

pub use error::{LspError, Result};
pub use framing::{
    DEFAULT_MAX_HEADER_BYTES, DEFAULT_MAX_PAYLOAD_BYTES, FrameDecoder, encode_frame,
};
pub use jcode_lsp_types::*;
pub use protocol::{IncomingMessage, decode_message, encode_message};
