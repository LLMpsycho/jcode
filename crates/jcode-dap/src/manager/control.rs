use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::*;
use crate::{
    DebugContinueRequest, DebugControlOperation, DebugControlResult, DebugExecutionRevision,
    DebugOperationConfig, DebugPauseRequest, DebugStepRequest, DebugSteppingGranularity,
    DebugThread, DebugThreadId, DebugThreadsSnapshot,
};

pub(super) async fn threads_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
) -> Result<DebugThreadsSnapshot> {
    let deadline = operation_deadline(&operations)?;
    let _gate = deadline_gate(&entry, deadline, "threads").await?;
    request_threads_locked(&entry, &operations, deadline).await
}

pub(super) async fn continue_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    request: DebugContinueRequest,
) -> Result<DebugControlResult> {
    control_owned(
        entry,
        operations,
        DebugControlOperation::Continue,
        request.thread_id,
        request.expected_execution_revision,
        DebugSteppingGranularity::Statement,
    )
    .await
}
pub(super) async fn pause_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    request: DebugPauseRequest,
) -> Result<DebugControlResult> {
    control_owned(
        entry,
        operations,
        DebugControlOperation::Pause,
        request.thread_id,
        request.expected_execution_revision,
        DebugSteppingGranularity::Statement,
    )
    .await
}
pub(super) async fn step_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    operation: DebugControlOperation,
    request: DebugStepRequest,
) -> Result<DebugControlResult> {
    control_owned(
        entry,
        operations,
        operation,
        request.thread_id,
        request.expected_execution_revision,
        request.granularity,
    )
    .await
}

async fn control_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    operation: DebugControlOperation,
    requested_thread: Option<DebugThreadId>,
    expected: Option<DebugExecutionRevision>,
    granularity: DebugSteppingGranularity,
) -> Result<DebugControlResult> {
    let deadline = operation_deadline(&operations)?;
    let _gate = deadline_gate(&entry, deadline, operation_name(operation)).await?;
    let (initial_revision, state, stopped_id, all_stopped, capabilities) = {
        let data = lock(&entry.data);
        ensure_control_state(&entry, &data, operation)?;
        if data.execution_revision == u64::MAX
            && matches!(
                operation,
                DebugControlOperation::Continue
                    | DebugControlOperation::StepOver
                    | DebugControlOperation::StepIn
                    | DebugControlOperation::StepOut
            )
        {
            return Err(DapError::ExecutionRevisionExhausted {
                session_id: entry.id,
            });
        }
        let revision = DebugExecutionRevision(data.execution_revision);
        validate_expected(&entry, expected, revision)?;
        let (stopped_id, all_stopped) = match &data.state {
            DebugSessionState::Stopped(stopped) => (
                stopped
                    .thread_id
                    .map(DebugThreadId::from_wire)
                    .transpose()?,
                stopped.all_threads_stopped,
            ),
            _ => (None, false),
        };
        (
            revision,
            data.state.kind(),
            stopped_id,
            all_stopped,
            data.capabilities.clone(),
        )
    };
    validate_granularity(operation, granularity, &capabilities)?;
    let thread_id = select_thread(
        &entry,
        &operations,
        deadline,
        operation,
        requested_thread,
        stopped_id,
        all_stopped,
    )
    .await?;
    let client = {
        let data = lock(&entry.data);
        ensure_control_state(&entry, &data, operation)?;
        let actual = DebugExecutionRevision(data.execution_revision);
        if actual != initial_revision || data.state.kind() != state {
            return Err(DapError::StaleExecutionRevision {
                session_id: entry.id,
                expected: initial_revision,
                actual,
            });
        }
        data.transport
            .as_ref()
            .map(|transport| transport.client.clone())
            .ok_or_else(|| invalid_transition(&entry, &data, operation_name(operation)))?
    };
    let command = command(operation);
    let mut arguments = json!({"threadId":thread_id.get()});
    if matches!(
        operation,
        DebugControlOperation::StepOver
            | DebugControlOperation::StepIn
            | DebugControlOperation::StepOut
    ) {
        match granularity {
            DebugSteppingGranularity::Statement => {}
            DebugSteppingGranularity::Line => arguments["granularity"] = json!("line"),
            DebugSteppingGranularity::Instruction => {
                arguments["granularity"] = json!("instruction")
            }
        }
    }
    let response = client
        .request(
            command,
            Some(arguments),
            deadline.saturating_duration_since(Instant::now()),
        )
        .await;
    let timed_out = matches!(response, Err(DapError::RequestTimeout { .. }));
    let all_threads_continued = match response {
        Ok(response) => parse_control_response(operation, response.body)?,
        Err(error) => {
            if timed_out
                && matches!(
                    operation,
                    DebugControlOperation::Continue
                        | DebugControlOperation::StepOver
                        | DebugControlOperation::StepIn
                        | DebugControlOperation::StepOut
                )
            {
                commit_running_if_current(&entry, initial_revision);
            }
            return Err(error);
        }
    };
    let mut data = lock(&entry.data);
    if !entry.closed.load(Ordering::Acquire)
        && data.execution_revision == initial_revision.0
        && matches!(
            operation,
            DebugControlOperation::Continue
                | DebugControlOperation::StepOver
                | DebugControlOperation::StepIn
                | DebugControlOperation::StepOut
        )
    {
        debug_assert!(entry.advance_execution(&mut data));
        data.state = DebugSessionState::Running;
        notify(&entry, &mut data);
    }
    Ok(DebugControlResult {
        operation,
        thread_id,
        all_threads_continued,
        state: data.state.clone(),
        execution_revision: DebugExecutionRevision(data.execution_revision),
    })
}

