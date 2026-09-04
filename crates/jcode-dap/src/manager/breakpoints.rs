use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::source_hash::{ResolvedSource, resolve_source};
use super::*;
use crate::{
    DebugBreakpoint, DebugBreakpointId, DebugBreakpointLocation, DebugBreakpointMutation,
    DebugBreakpointMutationResult, DebugBreakpointReason, DebugBreakpointSynchronization,
    DebugBreakpointsSnapshot, DebugOperationConfig, DebugRemoveBreakpointRequest,
    DebugSetBreakpointRequest, DebugSourceBreakpoint, DebugSourceBreakpoints, DebugSourceRevision,
};

const MAX_DAP_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Default)]
pub(super) struct BreakpointRegistry {
    sources: BTreeMap<PathBuf, SourceRecord>,
    next_id: u64,
    unmatched_events: u64,
    in_flight: Option<BreakpointTransactionInFlight>,
}

struct SourceRecord {
    original: PathBuf,
    relative: PathBuf,
    revision: DebugSourceRevision,
    generation: u64,
    synchronization: DebugBreakpointSynchronization,
    breakpoints: BTreeMap<DebugBreakpointId, DebugBreakpoint>,
}

pub(super) struct BreakpointTransactionInFlight {
    pub source: PathBuf,
    pub bounded_events: VecDeque<QueuedBreakpointEvent>,
    pub overflowed: bool,
}

#[derive(Clone)]
pub(super) struct QueuedBreakpointEvent {
    pub seq: i64,
    pub body: Value,
}

impl BreakpointRegistry {
    fn snapshot(&self) -> DebugBreakpointsSnapshot {
        let sources = self
            .sources
            .values()
            .map(source_snapshot)
            .collect::<Vec<_>>();
        DebugBreakpointsSnapshot {
            total_breakpoints: sources.iter().map(|source| source.breakpoints.len()).sum(),
            sources,
            unmatched_adapter_events: self.unmatched_events,
        }
    }
}

pub(super) fn queue_breakpoint_event(
    data: &mut SessionData,
    seq: i64,
    body: Value,
    operations: &DebugOperationConfig,
) {
    if let Some(transaction) = data.breakpoints.in_flight.as_mut() {
        if transaction.bounded_events.len() < operations.max_queued_breakpoint_events {
            transaction
                .bounded_events
                .push_back(QueuedBreakpointEvent { seq, body });
        } else {
            transaction.overflowed = true;
            for record in data.breakpoints.sources.values_mut() {
                record.synchronization = DebugBreakpointSynchronization::Indeterminate;
            }
        }
    } else {
        apply_breakpoint_event(&mut data.breakpoints, seq, body, operations);
    }
}

pub(super) fn breakpoint_snapshot(entry: &SessionEntry) -> DebugBreakpointsSnapshot {
    lock(&entry.data).breakpoints.snapshot()
}

