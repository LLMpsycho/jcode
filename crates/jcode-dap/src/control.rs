use std::fmt;

use crate::{DebugSessionState, DebugSessionStateKind, DebugStepInTargetHandle};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DebugThreadId(i64);

impl DebugThreadId {
    pub fn new(id: i32) -> Self {
        Self(i64::from(id))
    }
    pub fn get(self) -> i64 {
        self.0
    }
    pub(crate) fn from_wire(id: i64) -> crate::Result<Self> {
        i32::try_from(id).map_err(|_| crate::DapError::InvalidThreadsResponse {
            message: "thread id is outside signed 32-bit range".to_owned(),
        })?;
        Ok(Self(id))
    }
}

impl fmt::Display for DebugThreadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DebugExecutionRevision(pub(crate) u64);

impl DebugExecutionRevision {
    pub fn get(self) -> u64 {
        self.0
    }
}
impl fmt::Display for DebugExecutionRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugThread {
    pub id: DebugThreadId,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugThreadsSnapshot {
    pub execution_revision: DebugExecutionRevision,
    pub state: DebugSessionStateKind,
    pub stopped_thread_id: Option<DebugThreadId>,
    pub all_threads_stopped: bool,
    pub threads: Vec<DebugThread>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugContinueRequest {
    pub thread_id: Option<DebugThreadId>,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}
impl DebugContinueRequest {
    pub fn with_thread_id(mut self, id: DebugThreadId) -> Self {
        self.thread_id = Some(id);
        self
    }
    pub fn with_expected_execution_revision(mut self, revision: DebugExecutionRevision) -> Self {
        self.expected_execution_revision = Some(revision);
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugPauseRequest {
    pub thread_id: Option<DebugThreadId>,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}
impl DebugPauseRequest {
    pub fn with_thread_id(mut self, id: DebugThreadId) -> Self {
        self.thread_id = Some(id);
        self
    }
    pub fn with_expected_execution_revision(mut self, revision: DebugExecutionRevision) -> Self {
        self.expected_execution_revision = Some(revision);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DebugSteppingGranularity {
    #[default]
    Statement,
    Line,
    Instruction,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugStepRequest {
    pub thread_id: Option<DebugThreadId>,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
    pub granularity: DebugSteppingGranularity,
}
impl DebugStepRequest {
    pub fn with_thread_id(mut self, id: DebugThreadId) -> Self {
        self.thread_id = Some(id);
        self
    }
    pub fn with_expected_execution_revision(mut self, revision: DebugExecutionRevision) -> Self {
        self.expected_execution_revision = Some(revision);
        self
    }
    pub fn with_granularity(mut self, granularity: DebugSteppingGranularity) -> Self {
        self.granularity = granularity;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugTargetedStepInRequest {
    pub target: DebugStepInTargetHandle,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
    pub granularity: DebugSteppingGranularity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugControlOperation {
    Continue,
    Pause,
    StepOver,
    StepIn,
    StepOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugControlResult {
    pub operation: DebugControlOperation,
    pub thread_id: DebugThreadId,
    pub all_threads_continued: Option<bool>,
    pub state: DebugSessionState,
    pub execution_revision: DebugExecutionRevision,
}