async fn select_thread(
    entry: &SessionEntry,
    operations: &DebugOperationConfig,
    deadline: Instant,
    operation: DebugControlOperation,
    requested: Option<DebugThreadId>,
    stopped: Option<DebugThreadId>,
    all_stopped: bool,
) -> Result<DebugThreadId> {
    if operation != DebugControlOperation::Pause {
        if let Some(requested) = requested
            && Some(requested) == stopped
        {
            return Ok(requested);
        }
        if requested.is_some() && !all_stopped {
            return Err(DapError::StoppedThreadUnavailable {
                session_id: entry.id,
            });
        }
        if requested.is_none()
            && let Some(stopped) = stopped
        {
            return Ok(stopped);
        }
        if !all_stopped {
            return Err(DapError::StoppedThreadUnavailable {
                session_id: entry.id,
            });
        }
    }
    let snapshot = request_threads_locked(entry, operations, deadline).await?;
    if let Some(requested) = requested {
        if snapshot.threads.iter().any(|thread| thread.id == requested) {
            return Ok(requested);
        }
        return Err(DapError::ThreadNotFound {
            session_id: entry.id,
            thread_id: requested,
        });
    }
    if snapshot.threads.len() == 1 {
        return Ok(snapshot.threads[0].id);
    }
    let observed = snapshot.threads.len();
    if operation == DebugControlOperation::Pause {
        Err(DapError::AmbiguousThreadSelection {
            session_id: entry.id,
            observed_threads: observed,
        })
    } else {
        Err(DapError::AmbiguousStoppedThread {
            session_id: entry.id,
            observed_threads: observed,
        })
    }
}

