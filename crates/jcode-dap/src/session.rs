use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Deserialize;

use crate::{Capabilities, DapError, Event, Result};

static NEXT_MANAGER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DebugSessionId {
    manager: u64,
    sequence: u64,
}

impl DebugSessionId {
    #[allow(dead_code)]
    pub(crate) fn new(manager: u64, sequence: u64) -> Self {
        Self { manager, sequence }
    }
}

impl fmt::Display for DebugSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "dap-{:x}-{:x}", self.manager, self.sequence)
    }
}

pub(crate) fn next_manager_id() -> Result<u64> {
    NEXT_MANAGER_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map_err(|_| DapError::SessionIdExhausted)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DebugWorkspaceKey {
    canonical_root: PathBuf,
    worktree_identity: String,
}

impl DebugWorkspaceKey {
    pub fn new(root: &Path, worktree_identity: impl Into<String>) -> Result<Self> {
        let canonical_root = root
            .canonicalize()
            .map_err(|error| DapError::InvalidWorkspace {
                path: root.to_path_buf(),
                message: error.to_string(),
            })?;
        if !canonical_root.is_dir() {
            return Err(DapError::InvalidWorkspace {
                path: root.to_path_buf(),
                message: "workspace root is not a directory".to_owned(),
            });
        }
        let worktree_identity = worktree_identity.into();
        if worktree_identity.trim().is_empty() {
            return Err(DapError::InvalidWorkspace {
                path: root.to_path_buf(),
                message: "worktree identity must not be empty".to_owned(),
            });
        }
        Ok(Self {
            canonical_root,
            worktree_identity,
        })
    }

    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub fn worktree_identity(&self) -> &str {
        &self.worktree_identity
    }
}

#[derive(Clone, Debug)]
pub struct DebugSessionManagerConfig {
    pub max_active_sessions: usize,
    pub max_retained_ended_sessions: usize,
    pub output_max_events: usize,
    pub output_max_bytes: usize,
    pub output_page_limit: usize,
    pub termination_grace: Duration,
    pub process_poll_interval: Duration,
    pub startup_timeout: Duration,
    pub disconnect_timeout: Duration,
}

impl Default for DebugSessionManagerConfig {
    fn default() -> Self {
        Self {
            max_active_sessions: 64,
            max_retained_ended_sessions: 64,
            output_max_events: 1024,
            output_max_bytes: 1024 * 1024,
            output_page_limit: 256,
            termination_grace: Duration::from_secs(2),
            process_poll_interval: Duration::from_millis(250),
            startup_timeout: Duration::from_secs(30),
            disconnect_timeout: Duration::from_secs(2),
        }
    }
}

impl DebugSessionManagerConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_active_sessions == 0
            || self.output_max_events == 0
            || self.output_max_bytes == 0
            || self.output_page_limit == 0
            || self.process_poll_interval.is_zero()
        {
            return Err(DapError::InvalidManagerConfiguration {
                message: "active, output, page, and polling limits must be non-zero".to_owned(),
            });
        }
        let now = std::time::Instant::now();
        if now.checked_add(self.termination_grace).is_none()
            || now.checked_add(self.process_poll_interval).is_none()
            || now.checked_add(self.startup_timeout).is_none()
            || now.checked_add(self.disconnect_timeout).is_none()
        {
            return Err(DapError::InvalidManagerConfiguration {
                message: "termination and polling durations must fit the platform instant range"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugSessionStateKind {
    Reserved,
    Initializing,
    Configuring,
    Running,
    Stopped,
    Terminating,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugSessionState {
    Reserved,
    Initializing,
    Configuring,
    Running,
    Stopped(StoppedState),
    Terminating,
    Ended(DebugSessionEnd),
}

impl DebugSessionState {
    pub fn kind(&self) -> DebugSessionStateKind {
        match self {
            Self::Reserved => DebugSessionStateKind::Reserved,
            Self::Initializing => DebugSessionStateKind::Initializing,
            Self::Configuring => DebugSessionStateKind::Configuring,
            Self::Running => DebugSessionStateKind::Running,
            Self::Stopped(_) => DebugSessionStateKind::Stopped,
            Self::Terminating => DebugSessionStateKind::Terminating,
            Self::Ended(_) => DebugSessionStateKind::Ended,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Ended(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoppedState {
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<i64>,
    pub all_threads_stopped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSessionEnd {
    pub reason: DebugSessionEndReason,
    pub cleanup_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugSessionEndReason {
    Requested,
    LaunchCancelled,
    OwnerDisconnected,
    OwnerExpired,
    ServerShutdown,
    DebuggeeExited { exit_code: Option<i64> },
    AdapterExited { exit_code: Option<i32> },
    TransportClosed,
    ProtocolError { message: String },
    EventStreamLagged { skipped: u64 },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DebugOutputCursor(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugOutputCategory {
    Console,
    Important,
    Stdout,
    Stderr,
    Telemetry,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugOutputRecord {
    pub cursor: DebugOutputCursor,
    pub category: DebugOutputCategory,
    pub output: String,
    pub truncated_prefix_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugOutputStatus {
    pub first_retained_cursor: Option<DebugOutputCursor>,
    pub next_cursor: DebugOutputCursor,
    pub retained_events: usize,
    pub retained_bytes: usize,
    pub evicted_events: u64,
    pub source_dropped_events: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugOutputPage {
    pub records: Vec<DebugOutputRecord>,
    pub status: DebugOutputStatus,
    pub requested_history_was_evicted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugSessionSnapshot {
    pub id: DebugSessionId,
    pub workspace: DebugWorkspaceKey,
    pub adapter_id: String,
    pub start: crate::DebugSessionStart,
    pub state: DebugSessionState,
    pub capabilities: Capabilities,
    pub output: DebugOutputStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerCleanupCause {
    Disconnected,
    Expired,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugCleanupReport {
    pub cleaned: usize,
    pub already_ended: usize,
    pub failures: Vec<DebugCleanupFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugCleanupFailure {
    pub session_id: DebugSessionId,
    pub message: String,
}

pub(crate) struct OutputRing {
    records: VecDeque<DebugOutputRecord>,
    retained_bytes: usize,
    max_events: usize,
    max_bytes: usize,
    next_cursor: u64,
    evicted_events: u64,
    source_dropped_events: u64,
}

impl OutputRing {
    #[allow(dead_code)]
    pub(crate) fn new(max_events: usize, max_bytes: usize) -> Self {
        Self {
            records: VecDeque::new(),
            retained_bytes: 0,
            max_events,
            max_bytes,
            next_cursor: 1,
            evicted_events: 0,
            source_dropped_events: 0,
        }
    }

    pub(crate) fn push(&mut self, category: DebugOutputCategory, output: String) {
        let cursor = DebugOutputCursor(self.next_cursor);
        self.next_cursor = self.next_cursor.saturating_add(1);
        let (output, truncated_prefix_bytes) = utf8_tail(output, self.max_bytes);
        self.retained_bytes += output.len();
        self.records.push_back(DebugOutputRecord {
            cursor,
            category,
            output,
            truncated_prefix_bytes,
        });
        while self.records.len() > self.max_events || self.retained_bytes > self.max_bytes {
            if let Some(record) = self.records.pop_front() {
                self.retained_bytes -= record.output.len();
                self.evicted_events = self.evicted_events.saturating_add(1);
            }
        }
    }

    pub(crate) fn add_source_loss(&mut self, count: u64) {
        self.source_dropped_events = self.source_dropped_events.saturating_add(count);
    }

    pub(crate) fn status(&self) -> DebugOutputStatus {
        DebugOutputStatus {
            first_retained_cursor: self.records.front().map(|record| record.cursor),
            next_cursor: DebugOutputCursor(self.next_cursor),
            retained_events: self.records.len(),
            retained_bytes: self.retained_bytes,
            evicted_events: self.evicted_events,
            source_dropped_events: self.source_dropped_events,
        }
    }

    pub(crate) fn page(&self, after: Option<DebugOutputCursor>, limit: usize) -> DebugOutputPage {
        let first = self.records.front().map(|record| record.cursor);
        let requested_history_was_evicted = matches!((after, first), (Some(after), Some(first)) if after.0.saturating_add(1) < first.0);
        let records = self
            .records
            .iter()
            .filter(|record| after.is_none_or(|cursor| record.cursor > cursor))
            .take(limit)
            .cloned()
            .collect();
        DebugOutputPage {
            records,
            status: self.status(),
            requested_history_was_evicted,
        }
    }
}

fn utf8_tail(mut output: String, max_bytes: usize) -> (String, usize) {
    if output.len() <= max_bytes {
        return (output, 0);
    }
    let mut start = output.len() - max_bytes;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let truncated = start;
    output.drain(..start);
    (output, truncated)
}

#[derive(Deserialize)]
struct OutputBody {
    output: String,
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoppedBody {
    reason: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    thread_id: Option<i64>,
    #[serde(default)]
    all_threads_stopped: bool,
    #[serde(default)]
    hit_breakpoint_ids: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinuedBody {
    #[serde(default)]
    thread_id: Option<i64>,
    #[serde(default = "default_true")]
    all_threads_continued: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct ExitedBody {
    #[serde(rename = "exitCode")]
    exit_code: i64,
}

pub(crate) enum SessionEvent {
    Output(DebugOutputCategory, String),
    Initialized,
    Stopped(StoppedState),
    Continued,
    Breakpoint { seq: i64, body: serde_json::Value },
    Terminated,
    Exited(Option<i64>),
    Ignore,
}

pub(crate) fn parse_event(event: Event) -> Result<SessionEvent> {
    let body = event.body.unwrap_or(serde_json::Value::Null);
    match event.event.as_str() {
        "output" => {
            let body: OutputBody = serde_json::from_value(body)
                .map_err(|error| protocol_event_error("output", error))?;
            Ok(SessionEvent::Output(
                category(body.category.as_deref()),
                body.output,
            ))
        }
        "initialized" => Ok(SessionEvent::Initialized),
        "stopped" => {
            let body: StoppedBody = serde_json::from_value(body)
                .map_err(|error| protocol_event_error("stopped", error))?;
            if body.reason.is_empty() {
                return Err(DapError::InvalidMessage(
                    "stopped event reason is empty".to_owned(),
                ));
            }
            for id in &body.hit_breakpoint_ids {
                i32::try_from(*id).map_err(|_| {
                    DapError::InvalidMessage(
                        "stopped hitBreakpointIds contains an out-of-range id".to_owned(),
                    )
                })?;
            }
            if let Some(id) = body.thread_id {
                i32::try_from(id).map_err(|_| {
                    DapError::InvalidMessage(
                        "stopped threadId is outside signed 32-bit range".to_owned(),
                    )
                })?;
            }
            Ok(SessionEvent::Stopped(StoppedState {
                reason: body.reason,
                description: body.description,
                thread_id: body.thread_id,
                all_threads_stopped: body.all_threads_stopped,
            }))
        }
        "continued" => {
            if !body.is_null() && !body.is_object() {
                return Err(DapError::InvalidMessage(
                    "malformed continued event body".to_owned(),
                ));
            }
            if !body.is_null() {
                let parsed: ContinuedBody = serde_json::from_value(body)
                    .map_err(|error| protocol_event_error("continued", error))?;
                if let Some(id) = parsed.thread_id {
                    i32::try_from(id).map_err(|_| {
                        DapError::InvalidMessage(
                            "continued threadId is outside signed 32-bit range".to_owned(),
                        )
                    })?;
                }
                let _ = parsed.all_threads_continued;
            }
            Ok(SessionEvent::Continued)
        }
        "breakpoint" => Ok(SessionEvent::Breakpoint {
            seq: event.seq,
            body,
        }),
        "terminated" => Ok(SessionEvent::Terminated),
        "exited" => {
            let body: ExitedBody = serde_json::from_value(body)
                .map_err(|error| protocol_event_error("exited", error))?;
            Ok(SessionEvent::Exited(Some(body.exit_code)))
        }
        _ => Ok(SessionEvent::Ignore),
    }
}

fn protocol_event_error(event: &str, error: serde_json::Error) -> DapError {
    DapError::InvalidMessage(format!("malformed {event} event body: {error}"))
}

fn category(value: Option<&str>) -> DebugOutputCategory {
    match value {
        None | Some("console") => DebugOutputCategory::Console,
        Some("important") => DebugOutputCategory::Important,
        Some("stdout") => DebugOutputCategory::Stdout,
        Some("stderr") => DebugOutputCategory::Stderr,
        Some("telemetry") => DebugOutputCategory::Telemetry,
        Some(_) => DebugOutputCategory::Other,
    }
}

#[cfg(test)]
mod tests;