pub(super) async fn set_breakpoint_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    request: DebugSetBreakpointRequest,
) -> Result<DebugBreakpointMutationResult> {
    let deadline = Instant::now()
        .checked_add(operations.operation_timeout)
        .ok_or(DapError::InvalidRequestTimeout)?;
    let _gate = control::deadline_gate(&entry, deadline, "set breakpoint").await?;
    ensure_live_breakpoint_state(&entry)?;
    let source = resolve_source(
        &entry.workspace,
        &entry,
        &request.source,
        &operations,
        deadline,
    )
    .await?;
    validate_breakpoint(&request.breakpoint, &operations, &entry)?;
    if let Some(expected) = &request.expected_revision
        && expected != &source.revision
    {
        return Err(DapError::DebugSourceRevisionMismatch {
            path: source.relative,
            expected: expected.clone(),
            actual: source.revision,
        });
    }
    let (desired, mutation, discarded, needs_reset, source_modified) = {
        let mut data = lock(&entry.data);
        ensure_open(&entry, &data, "set breakpoint")?;
        check_capabilities(&data.capabilities, &request.breakpoint)?;
        let registry = &mut data.breakpoints;
        let mut total: usize = registry
            .sources
            .values()
            .map(|record| record.breakpoints.len())
            .sum();
        let record = registry.sources.get(&source.canonical);
        if let Some(record) = record
            && record.revision != source.revision
        {
            total = total.saturating_sub(record.breakpoints.len());
        }
        if let Some(record) = record
            && record.revision == source.revision
            && record.synchronization == DebugBreakpointSynchronization::Synchronized
            && let Some(existing) = record
                .breakpoints
                .values()
                .find(|bp| bp.requested == request.breakpoint)
        {
            return Ok(DebugBreakpointMutationResult {
                mutation: DebugBreakpointMutation::Existing {
                    breakpoint_id: existing.id,
                },
                source: source_snapshot(record),
                discarded_stale_breakpoints: Vec::new(),
            });
        }
        if record.is_none() && registry.sources.len() >= operations.max_breakpoint_sources {
            return Err(DapError::BreakpointLimitExceeded {
                scope: "sources",
                limit: operations.max_breakpoint_sources,
            });
        }
        let current_len = record
            .filter(|record| record.revision == source.revision)
            .map_or(0, |record| record.breakpoints.len());
        if current_len >= operations.max_breakpoints_per_source {
            return Err(DapError::BreakpointLimitExceeded {
                scope: "per-source",
                limit: operations.max_breakpoints_per_source,
            });
        }
        if total >= operations.max_total_breakpoints {
            return Err(DapError::BreakpointLimitExceeded {
                scope: "total",
                limit: operations.max_total_breakpoints,
            });
        }
        registry.next_id = registry
            .next_id
            .checked_add(1)
            .ok_or(DapError::SessionIdExhausted)?;
        let id = DebugBreakpointId(registry.next_id);
        let (mut desired, discarded, needs_reset, source_modified) = match record {
            Some(record) if record.revision == source.revision => (
                record.breakpoints.clone(),
                Vec::new(),
                record.synchronization == DebugBreakpointSynchronization::Indeterminate,
                false,
            ),
            Some(record) => (
                BTreeMap::new(),
                record.breakpoints.keys().copied().collect(),
                true,
                true,
            ),
            None => (BTreeMap::new(), Vec::new(), false, false),
        };
        desired.insert(
            id,
            pending_breakpoint(id, &source, request.breakpoint.clone()),
        );
        (
            desired,
            DebugBreakpointMutation::Created { breakpoint_id: id },
            discarded,
            needs_reset,
            source_modified,
        )
    };
    transact(
        Arc::clone(&entry),
        operations,
        source,
        desired,
        mutation,
        discarded,
        needs_reset,
        source_modified,
        deadline,
    )
    .await
}