async fn request_threads_locked(
    entry: &SessionEntry,
    operations: &DebugOperationConfig,
    deadline: Instant,
) -> Result<DebugThreadsSnapshot> {
    let (client, revision, state, stopped_thread_id, all_threads_stopped) = {
        let data = lock(&entry.data);
        if !matches!(
            data.state,
            DebugSessionState::Running | DebugSessionState::Stopped(_)
        ) {
            return Err(invalid_transition(entry, &data, "list threads"));
        }
        let (stopped, all) = match &data.state {
            DebugSessionState::Stopped(s) => (
                s.thread_id.map(DebugThreadId::from_wire).transpose()?,
                s.all_threads_stopped,
            ),
            _ => (None, false),
        };
        let client = data
            .transport
            .as_ref()
            .map(|t| t.client.clone())
            .ok_or_else(|| invalid_transition(entry, &data, "list threads"))?;
        (
            client,
            DebugExecutionRevision(data.execution_revision),
            data.state.kind(),
            stopped,
            all,
        )
    };
    let response = client
        .request(
            "threads",
            None,
            deadline.saturating_duration_since(Instant::now()),
        )
        .await?;
    #[derive(Deserialize)]
    struct Body {
        threads: Vec<WireThread>,
    }
    #[derive(Deserialize)]
    struct WireThread {
        id: i64,
        name: String,
    }
    let body: Body =
        serde_json::from_value(
            response
                .body
                .ok_or_else(|| DapError::InvalidThreadsResponse {
                    message: "missing response body".to_owned(),
                })?,
        )
        .map_err(|_| DapError::InvalidThreadsResponse {
            message: "malformed response body".to_owned(),
        })?;
    if body.threads.len() > operations.max_threads {
        return Err(DapError::ThreadLimitExceeded {
            observed: body.threads.len(),
            limit: operations.max_threads,
        });
    }
    let mut seen = HashSet::new();
    let mut threads = Vec::with_capacity(body.threads.len());
    for thread in body.threads {
        let id = DebugThreadId::from_wire(thread.id)?;
        if !seen.insert(id) {
            return Err(DapError::InvalidThreadsResponse {
                message: "duplicate thread id".to_owned(),
            });
        }
        if thread.name.len() > operations.max_thread_name_bytes {
            return Err(DapError::InvalidThreadsResponse {
                message: "thread name exceeds configured byte limit".to_owned(),
            });
        }
        threads.push(DebugThread {
            id,
            name: thread.name,
        });
    }
    Ok(DebugThreadsSnapshot {
        execution_revision: revision,
        state,
        stopped_thread_id,
        all_threads_stopped,
        threads,
    })
}

fn parse_control_response(
    operation: DebugControlOperation,
    body: Option<Value>,
) -> Result<Option<bool>> {
    if operation != DebugControlOperation::Continue {
        return Ok(None);
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        #[serde(default = "default_true")]
        all_threads_continued: bool,
    }
    fn default_true() -> bool {
        true
    }
    let body: Body =
        serde_json::from_value(body.ok_or_else(|| {
            DapError::InvalidMessage("continue response is missing body".to_owned())
        })?)
        .map_err(|_| DapError::InvalidMessage("malformed continue response body".to_owned()))?;
    Ok(Some(body.all_threads_continued))
}
fn operation_deadline(config: &DebugOperationConfig) -> Result<Instant> {
    Instant::now()
        .checked_add(config.operation_timeout)
        .ok_or(DapError::InvalidRequestTimeout)
}

