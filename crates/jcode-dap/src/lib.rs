//! Dependency-light Debug Adapter Protocol infrastructure.
//!
//! Low-level client and process primitives are intentionally not public:
//! ```compile_fail
//! use jcode_dap::{AdapterCommand, AdapterProcess, DapClient};
//! let client = DapClient::start(tokio::io::duplex(64).0);
//! client.request("attach", Some(serde_json::json!({"pid": 1234})), std::time::Duration::from_secs(1));
//! AdapterProcess::spawn(&AdapterCommand::new("/usr/bin/lldb-dap", "/"));
//! ```

mod breakpoints;
mod client;
mod control;
mod error;
mod framing;
mod inspection;
mod launch;
mod manager;
mod process;
mod protocol;
mod session;
#[cfg(test)]
mod testing;

pub use breakpoints::*;
pub(crate) use client::DapClient;
#[cfg(test)]
pub(crate) use client::{EVENT_CHANNEL_CAPACITY, MAX_RETAINED_EVENT_SIZE};
pub use control::*;
pub use error::{DapError, DebugStartOperation, DebugStartupPhase, Result};
pub use framing::{
    DEFAULT_MAX_HEADER_BYTES, DEFAULT_MAX_PAYLOAD_BYTES, FrameDecoder, encode_frame,
};
pub use inspection::*;
pub use jcode_dap_types::{
    Capabilities, DapAdapterConfig, DapAdapterKind, DapConfig, Event, EventType,
    InitializeRequestArguments, MAX_OPAQUE_HANDLES_PER_OWNER, MAX_OUTPUT_BYTES,
    MIN_OPAQUE_HANDLES_PER_OWNER, MIN_OUTPUT_BYTES, Request, RequestType, Response, ResponseType,
    RunInTerminalKind, RunInTerminalRequestArguments,
};
pub use launch::{
    DebugAdapterConfig, DebugAdapterKind, DebugLaunchRequest, DebugOwnedAttachRequest,
    DebugSessionStart,
};
pub use manager::DebugSessionManager;
pub(crate) use process::{AdapterCommand, AdapterProcess, ProcessStatus};
pub use protocol::{Message, decode_message, encode_message};
pub use session::{
    DebugCleanupFailure, DebugCleanupReport, DebugOutputCategory, DebugOutputCursor,
    DebugOutputPage, DebugOutputRecord, DebugOutputStatus, DebugSessionEnd, DebugSessionEndReason,
    DebugSessionId, DebugSessionManagerConfig, DebugSessionSnapshot, DebugSessionState,
    DebugSessionStateKind, DebugWorkspaceKey, OwnerCleanupCause, StoppedState,
};