pub(super) async fn remove_breakpoint_owned(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    request: DebugRemoveBreakpointRequest,
) -> Result<DebugBreakpointMutationResult> {
    let deadline = Instant::now()
        .checked_add(operations.operation_timeout)
        .ok_or(DapError::InvalidRequestTimeout)?;
    let _gate = control::deadline_gate(&entry, deadline, "remove breakpoint").await?;
    ensure_live_breakpoint_state(&entry)?;
    let (source_path, source_revision, desired, needs_reset) = {
        let data = lock(&entry.data);
        ensure_open(&entry, &data, "remove breakpoint")?;
        let Some((_, record)) = data
            .breakpoints
            .sources
            .iter()
            .find(|(_, record)| record.breakpoints.contains_key(&request.breakpoint_id))
        else {
            return Err(DapError::BreakpointNotFound {
                session_id: entry.id,
                breakpoint_id: request.breakpoint_id,
            });
        };
        if let Some(expected) = &request.expected_revision
            && expected != &record.revision
        {
            return Err(DapError::DebugSourceRevisionMismatch {
                path: record.relative.clone(),
                expected: expected.clone(),
                actual: record.revision.clone(),
            });
        }
        let mut desired = record.breakpoints.clone();
        desired.remove(&request.breakpoint_id);
        (
            record.original.clone(),
            record.revision.clone(),
            desired,
            record.synchronization == DebugBreakpointSynchronization::Indeterminate,
        )
    };
    let actual = resolve_source(
        &entry.workspace,
        &entry,
        &source_path,
        &operations,
        deadline,
    )
    .await?;
    let drifted = actual.revision != source_revision;
    let desired = if drifted {
        desired
            .into_iter()
            .map(|(id, breakpoint)| (id, pending_breakpoint(id, &actual, breakpoint.requested)))
            .collect()
    } else {
        desired
    };
    transact(
        Arc::clone(&entry),
        operations,
        actual,
        desired,
        DebugBreakpointMutation::Removed {
            breakpoint_id: request.breakpoint_id,
        },
        Vec::new(),
        needs_reset || drifted,
        drifted,
        deadline,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn transact(
    entry: Arc<SessionEntry>,
    operations: Arc<DebugOperationConfig>,
    source: ResolvedSource,
    desired: BTreeMap<DebugBreakpointId, DebugBreakpoint>,
    mutation: DebugBreakpointMutation,
    discarded: Vec<DebugBreakpointId>,
    reset: bool,
    reset_source_modified: bool,
    deadline: Instant,
) -> Result<DebugBreakpointMutationResult> {
    let client = client_for(&entry, "mutate breakpoints")?;
    let requested = desired
        .values()
        .map(|bp| bp.requested.clone())
        .collect::<Vec<_>>();
    {
        let mut data = lock(&entry.data);
        ensure_open(&entry, &data, "mutate breakpoints")?;
        data.breakpoints.in_flight = Some(BreakpointTransactionInFlight {
            source: source.canonical.clone(),
            bounded_events: VecDeque::new(),
            overflowed: false,
        });
    }
    if reset {
        match send_set(
            &client,
            &source.wire_path,
            &[],
            reset_source_modified,
            deadline,
        )
        .await
        {
            Ok(_) => {
                let mut data = lock(&entry.data);
                data.breakpoints.sources.remove(&source.canonical);
                notify(&entry, &mut data);
            }
            Err(error) => {
                finish_ambiguous(&entry, &source, &desired, &operations);
                return Err(error);
            }
        }
    }
    #[cfg(test)]
    let _predispatch = match tokio::time::timeout_at(
        deadline,
        Arc::clone(&entry.breakpoint_test_gates.0).acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            clear_transaction(&entry, &operations);
            return Err(DapError::TransportClosed);
        }
        Err(_) => {
            clear_transaction(&entry, &operations);
            return Err(DapError::RequestTimeout {
                command: "source revision".to_owned(),
            });
        }
    };
    let before = match resolve_source(
        &entry.workspace,
        &entry,
        &source.original,
        &operations,
        deadline,
    )
    .await
    {
        Ok(before) => before,
        Err(error) => {
            clear_transaction(&entry, &operations);
            return Err(error);
        }
    };
    if before.canonical != source.canonical || before.revision != source.revision {
        clear_transaction(&entry, &operations);
        return Err(DapError::DebugSourceChangedDuringOperation {
            path: source.relative,
            before: source.revision,
            after: before.revision,
        });
    }
    let response = send_set(&client, &source.wire_path, &requested, false, deadline).await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            if matches!(error, DapError::Response { .. }) {
                clear_transaction(&entry, &operations);
            } else {
                finish_ambiguous(&entry, &source, &desired, &operations);
            }
            return Err(error);
        }
    };
    #[cfg(test)]
    let _response_validation = match tokio::time::timeout_at(
        deadline,
        Arc::clone(&entry.breakpoint_test_gates.1).acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            clear_transaction(&entry, &operations);
            return Err(DapError::TransportClosed);
        }
        Err(_) => {
            finish_ambiguous(&entry, &source, &desired, &operations);
            return Err(DapError::RequestTimeout {
                command: "setBreakpoints response validation".to_owned(),
            });
        }
    };
    let parsed = match parse_breakpoints_response(
        &response,
        desired.len(),
        &operations,
        &entry.workspace,
        &source.canonical,
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            finish_ambiguous(&entry, &source, &desired, &operations);
            return Err(error);
        }
    };
    let after = resolve_source(
        &entry.workspace,
        &entry,
        &source.original,
        &operations,
        deadline,
    )
    .await;
    if !matches!(&after, Ok(after) if after.canonical == source.canonical && after.revision == source.revision)
    {
        let compensation = send_set(&client, &source.wire_path, &[], true, deadline).await;
        if compensation.is_err() {
            finish_ambiguous(&entry, &source, &desired, &operations);
            return Err(DapError::BreakpointReconciliationIndeterminate {
                path: source.relative,
                message: "source changed and compensating clear failed".to_owned(),
            });
        }
        discard_source_and_finish(&entry, &source, &operations);
        let after_revision = after
            .ok()
            .map(|resolved| resolved.revision)
            .unwrap_or_else(|| source.revision.clone());
        return Err(DapError::DebugSourceChangedDuringOperation {
            path: source.relative,
            before: source.revision,
            after: after_revision,
        });
    }
    let mut committed = desired;
    for ((_, bp), wire) in committed.iter_mut().zip(parsed) {
        apply_wire_breakpoint(bp, wire, &operations);
    }
    let mut data = lock(&entry.data);
    if entry.closed.load(std::sync::atomic::Ordering::Acquire)
        || data.state.is_terminal()
        || matches!(data.state, DebugSessionState::Terminating)
    {
        data.breakpoints.in_flight = None;
        return Err(DapError::TransportClosed);
    }
    if adapter_ids_collide(&data.breakpoints, &source.canonical, &committed) {
        drop(data);
        finish_ambiguous(&entry, &source, &committed, &operations);
        return Err(DapError::InvalidSetBreakpointsResponse {
            message: "adapter breakpoint id collides with another source".to_owned(),
        });
    }
    let Some(transaction) = data.breakpoints.in_flight.take() else {
        return Err(DapError::BreakpointReconciliationIndeterminate {
            path: source.relative,
            message: "breakpoint transaction marker was lost".to_owned(),
        });
    };
    debug_assert_eq!(transaction.source, source.canonical);
    let generation = data
        .breakpoints
        .sources
        .get(&source.canonical)
        .map_or(1, |record| record.generation.saturating_add(1));
    data.breakpoints.sources.insert(
        source.canonical.clone(),
        SourceRecord {
            original: source.original.clone(),
            relative: source.relative.clone(),
            revision: source.revision.clone(),
            generation,
            synchronization: if transaction.overflowed {
                DebugBreakpointSynchronization::Indeterminate
            } else {
                DebugBreakpointSynchronization::Synchronized
            },
            breakpoints: committed,
        },
    );
    let mut queued = transaction
        .bounded_events
        .into_iter()
        .filter(|event| event.seq > response.seq)
        .collect::<Vec<_>>();
    queued.sort_by_key(|event| event.seq);
    for event in queued {
        if !transaction.overflowed {
            apply_breakpoint_event(&mut data.breakpoints, event.seq, event.body, &operations);
        } else {
            apply_event_for_other_source(
                &mut data.breakpoints,
                &source.canonical,
                event,
                &operations,
            );
        }
    }
    let Some(record) = data.breakpoints.sources.get(&source.canonical) else {
        return Err(DapError::BreakpointReconciliationIndeterminate {
            path: source.relative,
            message: "committed source disappeared during reconciliation".to_owned(),
        });
    };
    let source_result = source_snapshot(record);
    if source_result.breakpoints.is_empty() {
        data.breakpoints.sources.remove(&source.canonical);
    }
    notify(&entry, &mut data);
    if transaction.overflowed {
        return Err(DapError::BreakpointReconciliationIndeterminate {
            path: source.relative,
            message: "breakpoint event queue overflowed".to_owned(),
        });
    }
    Ok(DebugBreakpointMutationResult {
        mutation,
        source: source_result,
        discarded_stale_breakpoints: discarded,
    })
}