pub(super) async fn deadline_gate<'a>(
    entry: &'a SessionEntry,
    deadline: Instant,
    command: &'static str,
) -> Result<tokio::sync::MutexGuard<'a, ()>> {
    tokio::time::timeout_at(deadline, entry.operation.lock())
        .await
        .map_err(|_| DapError::RequestTimeout {
            command: command.to_owned(),
        })
}
fn operation_name(operation: DebugControlOperation) -> &'static str {
    match operation {
        DebugControlOperation::Continue => "continue",
        DebugControlOperation::Pause => "pause",
        DebugControlOperation::StepOver => "step over",
        DebugControlOperation::StepIn => "step in",
        DebugControlOperation::StepOut => "step out",
    }
}
fn command(operation: DebugControlOperation) -> &'static str {
    match operation {
        DebugControlOperation::Continue => "continue",
        DebugControlOperation::Pause => "pause",
        DebugControlOperation::StepOver => "next",
        DebugControlOperation::StepIn => "stepIn",
        DebugControlOperation::StepOut => "stepOut",
    }
}
fn ensure_control_state(
    entry: &SessionEntry,
    data: &SessionData,
    operation: DebugControlOperation,
) -> Result<()> {
    if entry.closed.load(Ordering::Acquire) {
        return Err(invalid_transition(entry, data, operation_name(operation)));
    }
    let valid = match operation {
        DebugControlOperation::Pause => matches!(data.state, DebugSessionState::Running),
        _ => matches!(data.state, DebugSessionState::Stopped(_)),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_transition(entry, data, operation_name(operation)))
    }
}
fn validate_expected(
    entry: &SessionEntry,
    expected: Option<DebugExecutionRevision>,
    actual: DebugExecutionRevision,
) -> Result<()> {
    if let Some(expected) = expected
        && expected != actual
    {
        return Err(DapError::StaleExecutionRevision {
            session_id: entry.id,
            expected,
            actual,
        });
    }
    Ok(())
}
fn validate_granularity(
    operation: DebugControlOperation,
    granularity: DebugSteppingGranularity,
    capabilities: &Capabilities,
) -> Result<()> {
    if matches!(
        operation,
        DebugControlOperation::StepOver
            | DebugControlOperation::StepIn
            | DebugControlOperation::StepOut
    ) && granularity != DebugSteppingGranularity::Statement
        && capabilities
            .additional
            .get("supportsSteppingGranularity")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(DapError::UnsupportedDapCapability {
            operation: "stepping granularity",
            capability: "supportsSteppingGranularity",
        });
    }
    Ok(())
}
fn commit_running_if_current(entry: &SessionEntry, revision: DebugExecutionRevision) {
    let mut data = lock(&entry.data);
    if !entry.closed.load(Ordering::Acquire)
        && data.execution_revision == revision.0
        && matches!(data.state, DebugSessionState::Stopped(_))
    {
        if entry.advance_execution(&mut data) {
            data.state = DebugSessionState::Running;
        }
        notify(entry, &mut data)
    }
}

impl DebugSessionManager {
    pub async fn threads(&self, owner: &str, id: DebugSessionId) -> Result<DebugThreadsSnapshot> {
        let entry = self.core.authorized_entry(owner, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(async move { control::threads_owned(entry, operations).await })
            .await
            .map_err(|error| DapError::DebugOperationTaskFailed {
                operation: "threads",
                message: error.to_string(),
            })?
    }

    pub async fn continue_execution(
        &self,
        owner: &str,
        id: DebugSessionId,
        request: DebugContinueRequest,
    ) -> Result<DebugControlResult> {
        let entry = self.core.authorized_entry(owner, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(async move { control::continue_owned(entry, operations, request).await })
            .await
            .map_err(|error| DapError::DebugOperationTaskFailed {
                operation: "continue",
                message: error.to_string(),
            })?
    }

    pub async fn pause(
        &self,
        owner: &str,
        id: DebugSessionId,
        request: DebugPauseRequest,
    ) -> Result<DebugControlResult> {
        let entry = self.core.authorized_entry(owner, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(async move { control::pause_owned(entry, operations, request).await })
            .await
            .map_err(|error| DapError::DebugOperationTaskFailed {
                operation: "pause",
                message: error.to_string(),
            })?
    }

    pub async fn step_over(
        &self,
        owner: &str,
        id: DebugSessionId,
        request: DebugStepRequest,
    ) -> Result<DebugControlResult> {
        self.step(owner, id, DebugControlOperation::StepOver, request)
            .await
    }
    pub async fn step_in(
        &self,
        owner: &str,
        id: DebugSessionId,
        request: DebugStepRequest,
    ) -> Result<DebugControlResult> {
        self.step(owner, id, DebugControlOperation::StepIn, request)
            .await
    }
    pub async fn step_out(
        &self,
        owner: &str,
        id: DebugSessionId,
        request: DebugStepRequest,
    ) -> Result<DebugControlResult> {
        self.step(owner, id, DebugControlOperation::StepOut, request)
            .await
    }

    async fn step(
        &self,
        owner: &str,
        id: DebugSessionId,
        operation: DebugControlOperation,
        request: DebugStepRequest,
    ) -> Result<DebugControlResult> {
        let entry = self.core.authorized_entry(owner, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(
            async move { control::step_owned(entry, operations, operation, request).await },
        )
        .await
        .map_err(|error| DapError::DebugOperationTaskFailed {
            operation: "step",
            message: error.to_string(),
        })?
    }
}

#[cfg(test)]
mod tests;
