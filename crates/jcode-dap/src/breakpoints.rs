use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct DebugOperationConfig {
    pub operation_timeout: Duration,
    pub max_breakpoint_sources: usize,
    pub max_breakpoints_per_source: usize,
    pub max_total_breakpoints: usize,
    pub max_breakpoint_expression_bytes: usize,
    pub max_breakpoint_message_bytes: usize,
    pub max_source_path_bytes: usize,
    pub max_source_revision_bytes: u64,
    pub max_threads: usize,
    pub max_thread_name_bytes: usize,
    pub max_queued_breakpoint_events: usize,
}

impl Default for DebugOperationConfig {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(10),
            max_breakpoint_sources: 256,
            max_breakpoints_per_source: 256,
            max_total_breakpoints: 1024,
            max_breakpoint_expression_bytes: 4096,
            max_breakpoint_message_bytes: 4096,
            max_source_path_bytes: 4096,
            max_source_revision_bytes: 64 * 1024 * 1024,
            max_threads: 4096,
            max_thread_name_bytes: 4096,
            max_queued_breakpoint_events: 128,
        }
    }
}

impl DebugOperationConfig {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.operation_timeout.is_zero()
            || self.max_breakpoint_sources == 0
            || self.max_breakpoints_per_source == 0
            || self.max_total_breakpoints == 0
            || self.max_breakpoint_expression_bytes == 0
            || self.max_breakpoint_message_bytes == 0
            || self.max_source_path_bytes == 0
            || self.max_source_revision_bytes == 0
            || self.max_threads == 0
            || self.max_thread_name_bytes == 0
            || self.max_queued_breakpoint_events == 0
            || self.max_breakpoints_per_source > self.max_total_breakpoints
            || tokio::time::Instant::now()
                .checked_add(self.operation_timeout)
                .is_none()
        {
            return Err(crate::DapError::InvalidManagerConfiguration {
                message: "debug operation limits must be non-zero, per-source breakpoints must not exceed the total, and timeout must fit the instant range".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn validation_boundary_matrix_covers_every_limit_relation_and_timeout() {
        let minimal = DebugOperationConfig {
            operation_timeout: Duration::from_nanos(1),
            max_breakpoint_sources: 1,
            max_breakpoints_per_source: 1,
            max_total_breakpoints: 1,
            max_breakpoint_expression_bytes: 1,
            max_breakpoint_message_bytes: 1,
            max_source_path_bytes: 1,
            max_source_revision_bytes: 1,
            max_threads: 1,
            max_thread_name_bytes: 1,
            max_queued_breakpoint_events: 1,
        };
        assert!(minimal.validate().is_ok());

        let mut invalid = Vec::new();
        macro_rules! zero {
            ($field:ident) => {{
                let mut value = minimal.clone();
                value.$field = 0;
                invalid.push(value);
            }};
        }
        let mut zero_timeout = minimal.clone();
        zero_timeout.operation_timeout = Duration::ZERO;
        invalid.push(zero_timeout);
        zero!(max_breakpoint_sources);
        zero!(max_breakpoints_per_source);
        zero!(max_total_breakpoints);
        zero!(max_breakpoint_expression_bytes);
        zero!(max_breakpoint_message_bytes);
        zero!(max_source_path_bytes);
        zero!(max_source_revision_bytes);
        zero!(max_threads);
        zero!(max_thread_name_bytes);
        zero!(max_queued_breakpoint_events);
        let mut relation = minimal.clone();
        relation.max_breakpoints_per_source = 2;
        invalid.push(relation);
        let mut overflow = minimal;
        overflow.operation_timeout = Duration::MAX;
        invalid.push(overflow);
        for config in invalid {
            assert!(matches!(
                config.validate(),
                Err(crate::DapError::InvalidManagerConfiguration { .. })
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DebugBreakpointId(pub(crate) u64);

impl DebugBreakpointId {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for DebugBreakpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSourceRevision {
    pub sha256: [u8; 32],
    pub byte_len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSourceBreakpoint {
    pub line: u64,
    pub column: Option<u64>,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

impl DebugSourceBreakpoint {
    pub fn new(line: u64) -> Self {
        Self {
            line,
            column: None,
            condition: None,
            hit_condition: None,
            log_message: None,
        }
    }
    pub fn with_column(mut self, column: u64) -> Self {
        self.column = Some(column);
        self
    }
    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }
    pub fn with_hit_condition(mut self, condition: impl Into<String>) -> Self {
        self.hit_condition = Some(condition.into());
        self
    }
    pub fn with_log_message(mut self, message: impl Into<String>) -> Self {
        self.log_message = Some(message.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSetBreakpointRequest {
    pub source: PathBuf,
    pub expected_revision: Option<DebugSourceRevision>,
    pub breakpoint: DebugSourceBreakpoint,
}

impl DebugSetBreakpointRequest {
    pub fn new(source: impl Into<PathBuf>, breakpoint: DebugSourceBreakpoint) -> Self {
        Self {
            source: source.into(),
            expected_revision: None,
            breakpoint,
        }
    }
    pub fn with_expected_revision(mut self, revision: DebugSourceRevision) -> Self {
        self.expected_revision = Some(revision);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugRemoveBreakpointRequest {
    pub breakpoint_id: DebugBreakpointId,
    pub expected_revision: Option<DebugSourceRevision>,
}

impl DebugRemoveBreakpointRequest {
    pub fn new(breakpoint_id: DebugBreakpointId) -> Self {
        Self {
            breakpoint_id,
            expected_revision: None,
        }
    }
    pub fn with_expected_revision(mut self, revision: DebugSourceRevision) -> Self {
        self.expected_revision = Some(revision);
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugBreakpointLocation {
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub end_line: Option<u64>,
    pub end_column: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugBreakpointReason {
    Pending,
    Failed,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugBreakpoint {
    pub id: DebugBreakpointId,
    pub source: PathBuf,
    pub source_revision: DebugSourceRevision,
    pub requested: DebugSourceBreakpoint,
    pub verified: bool,
    pub reason: Option<DebugBreakpointReason>,
    pub message: Option<String>,
    pub message_truncated_prefix_bytes: usize,
    pub adapter_id: Option<i64>,
    pub resolved: DebugBreakpointLocation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugBreakpointSynchronization {
    Synchronized,
    Indeterminate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSourceBreakpoints {
    pub source: PathBuf,
    pub source_revision: DebugSourceRevision,
    pub generation: u64,
    pub synchronization: DebugBreakpointSynchronization,
    pub breakpoints: Vec<DebugBreakpoint>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugBreakpointsSnapshot {
    pub sources: Vec<DebugSourceBreakpoints>,
    pub total_breakpoints: usize,
    pub unmatched_adapter_events: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugBreakpointMutation {
    Created { breakpoint_id: DebugBreakpointId },
    Existing { breakpoint_id: DebugBreakpointId },
    Removed { breakpoint_id: DebugBreakpointId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugBreakpointMutationResult {
    pub mutation: DebugBreakpointMutation,
    pub source: DebugSourceBreakpoints,
    pub discarded_stale_breakpoints: Vec<DebugBreakpointId>,
}
