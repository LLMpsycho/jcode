//! Dependency-light Debug Adapter Protocol infrastructure.

mod client;
mod error;
mod framing;
mod process;
mod protocol;
pub mod testing;

pub use client::{
    DapClient, EVENT_CHANNEL_CAPACITY, MAX_RETAINED_EVENT_BYTES, MAX_RETAINED_EVENT_SIZE,
};
pub use error::{DapError, Result};
pub use framing::{
    DEFAULT_MAX_HEADER_BYTES, DEFAULT_MAX_PAYLOAD_BYTES, FrameDecoder, encode_frame,
};
pub use jcode_dap_types::*;
pub use process::{AdapterCommand, AdapterProcess, ProcessStatus, controlled_environment};
pub use protocol::{Message, decode_message, encode_message};