async fn send_set(
    client: &DapClient,
    path: &str,
    breakpoints: &[DebugSourceBreakpoint],
    source_modified: bool,
    deadline: Instant,
) -> Result<Response> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let wire = breakpoints
        .iter()
        .map(source_breakpoint_json)
        .collect::<Vec<_>>();
    let mut arguments = json!({"source":{"path":path},"breakpoints":wire});
    if source_modified {
        arguments["sourceModified"] = Value::Bool(true);
    }
    client
        .request("setBreakpoints", Some(arguments), remaining)
        .await
}

fn source_breakpoint_json(bp: &DebugSourceBreakpoint) -> Value {
    let mut value = json!({"line":bp.line});
    if let Some(column) = bp.column {
        value["column"] = json!(column);
    }
    if let Some(condition) = &bp.condition {
        value["condition"] = json!(condition);
    }
    if let Some(condition) = &bp.hit_condition {
        value["hitCondition"] = json!(condition);
    }
    if let Some(message) = &bp.log_message {
        value["logMessage"] = json!(message);
    }
    value
}

#[derive(Deserialize)]
struct SetBody {
    breakpoints: Vec<WireBreakpoint>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireBreakpoint {
    id: Option<i64>,
    verified: bool,
    reason: Option<String>,
    message: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
    end_line: Option<u64>,
    end_column: Option<u64>,
    source: Option<WireSource>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSource {
    path: Option<String>,
    source_reference: Option<i64>,
}

fn parse_breakpoints_response(
    response: &Response,
    count: usize,
    operations: &DebugOperationConfig,
    workspace: &DebugWorkspaceKey,
    expected_source: &Path,
) -> Result<Vec<WireBreakpoint>> {
    let body: SetBody = serde_json::from_value(response.body.clone().ok_or_else(|| {
        DapError::InvalidSetBreakpointsResponse {
            message: "missing response body".to_owned(),
        }
    })?)
    .map_err(|_| DapError::InvalidSetBreakpointsResponse {
        message: "malformed response body".to_owned(),
    })?;
    if body.breakpoints.len() != count {
        return Err(DapError::InvalidSetBreakpointsResponse {
            message: "response cardinality does not match request".to_owned(),
        });
    }
    let mut ids = std::collections::HashSet::new();
    for bp in &body.breakpoints {
        if let Some(source) = &bp.source {
            let path =
                source
                    .path
                    .as_deref()
                    .ok_or_else(|| DapError::InvalidSetBreakpointsResponse {
                        message: "adapter breakpoint source has no path".to_owned(),
                    })?;
            if source.source_reference.is_some() || path.as_bytes().contains(&0) {
                return Err(DapError::InvalidSetBreakpointsResponse {
                    message: "adapter breakpoint source is not path-only".to_owned(),
                });
            }
            let canonical = PathBuf::from(path).canonicalize().map_err(|_| {
                DapError::InvalidSetBreakpointsResponse {
                    message: "adapter breakpoint source cannot be canonicalized".to_owned(),
                }
            })?;
            if !canonical.starts_with(workspace.canonical_root())
                || canonical != expected_source
                || canonical.to_str().is_none()
            {
                return Err(DapError::InvalidSetBreakpointsResponse {
                    message:
                        "adapter breakpoint source does not match the requested workspace file"
                            .to_owned(),
                });
            }
        }
        if let Some(id) = bp.id {
            i32::try_from(id).map_err(|_| DapError::InvalidSetBreakpointsResponse {
                message: "adapter breakpoint id overflow".to_owned(),
            })?;
            if !ids.insert(id) {
                return Err(DapError::InvalidSetBreakpointsResponse {
                    message: "duplicate adapter breakpoint id".to_owned(),
                });
            }
        }
        validate_location(bp)?;
        if bp
            .message
            .as_ref()
            .is_some_and(|m| m.len() > operations.max_breakpoint_message_bytes.saturating_mul(16))
        {
            return Err(DapError::InvalidSetBreakpointsResponse {
                message: "adapter breakpoint message is excessively large".to_owned(),
            });
        }
    }
    Ok(body.breakpoints)
}
fn validate_location(bp: &WireBreakpoint) -> Result<()> {
    for value in [bp.line, bp.column, bp.end_line, bp.end_column]
        .into_iter()
        .flatten()
    {
        if value == 0 || value > MAX_DAP_INTEGER {
            return Err(DapError::InvalidSetBreakpointsResponse {
                message: "invalid breakpoint location".to_owned(),
            });
        }
    }
    Ok(())
}

fn pending_breakpoint(
    id: DebugBreakpointId,
    source: &ResolvedSource,
    requested: DebugSourceBreakpoint,
) -> DebugBreakpoint {
    DebugBreakpoint {
        id,
        source: source.relative.clone(),
        source_revision: source.revision.clone(),
        requested,
        verified: false,
        reason: None,
        message: None,
        message_truncated_prefix_bytes: 0,
        adapter_id: None,
        resolved: DebugBreakpointLocation::default(),
    }
}
fn apply_wire_breakpoint(
    bp: &mut DebugBreakpoint,
    wire: WireBreakpoint,
    operations: &DebugOperationConfig,
) {
    bp.verified = wire.verified;
    bp.adapter_id = wire.id;
    bp.reason = wire
        .reason
        .map(|reason| parse_reason(reason, operations.max_breakpoint_message_bytes));
    let (message, truncated) = truncate(wire.message, operations.max_breakpoint_message_bytes);
    bp.message = message;
    bp.message_truncated_prefix_bytes = truncated;
    bp.resolved = DebugBreakpointLocation {
        line: wire.line,
        column: wire.column,
        end_line: wire.end_line,
        end_column: wire.end_column,
    };
}
fn parse_reason(reason: String, limit: usize) -> DebugBreakpointReason {
    match reason.as_str() {
        "pending" => DebugBreakpointReason::Pending,
        "failed" => DebugBreakpointReason::Failed,
        _ => DebugBreakpointReason::Other(truncate(Some(reason), limit).0.unwrap_or_default()),
    }
}
fn truncate(message: Option<String>, limit: usize) -> (Option<String>, usize) {
    let Some(mut message) = message else {
        return (None, 0);
    };
    if message.len() <= limit {
        return (Some(message), 0);
    }
    let mut start = message.len() - limit;
    while !message.is_char_boundary(start) {
        start += 1
    }
    message.drain(..start);
    (Some(message), start)
}
fn source_snapshot(record: &SourceRecord) -> DebugSourceBreakpoints {
    DebugSourceBreakpoints {
        source: record.relative.clone(),
        source_revision: record.revision.clone(),
        generation: record.generation,
        synchronization: record.synchronization,
        breakpoints: record.breakpoints.values().cloned().collect(),
    }
}

fn validate_breakpoint(
    bp: &DebugSourceBreakpoint,
    operations: &DebugOperationConfig,
    _entry: &SessionEntry,
) -> Result<()> {
    if bp.line == 0
        || bp.line > MAX_DAP_INTEGER
        || bp.column.is_some_and(|v| v == 0 || v > MAX_DAP_INTEGER)
    {
        return Err(DapError::InvalidBreakpoint {
            message: "line and column must be one-based exact JSON integers".to_owned(),
        });
    }
    for value in [&bp.condition, &bp.hit_condition, &bp.log_message]
        .into_iter()
        .flatten()
    {
        if value.is_empty() || value.len() > operations.max_breakpoint_expression_bytes {
            return Err(DapError::InvalidBreakpoint {
                message: "breakpoint expression is empty or exceeds configured byte limit"
                    .to_owned(),
            });
        }
    }
    Ok(())
}
fn check_capabilities(capabilities: &Capabilities, bp: &DebugSourceBreakpoint) -> Result<()> {
    let gates = [
        (
            bp.condition.is_some(),
            "supportsConditionalBreakpoints",
            "condition",
        ),
        (
            bp.hit_condition.is_some(),
            "supportsHitConditionalBreakpoints",
            "hit condition",
        ),
        (bp.log_message.is_some(), "supportsLogPoints", "log message"),
    ];
    for (used, key, operation) in gates {
        if used && capabilities.additional.get(key).and_then(Value::as_bool) != Some(true) {
            return Err(DapError::UnsupportedDapCapability {
                operation,
                capability: key,
            });
        }
    }
    Ok(())
}
fn ensure_live_breakpoint_state(entry: &SessionEntry) -> Result<()> {
    let data = lock(&entry.data);
    ensure_open(entry, &data, "mutate breakpoints")?;
    if !matches!(
        data.state,
        DebugSessionState::Running | DebugSessionState::Stopped(_)
    ) {
        return Err(invalid_transition(entry, &data, "mutate breakpoints"));
    }
    Ok(())
}
fn ensure_open(entry: &SessionEntry, data: &SessionData, operation: &'static str) -> Result<()> {
    if entry.closed.load(std::sync::atomic::Ordering::Acquire)
        || data.state.is_terminal()
        || matches!(data.state, DebugSessionState::Terminating)
    {
        return Err(invalid_transition(entry, data, operation));
    }
    Ok(())
}
fn client_for(entry: &SessionEntry, operation: &'static str) -> Result<DapClient> {
    let data = lock(&entry.data);
    ensure_open(entry, &data, operation)?;
    data.transport
        .as_ref()
        .map(|t| t.client.clone())
        .ok_or_else(|| invalid_transition(entry, &data, operation))
}
fn clear_transaction(entry: &SessionEntry, operations: &DebugOperationConfig) {
    let mut data = lock(&entry.data);
    let queued = data
        .breakpoints
        .in_flight
        .take()
        .map(|transaction| transaction.bounded_events);
    if let Some(queued) = queued {
        let mut queued = queued.into_iter().collect::<Vec<_>>();
        queued.sort_by_key(|event| event.seq);
        for event in queued {
            apply_breakpoint_event(&mut data.breakpoints, event.seq, event.body, operations);
        }
    }
    notify(entry, &mut data);
}

fn finish_ambiguous(
    entry: &SessionEntry,
    source: &ResolvedSource,
    desired: &BTreeMap<DebugBreakpointId, DebugBreakpoint>,
    operations: &DebugOperationConfig,
) {
    let mut data = lock(&entry.data);
    if entry.closed.load(std::sync::atomic::Ordering::Acquire)
        || data.state.is_terminal()
        || matches!(data.state, DebugSessionState::Terminating)
    {
        data.breakpoints.in_flight = None;
        return;
    }
    let queued = data
        .breakpoints
        .in_flight
        .take()
        .map(|transaction| transaction.bounded_events);
    if let Some(queued) = queued {
        for event in queued {
            apply_event_for_other_source(
                &mut data.breakpoints,
                &source.canonical,
                event,
                operations,
            );
        }
    }
    let generation = data
        .breakpoints
        .sources
        .get(&source.canonical)
        .map_or(1, |record| record.generation.saturating_add(1));
    data.breakpoints.sources.insert(
        source.canonical.clone(),
        SourceRecord {
            original: source.original.clone(),
            relative: source.relative.clone(),
            revision: source.revision.clone(),
            generation,
            synchronization: DebugBreakpointSynchronization::Indeterminate,
            breakpoints: desired.clone(),
        },
    );
    notify(entry, &mut data);
}

fn discard_source_and_finish(
    entry: &SessionEntry,
    source: &ResolvedSource,
    operations: &DebugOperationConfig,
) {
    let mut data = lock(&entry.data);
    let queued = data
        .breakpoints
        .in_flight
        .take()
        .map(|transaction| transaction.bounded_events);
    data.breakpoints.sources.remove(&source.canonical);
    if let Some(queued) = queued {
        for event in queued {
            apply_event_for_other_source(
                &mut data.breakpoints,
                &source.canonical,
                event,
                operations,
            );
        }
    }
    notify(entry, &mut data);
}

fn adapter_ids_collide(
    registry: &BreakpointRegistry,
    replacing: &Path,
    proposed: &BTreeMap<DebugBreakpointId, DebugBreakpoint>,
) -> bool {
    let other = registry
        .sources
        .iter()
        .filter(|(path, _)| path.as_path() != replacing)
        .flat_map(|(_, record)| record.breakpoints.values().filter_map(|bp| bp.adapter_id))
        .collect::<std::collections::HashSet<_>>();
    proposed
        .values()
        .filter_map(|bp| bp.adapter_id)
        .any(|id| other.contains(&id))
}

fn apply_event_for_other_source(
    registry: &mut BreakpointRegistry,
    target: &Path,
    event: QueuedBreakpointEvent,
    operations: &DebugOperationConfig,
) {
    let id = event
        .body
        .get("breakpoint")
        .and_then(|bp| bp.get("id"))
        .and_then(Value::as_i64);
    let Some(id) = id else {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
        return;
    };
    let paths = registry
        .sources
        .iter()
        .filter_map(|(path, record)| {
            record
                .breakpoints
                .values()
                .any(|bp| bp.adapter_id == Some(id))
                .then_some(path.clone())
        })
        .collect::<Vec<_>>();
    if paths.len() == 1 && paths[0].as_path() != target {
        apply_breakpoint_event(registry, event.seq, event.body, operations);
    } else {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
    }
}

fn apply_breakpoint_event(
    registry: &mut BreakpointRegistry,
    _seq: i64,
    body: Value,
    operations: &DebugOperationConfig,
) {
    #[derive(Deserialize)]
    struct EventBody {
        reason: String,
        breakpoint: WireBreakpointEvent,
    }
    #[derive(Deserialize)]
    struct WireBreakpointEvent {
        id: Option<i64>,
        verified: Option<bool>,
        message: Option<String>,
        reason: Option<String>,
        line: Option<u64>,
        column: Option<u64>,
        #[serde(rename = "endLine")]
        end_line: Option<u64>,
        #[serde(rename = "endColumn")]
        end_column: Option<u64>,
    }
    let Ok(event) = serde_json::from_value::<EventBody>(body) else {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
        return;
    };
    let Some(id) = event.breakpoint.id else {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
        return;
    };
    let invalid_location = [
        event.breakpoint.line,
        event.breakpoint.column,
        event.breakpoint.end_line,
        event.breakpoint.end_column,
    ]
    .into_iter()
    .flatten()
    .any(|value| value == 0 || value > MAX_DAP_INTEGER);
    if i32::try_from(id).is_err() || invalid_location {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
        return;
    }
    let matches = registry
        .sources
        .iter()
        .filter_map(|(path, record)| {
            record
                .breakpoints
                .values()
                .find(|bp| bp.adapter_id == Some(id))
                .map(|bp| (path.clone(), bp.id))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        registry.unmatched_events = registry.unmatched_events.saturating_add(1);
        return;
    }
    let (path, local_id) = &matches[0];
    match event.reason.as_str() {
        "removed" => {
            let mut remove_source = false;
            if let Some(record) = registry.sources.get_mut(path) {
                record.breakpoints.remove(local_id);
                record.generation = record.generation.saturating_add(1);
                remove_source = record.breakpoints.is_empty()
                    && record.synchronization == DebugBreakpointSynchronization::Synchronized;
            }
            if remove_source {
                registry.sources.remove(path);
            }
        }
        "changed" => {
            let Some(record) = registry.sources.get_mut(path) else {
                return;
            };
            let Some(bp) = record.breakpoints.get_mut(local_id) else {
                return;
            };
            if let Some(v) = event.breakpoint.verified {
                bp.verified = v
            }
            if let Some(v) = event.breakpoint.reason {
                bp.reason = Some(parse_reason(v, operations.max_breakpoint_message_bytes))
            }
            if let Some(v) = event.breakpoint.message {
                let (message, truncated) =
                    truncate(Some(v), operations.max_breakpoint_message_bytes);
                bp.message = message;
                bp.message_truncated_prefix_bytes = truncated;
            }
            bp.resolved = DebugBreakpointLocation {
                line: event.breakpoint.line.or(bp.resolved.line),
                column: event.breakpoint.column.or(bp.resolved.column),
                end_line: event.breakpoint.end_line.or(bp.resolved.end_line),
                end_column: event.breakpoint.end_column.or(bp.resolved.end_column),
            }
        }
        _ => registry.unmatched_events = registry.unmatched_events.saturating_add(1),
    }
}

impl DebugSessionManager {
    pub fn breakpoints(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
    ) -> Result<DebugBreakpointsSnapshot> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        Ok(breakpoints::breakpoint_snapshot(&entry))
    }

    pub async fn set_breakpoint(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
        request: DebugSetBreakpointRequest,
    ) -> Result<DebugBreakpointMutationResult> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(
            async move { breakpoints::set_breakpoint_owned(entry, operations, request).await },
        )
        .await
        .map_err(|error| DapError::DebugOperationTaskFailed {
            operation: "set breakpoint",
            message: error.to_string(),
        })?
    }

    pub async fn remove_breakpoint(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
        request: DebugRemoveBreakpointRequest,
    ) -> Result<DebugBreakpointMutationResult> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        let operations = Arc::clone(&self.core.operations);
        tokio::spawn(async move {
            breakpoints::remove_breakpoint_owned(entry, operations, request).await
        })
        .await
        .map_err(|error| DapError::DebugOperationTaskFailed {
            operation: "remove breakpoint",
            message: error.to_string(),
        })?
    }
}

#[cfg(test)]
mod tests;
