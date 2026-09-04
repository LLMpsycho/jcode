use super::*;
use crate::{
    DebugEvaluateContext, DebugEvaluateOutcome, DebugEvaluateOutcomeUnknown, DebugEvaluateRequest,
    DebugEvaluateResult, DebugEvaluateTarget, DebugEvaluateUnknownReason,
    DebugInspectionCapability, DebugInspectionError, DebugInspectionLimit,
    DebugInspectionOperation, DebugInspectionRequestReason, DebugInspectionResponseReason,
    DebugScope, DebugScopeLocation, DebugScopePresentationHint, DebugScopesRequest,
    DebugScopesResult, DebugSource, DebugSourcePresentationHint, DebugStackFrame,
    DebugStackFrameHandle, DebugStackFrameLocation, DebugStackFramePresentationHint,
    DebugStackTraceRequest, DebugStackTraceResult, DebugThreadId, DebugVariable,
    DebugVariableAttribute, DebugVariableFilter, DebugVariableHandle, DebugVariableKind,
    DebugVariablePresentationHint, DebugVariableValue, DebugVariableVisibility,
    DebugVariablesRequest, DebugVariablesResult,
};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::Instant;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
type IResult<T> = std::result::Result<T, DebugInspectionError>;
#[derive(Clone)]
struct Captured {
    entry: Arc<SessionEntry>,
    client: DapClient,
    owns_adapter: bool,
    revision: u64,
    transport_revision: u64,
    stopped_thread: Option<i32>,
    all_stopped: bool,
    capabilities: Capabilities,
}

struct InspectionRequestFailure {
    error: DebugInspectionError,
    post_admission: bool,
    unknown_transport: Option<DebugEvaluateUnknownReason>,
}

fn token_error(
    cause: InspectionTokenCause,
    post_admission: bool,
    operation: DebugInspectionOperation,
) -> DebugInspectionError {
    match cause {
        InspectionTokenCause::AdapterOrTargetExited => {
            DebugInspectionError::AdapterOrTargetExited { operation }
        }
        InspectionTokenCause::TransportFailure => {
            DebugInspectionError::TransportFailure { operation }
        }
        InspectionTokenCause::ClientReplaced => DebugInspectionError::ClientReplaced { operation },
        InspectionTokenCause::ManagerOrOwnerShutdown => {
            DebugInspectionError::ManagerOrOwnerShutdown { operation }
        }
        InspectionTokenCause::Revision if post_admission => {
            DebugInspectionError::LifecycleInvalidated { operation }
        }
        InspectionTokenCause::Revision => DebugInspectionError::StaleExecutionRevision,
    }
}

async fn inspection_request(
    captured: &Captured,
    owner: &str,
    thread: Option<i32>,
    command: &'static str,
    arguments: Option<Value>,
    deadline: Instant,
    operation: DebugInspectionOperation,
) -> std::result::Result<Response, InspectionRequestFailure> {
    let token = InspectionToken::new(captured.client.clone());
    {
        let _publication = lock(&captured.entry.publication);
        {
            let data = lock(&captured.entry.data);
            final_recheck(captured, owner, thread, &data, operation).map_err(|error| {
                InspectionRequestFailure {
                    error,
                    post_admission: false,
                    unknown_transport: None,
                }
            })?;
        }
        captured.entry.register_inspection(&token);
    }
    let gate_token = Arc::clone(&token);
    let mut tracked = captured
        .client
        .tracked_request_with_admission_gate(
            command,
            arguments,
            remaining(deadline, operation).map_err(|error| InspectionRequestFailure {
                error,
                post_admission: false,
                unknown_transport: None,
            })?,
            Box::new(move || gate_token.admit()),
        )
        .map_err(|error| {
            let invalidation = token.invalidation();
            let post_admission = token.is_post_admission();
            InspectionRequestFailure {
                error: invalidation
                    .map(|(post, cause)| token_error(cause, post, operation))
                    .unwrap_or_else(|| map_captured_transport(captured, error.clone(), operation)),
                post_admission,
                unknown_transport: (post_admission && invalidation.is_none())
                    .then(|| map_unknown_transport(error)),
            }
        })?;
    token.attach(tracked.invalidator());
    let result = (&mut tracked).await;
    match result {
        Ok(response) => {
            token.mark_response_won();
            Ok(response)
        }
        Err(error) => {
            let post_admission = token.is_post_admission();
            let invalidation = token.invalidation();
            token.settle();
            Err(InspectionRequestFailure {
                error: invalidation
                    .map(|(post, cause)| token_error(cause, post, operation))
                    .unwrap_or_else(|| map_captured_transport(captured, error.clone(), operation)),
                post_admission,
                unknown_transport: (post_admission && invalidation.is_none())
                    .then(|| map_unknown_transport(error)),
            })
        }
    }
}
impl DebugSessionManager {
    pub async fn stack_trace(
        &self,
        _owner_session_id: &str,
        _id: DebugSessionId,
        _request: DebugStackTraceRequest,
    ) -> std::result::Result<DebugStackTraceResult, DebugInspectionError> {
        stack_trace_owned(self, _owner_session_id, _id, _request).await
    }
    pub async fn scopes(
        &self,
        _owner_session_id: &str,
        _id: DebugSessionId,
        _request: DebugScopesRequest,
    ) -> std::result::Result<DebugScopesResult, DebugInspectionError> {
        scopes_owned(self, _owner_session_id, _id, _request).await
    }
    pub async fn variables(
        &self,
        _owner_session_id: &str,
        _id: DebugSessionId,
        _request: DebugVariablesRequest,
    ) -> std::result::Result<DebugVariablesResult, DebugInspectionError> {
        variables_owned(self, _owner_session_id, _id, _request).await
    }
    pub async fn evaluate(
        &self,
        _owner_session_id: &str,
        _id: DebugSessionId,
        _request: DebugEvaluateRequest,
    ) -> std::result::Result<DebugEvaluateOutcome, DebugInspectionError> {
        evaluate_owned(self, _owner_session_id, _id, _request).await
    }
}
async fn stack_trace_owned(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    request: DebugStackTraceRequest,
) -> IResult<DebugStackTraceResult> {
    let op = DebugInspectionOperation::StackTrace;
    let deadline = operation_deadline(manager, op)?;
    let captured = capture(manager, owner, id, request.expected_execution_revision, op)?;
    if request.levels == 0 {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::ZeroPageSize,
        ));
    }
    if usize::try_from(request.levels).map_or(true, |n| {
        n > manager.core.inspection.max_stack_frames_per_response
    }) {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::PageSizeExceedsConfiguredMaximum,
        ));
    }
    let delayed = capability(&captured.capabilities, "supportsDelayedStackTraceLoading");
    if request.start_frame > 0 && !delayed {
        return Err(unsupported(
            op,
            DebugInspectionCapability::DelayedStackTraceLoading,
        ));
    }
    let _operation = operation_gate(&captured.entry, deadline, op).await?;
    let thread = resolve_thread(
        &captured,
        owner,
        request.thread_id,
        manager.core.operations.max_threads,
        deadline,
    )
    .await?;
    let mut arguments = json!({"threadId": thread});
    if delayed {
        arguments["startFrame"] = json!(request.start_frame);
        arguments["levels"] = json!(request.levels);
    }
    recheck(&captured, owner, Some(thread), None, op)?;
    let response = inspection_request(
        &captured,
        owner,
        Some(thread),
        "stackTrace",
        Some(arguments),
        deadline,
        op,
    )
    .await
    .map_err(|failure| failure.error)?;
    validate_response(&response, "stackTrace", op)?;
    let body = parse_body(&captured.entry, response.body, deadline, op).await?;
    let parse_manager = manager.clone();
    let start_frame = request.start_frame;
    let levels = request.levels;
    let (parsed, identities, total_frames, next_start_frame) =
        parse_bounded(&captured.entry, deadline, op, move || {
            let frames_value = body
                .get("stackFrames")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
                })?;
            if frames_value.len() > usize::try_from(levels).unwrap_or(usize::MAX) {
                return Err(invalid_response(
                    op,
                    DebugInspectionResponseReason::ReturnedMoreThanRequested,
                ));
            }
            let mut aggregate = 0usize;
            let mut seen = HashSet::new();
            let mut parsed = Vec::with_capacity(frames_value.len());
            let mut identities = Vec::with_capacity(frames_value.len());
            for (offset, value) in frames_value.iter().enumerate() {
                let object = object(value, op)?;
                let frame_id = signed_i32(required(object, "id", op)?, op)?;
                if !seen.insert(frame_id) {
                    return Err(invalid_response(
                        op,
                        DebugInspectionResponseReason::DuplicateFrameId,
                    ));
                }
                let absolute = start_frame
                    .checked_add(u32::try_from(offset).map_err(|_| {
                        invalid_response(op, DebugInspectionResponseReason::InvalidInteger)
                    })?)
                    .ok_or_else(|| {
                        invalid_request(op, DebugInspectionRequestReason::PageOffsetOverflow)
                    })?;
                identities.push((frame_id, (thread, absolute)));
                let name = bounded_required_string(
                    object,
                    "name",
                    parse_manager.core.inspection.max_name_bytes,
                    DebugInspectionLimit::NameBytes,
                    op,
                    &mut aggregate,
                    &parse_manager.core.inspection,
                )?;
                let line = position(required(object, "line", op)?, op)?;
                let column = position(required(object, "column", op)?, op)?;
                let end_line = optional_position(object.get("endLine"), op)?;
                let end_column = optional_position(object.get("endColumn"), op)?;
                let source =
                    parse_source(object.get("source"), &parse_manager, op, &mut aggregate)?;
                let hint = match object.get("presentationHint") {
                    None => None,
                    Some(Value::String(s)) => Some(match s.as_str() {
                        "normal" => DebugStackFramePresentationHint::Normal,
                        "label" => DebugStackFramePresentationHint::Label,
                        "subtle" => DebugStackFramePresentationHint::Subtle,
                        _ => {
                            return Err(invalid_response(
                                op,
                                DebugInspectionResponseReason::InvalidPresentationHint,
                            ));
                        }
                    }),
                    Some(_) => {
                        return Err(invalid_response(
                            op,
                            DebugInspectionResponseReason::InvalidPresentationHint,
                        ));
                    }
                };
                parsed.push((
                    frame_id,
                    name,
                    DebugStackFrameLocation {
                        source,
                        line,
                        column,
                        end_line,
                        end_column,
                    },
                    hint,
                ));
            }
            let total_frames = body
                .get("totalFrames")
                .map(|v| unsigned_u32(v, op))
                .transpose()?;
            let returned = u32::try_from(parsed.len())
                .map_err(|_| invalid_response(op, DebugInspectionResponseReason::TooManyItems))?;
            let next_start_frame = if delayed && returned > 0 && returned == levels {
                let next = start_frame.checked_add(returned).ok_or_else(|| {
                    invalid_request(op, DebugInspectionRequestReason::PageOffsetOverflow)
                })?;
                if total_frames.is_none_or(|total| next < total) {
                    Some(next)
                } else {
                    None
                }
            } else {
                None
            };
            Ok((parsed, identities, total_frames, next_start_frame))
        })
        .await?;
    let _publication = lock(&captured.entry.publication);
    let mut data = lock(&captured.entry.data);
    remaining(deadline, op)?;
    final_recheck(&captured, owner, Some(thread), &data, op)?;
    let mut proposed = data.frame_identities.clone();
    for (id, identity) in identities {
        if let Some(existing) = proposed.get(&id) {
            if existing != &identity {
                return Err(invalid_response(
                    op,
                    DebugInspectionResponseReason::DuplicateFrameId,
                ));
            }
        } else {
            if proposed.len()
                >= manager
                    .core
                    .inspection
                    .max_frame_handles_per_execution_revision
            {
                return Err(DebugInspectionError::LimitExceeded {
                    operation: op,
                    limit: DebugInspectionLimit::FrameHandlesPerExecutionRevision,
                });
            }
            proposed.insert(id, identity);
        }
    }
    data.frame_identities = proposed;
    let revision = crate::DebugExecutionRevision(captured.revision);
    let frames = parsed
        .into_iter()
        .map(
            |(frame_id, name, location, presentation_hint)| DebugStackFrame {
                handle: DebugStackFrameHandle {
                    manager_id: manager.core.manager_id,
                    session_id: id,
                    execution_revision: revision,
                    thread_id: thread,
                    frame_id,
                },
                name,
                location,
                presentation_hint,
            },
        )
        .collect();
    Ok(DebugStackTraceResult {
        thread_id: DebugThreadId::from_wire(i64::from(thread))
            .map_err(|_| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))?,
        execution_revision: revision,
        frames,
        total_frames,
        next_start_frame,
    })
}
async fn scopes_owned(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    request: DebugScopesRequest,
) -> IResult<DebugScopesResult> {
    let op = DebugInspectionOperation::Scopes;
    let deadline = operation_deadline(manager, op)?;
    let captured = capture(manager, owner, id, request.expected_execution_revision, op)?;
    validate_frame(manager, id, &request.frame, captured.revision)?;
    ensure_frame_issued(&captured, &request.frame)?;
    let _operation = operation_gate(&captured.entry, deadline, op).await?;
    recheck(
        &captured,
        owner,
        Some(request.frame.thread_id),
        Some(&request.frame),
        op,
    )?;
    let response = inspection_request(
        &captured,
        owner,
        Some(request.frame.thread_id),
        "scopes",
        Some(json!({"frameId":request.frame.frame_id})),
        deadline,
        op,
    )
    .await
    .map_err(|failure| failure.error)?;
    validate_response(&response, "scopes", op)?;
    let body = parse_body(&captured.entry, response.body, deadline, op).await?;
    let parse_manager = manager.clone();
    let parse_captured = captured.clone();
    let thread_id = request.frame.thread_id;
    let scopes = parse_bounded(&captured.entry, deadline, op, move || {
        let values = body
            .get("scopes")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
            })?;
        if values.len() > parse_manager.core.inspection.max_scopes_per_response {
            return Err(DebugInspectionError::LimitExceeded {
                operation: op,
                limit: DebugInspectionLimit::Scopes,
            });
        }
        let mut aggregate = 0;
        let mut scopes = Vec::with_capacity(values.len());
        for value in values {
            let o = object(value, op)?;
            let name = bounded_required_string(
                o,
                "name",
                parse_manager.core.inspection.max_name_bytes,
                DebugInspectionLimit::NameBytes,
                op,
                &mut aggregate,
                &parse_manager.core.inspection,
            )?;
            let reference = nonnegative_i32(required(o, "variablesReference", op)?, op)?;
            let hint = optional_string(
                o.get("presentationHint"),
                parse_manager.core.inspection.max_presentation_string_bytes,
                DebugInspectionLimit::PresentationStringBytes,
                op,
                &mut aggregate,
                &parse_manager.core.inspection,
            )?
            .map(scope_hint);
            let source = parse_source(o.get("source"), &parse_manager, op, &mut aggregate)?;
            scopes.push(DebugScope {
                name,
                presentation_hint: hint,
                variables: variable_handle(
                    &parse_manager,
                    id,
                    parse_captured.revision,
                    Some(thread_id),
                    reference,
                ),
                named_variables: optional_count(o.get("namedVariables"), op)?,
                indexed_variables: optional_count(o.get("indexedVariables"), op)?,
                expensive: o.get("expensive").and_then(Value::as_bool).ok_or_else(|| {
                    invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
                })?,
                location: DebugScopeLocation {
                    source,
                    line: optional_position(o.get("line"), op)?,
                    column: optional_position(o.get("column"), op)?,
                    end_line: optional_position(o.get("endLine"), op)?,
                    end_column: optional_position(o.get("endColumn"), op)?,
                },
            });
        }
        Ok(scopes)
    })
    .await?;
    publish_result(
        &captured,
        owner,
        Some(request.frame.thread_id),
        deadline,
        op,
        |_| {
            Ok(DebugScopesResult {
                execution_revision: crate::DebugExecutionRevision(captured.revision),
                scopes,
            })
        },
    )
}
async fn variables_owned(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    request: DebugVariablesRequest,
) -> IResult<DebugVariablesResult> {
    let op = DebugInspectionOperation::Variables;
    let deadline = operation_deadline(manager, op)?;
    let captured = capture(manager, owner, id, request.expected_execution_revision, op)?;
    validate_variable(manager, id, &request.variables, captured.revision)?;
    if !request.variables.is_expandable() {
        return Err(DebugInspectionError::VariableNotExpandable);
    }
    if request.count == 0 {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::ZeroPageSize,
        ));
    }
    if usize::try_from(request.count).map_or(true, |n| {
        n > manager.core.inspection.max_variables_per_response
    }) {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::PageSizeExceedsConfiguredMaximum,
        ));
    }
    let _operation = operation_gate(&captured.entry, deadline, op).await?;
    let mut args =
        json!({"variablesReference":request.variables.variables_reference,"count":request.count});
    if request.start != 0 {
        args["start"] = json!(request.start);
    }
    if let Some(filter) = request.filter {
        args["filter"] = json!(match filter {
            DebugVariableFilter::Named => "named",
            DebugVariableFilter::Indexed => "indexed",
        });
    }
    recheck(&captured, owner, request.variables.thread_id, None, op)?;
    let response = inspection_request(
        &captured,
        owner,
        request.variables.thread_id,
        "variables",
        Some(args),
        deadline,
        op,
    )
    .await
    .map_err(|failure| failure.error)?;
    validate_response(&response, "variables", op)?;
    let body = parse_body(&captured.entry, response.body, deadline, op).await?;
    let parse_manager = manager.clone();
    let parse_captured = captured.clone();
    let count = request.count;
    let thread_id = request.variables.thread_id;
    let variables = parse_bounded(&captured.entry, deadline, op, move || {
        let values = body
            .get("variables")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
            })?;
        if values.len() > usize::try_from(count).unwrap_or(usize::MAX) {
            return Err(invalid_response(
                op,
                DebugInspectionResponseReason::ReturnedMoreThanRequested,
            ));
        }
        let mut aggregate = 0;
        let mut variables = Vec::with_capacity(values.len());
        for value in values {
            variables.push(parse_variable(
                value,
                &parse_manager,
                id,
                &parse_captured,
                thread_id,
                op,
                &mut aggregate,
            )?);
        }
        Ok(variables)
    })
    .await?;
    let returned = u32::try_from(variables.len())
        .map_err(|_| invalid_response(op, DebugInspectionResponseReason::TooManyItems))?;
    let next_start =
        if returned == request.count {
            Some(request.start.checked_add(returned).ok_or_else(|| {
                invalid_request(op, DebugInspectionRequestReason::PageOffsetOverflow)
            })?)
        } else {
            None
        };
    publish_result(
        &captured,
        owner,
        request.variables.thread_id,
        deadline,
        op,
        |_| {
            Ok(DebugVariablesResult {
                execution_revision: crate::DebugExecutionRevision(captured.revision),
                variables,
                next_start,
            })
        },
    )
}
async fn evaluate_owned(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    request: DebugEvaluateRequest,
) -> IResult<DebugEvaluateOutcome> {
    let op = DebugInspectionOperation::Evaluate;
    let deadline = operation_deadline(manager, op)?;
    let captured = capture(
        manager,
        owner,
        id,
        Some(request.expected_execution_revision),
        op,
    )?;
    if request.expression.is_empty() {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::EmptyExpression,
        ));
    }
    if request.expression.len() > manager.core.inspection.max_evaluate_expression_bytes {
        return Err(invalid_request(
            op,
            DebugInspectionRequestReason::ExpressionTooLarge,
        ));
    }
    match request.context {
        DebugEvaluateContext::Hover
            if !capability(&captured.capabilities, "supportsEvaluateForHovers") =>
        {
            return Err(unsupported(
                op,
                DebugInspectionCapability::EvaluateForHovers,
            ));
        }
        DebugEvaluateContext::Clipboard
            if !capability(&captured.capabilities, "supportsClipboardContext") =>
        {
            return Err(unsupported(op, DebugInspectionCapability::ClipboardContext));
        }
        _ => {}
    }
    let (frame_id, thread_id) = match &request.target {
        DebugEvaluateTarget::Global => (None, captured.stopped_thread),
        DebugEvaluateTarget::Frame(frame) => {
            validate_frame(manager, id, frame, captured.revision)?;
            ensure_frame_issued(&captured, frame)?;
            (Some(frame.frame_id), Some(frame.thread_id))
        }
    };
    let _operation = operation_gate(&captured.entry, deadline, op).await?;
    let mut args = json!({"expression":request.expression});
    let context = match request.context {
        DebugEvaluateContext::Unspecified => None,
        DebugEvaluateContext::Watch => Some("watch"),
        DebugEvaluateContext::Repl => Some("repl"),
        DebugEvaluateContext::Hover => Some("hover"),
        DebugEvaluateContext::Clipboard => Some("clipboard"),
        DebugEvaluateContext::Variables => Some("variables"),
    };
    if let Some(context) = context {
        args["context"] = json!(context);
    }
    if let Some(frame) = frame_id {
        args["frameId"] = json!(frame);
    }
    recheck(&captured, owner, thread_id, None, op)?;
    let response = match inspection_request(
        &captured,
        owner,
        thread_id,
        "evaluate",
        Some(args),
        deadline,
        op,
    )
    .await
    {
        Ok(response) => response,
        Err(failure) if failure.post_admission => {
            return Ok(unknown(
                failure
                    .unknown_transport
                    .unwrap_or_else(|| map_unknown_inspection(failure.error)),
            ));
        }
        Err(failure) => return Err(failure.error),
    };
    if response.command != "evaluate" {
        return Ok(unknown(DebugEvaluateUnknownReason::CommandMismatch));
    }
    if !response.success {
        return Ok(unknown(
            if response.message.as_deref() == Some("cancelled") {
                DebugEvaluateUnknownReason::CancelledByAdapter
            } else {
                DebugEvaluateUnknownReason::AdapterRejected
            },
        ));
    }
    let body = match parse_body(&captured.entry, response.body, deadline, op).await {
        Ok(body) => body,
        Err(DebugInspectionError::LimitExceeded { .. }) => {
            return Ok(unknown(DebugEvaluateUnknownReason::OversizedSuccess));
        }
        Err(DebugInspectionError::InvalidResponse { .. }) => {
            return Ok(unknown(DebugEvaluateUnknownReason::MalformedSuccess));
        }
        Err(error) => return Ok(unknown(map_unknown_inspection(error))),
    };
    let parse_manager = manager.clone();
    let parse_captured = captured.clone();
    let parsed = parse_bounded(&captured.entry, deadline, op, move || {
        let mut aggregate = 0;
        let result = bounded_required_string(
            &body,
            "result",
            parse_manager.core.inspection.max_evaluate_result_bytes,
            DebugInspectionLimit::EvaluateResultBytes,
            op,
            &mut aggregate,
            &parse_manager.core.inspection,
        )?;
        let type_name = optional_string(
            body.get("type"),
            parse_manager.core.inspection.max_type_bytes,
            DebugInspectionLimit::TypeBytes,
            op,
            &mut aggregate,
            &parse_manager.core.inspection,
        )?;
        let reference = nonnegative_i32(required(&body, "variablesReference", op)?, op)?;
        let presentation_hint = parse_variable_hint(
            body.get("presentationHint"),
            &parse_manager,
            op,
            &mut aggregate,
        )?;
        Ok(DebugEvaluateResult {
            execution_revision: crate::DebugExecutionRevision(parse_captured.revision),
            result,
            type_name,
            presentation_hint,
            variables: variable_handle(
                &parse_manager,
                id,
                parse_captured.revision,
                thread_id,
                reference,
            ),
            named_variables: optional_count(body.get("namedVariables"), op)?,
            indexed_variables: optional_count(body.get("indexedVariables"), op)?,
        })
    })
    .await;
    let result = match parsed {
        Ok(v) => v,
        Err(DebugInspectionError::LimitExceeded { .. }) => {
            return Ok(unknown(DebugEvaluateUnknownReason::OversizedSuccess));
        }
        Err(_) => return Ok(unknown(DebugEvaluateUnknownReason::MalformedSuccess)),
    };
    match publish_result(&captured, owner, thread_id, deadline, op, |_| {
        Ok(DebugEvaluateOutcome::Known(result))
    }) {
        Ok(outcome) => Ok(outcome),
        Err(error) => Ok(unknown(map_unknown_inspection(error))),
    }
}
fn capture(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    expected: Option<crate::DebugExecutionRevision>,
    _op: DebugInspectionOperation,
) -> IResult<Captured> {
    let entry = {
        let registry = lock(&manager.core.registry);
        registry
            .entries
            .get(&id)
            .cloned()
            .ok_or(DebugInspectionError::SessionNotFound)?
    };
    if entry.owner_session_id != owner {
        return Err(DebugInspectionError::SessionAccessDenied);
    }
    let _publication = lock(&entry.publication);
    let data = lock(&entry.data);
    if matches!(
        data.state,
        DebugSessionState::Terminating | DebugSessionState::Ended(_)
    ) {
        return Err(DebugInspectionError::SessionTerminal);
    }
    if entry.closed.load(Ordering::Acquire) || data.transport.is_none() {
        return Err(DebugInspectionError::SessionClosed);
    }
    if !matches!(data.state, DebugSessionState::Stopped(_)) {
        return Err(DebugInspectionError::SessionNotStopped);
    }
    if expected.is_some_and(|r| r.0 != data.execution_revision) {
        return Err(DebugInspectionError::StaleExecutionRevision);
    }
    let transport = data
        .transport
        .as_ref()
        .ok_or(DebugInspectionError::SessionClosed)?;
    let (stopped_thread, all_stopped) = match &data.state {
        DebugSessionState::Stopped(s) => (
            s.thread_id.map(i32::try_from).transpose().map_err(|_| {
                invalid_response(
                    DebugInspectionOperation::StackTrace,
                    DebugInspectionResponseReason::InvalidInteger,
                )
            })?,
            s.all_threads_stopped,
        ),
        _ => unreachable!(),
    };
    Ok(Captured {
        entry: Arc::clone(&entry),
        client: transport.client.clone(),
        owns_adapter: transport.adapter.is_some(),
        revision: data.execution_revision,
        transport_revision: data.transport_revision,
        stopped_thread,
        all_stopped,
        capabilities: data.capabilities.clone(),
    })
}
async fn resolve_thread(
    c: &Captured,
    owner: &str,
    requested: Option<DebugThreadId>,
    max_threads: usize,
    deadline: Instant,
) -> IResult<i32> {
    let requested = requested
        .map(DebugThreadId::get)
        .map(i32::try_from)
        .transpose()
        .map_err(|_| {
            invalid_response(
                DebugInspectionOperation::StackTrace,
                DebugInspectionResponseReason::InvalidInteger,
            )
        })?;
    if let Some(id) = requested {
        if Some(id) == c.stopped_thread {
            return Ok(id);
        }
        if !c.all_stopped {
            return Err(DebugInspectionError::StoppedThreadUnavailable);
        }
    } else if let Some(id) = c.stopped_thread {
        return Ok(id);
    } else if !c.all_stopped {
        return Err(DebugInspectionError::StoppedThreadUnavailable);
    }
    recheck(
        c,
        owner,
        c.stopped_thread,
        None,
        DebugInspectionOperation::StackTrace,
    )?;
    let response = inspection_request(
        c,
        owner,
        c.stopped_thread,
        "threads",
        None,
        deadline,
        DebugInspectionOperation::StackTrace,
    )
    .await
    .map_err(|failure| failure.error)?;
    validate_response(&response, "threads", DebugInspectionOperation::StackTrace)?;
    let body = parse_body(
        &c.entry,
        response.body,
        deadline,
        DebugInspectionOperation::StackTrace,
    )
    .await?;
    let threads = body
        .get("threads")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            invalid_response(
                DebugInspectionOperation::StackTrace,
                DebugInspectionResponseReason::MissingRequiredField,
            )
        })?;
    if threads.len() > max_threads {
        return Err(DebugInspectionError::AmbiguousStoppedThread);
    }
    let mut ids = Vec::with_capacity(threads.len());
    let mut seen = HashSet::new();
    for thread in threads {
        let id = signed_i32(
            required(
                object(thread, DebugInspectionOperation::StackTrace)?,
                "id",
                DebugInspectionOperation::StackTrace,
            )?,
            DebugInspectionOperation::StackTrace,
        )?;
        if !seen.insert(id) {
            return Err(invalid_response(
                DebugInspectionOperation::StackTrace,
                DebugInspectionResponseReason::InvalidInteger,
            ));
        }
        ids.push(id);
    }
    match requested {
        Some(id) if ids.iter().filter(|v| **v == id).count() == 1 => Ok(id),
        Some(_) => Err(DebugInspectionError::ThreadNotFound),
        None if ids.len() == 1 => Ok(ids[0]),
        None => Err(DebugInspectionError::AmbiguousStoppedThread),
    }
}
async fn parse_body(
    entry: &SessionEntry,
    body: Option<Value>,
    deadline: Instant,
    op: DebugInspectionOperation,
) -> IResult<Map<String, Value>> {
    let permit = tokio::time::timeout_at(
        deadline,
        Arc::clone(&entry.inspection_parse).acquire_owned(),
    )
    .await
    .map_err(|_| DebugInspectionError::Timeout { operation: op })?
    .map_err(|_| DebugInspectionError::InternalFailure { operation: op })?;
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        body.and_then(|v| v.as_object().cloned())
            .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::MissingBody))
    });
    tokio::time::timeout_at(deadline, &mut task)
        .await
        .map_err(|_| DebugInspectionError::Timeout { operation: op })?
        .map_err(|_| DebugInspectionError::InternalFailure { operation: op })?
}
async fn parse_bounded<T>(
    entry: &SessionEntry,
    deadline: Instant,
    op: DebugInspectionOperation,
    parse: impl FnOnce() -> IResult<T> + Send + 'static,
) -> IResult<T>
where
    T: Send + 'static,
{
    let permit = tokio::time::timeout_at(
        deadline,
        Arc::clone(&entry.inspection_parse).acquire_owned(),
    )
    .await
    .map_err(|_| DebugInspectionError::Timeout { operation: op })?
    .map_err(|_| DebugInspectionError::InternalFailure { operation: op })?;
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        parse()
    });
    tokio::time::timeout_at(deadline, &mut task)
        .await
        .map_err(|_| DebugInspectionError::Timeout { operation: op })?
        .map_err(|_| DebugInspectionError::InternalFailure { operation: op })?
}
fn validate_response(
    response: &Response,
    command: &str,
    op: DebugInspectionOperation,
) -> IResult<()> {
    if response.command != command {
        return Err(invalid_response(
            op,
            DebugInspectionResponseReason::CommandMismatch,
        ));
    }
    if !response.success {
        return Err(DebugInspectionError::AdapterRejected { operation: op });
    }
    Ok(())
}
fn recheck(
    c: &Captured,
    owner: &str,
    thread: Option<i32>,
    frame: Option<&DebugStackFrameHandle>,
    op: DebugInspectionOperation,
) -> IResult<()> {
    let _p = lock(&c.entry.publication);
    let data = lock(&c.entry.data);
    final_recheck(c, owner, thread, &data, op)?;
    if let Some(frame) = frame
        && !matches!(
            data.frame_identities.get(&frame.frame_id),
            Some((thread_id, _)) if *thread_id == frame.thread_id
        )
    {
        return Err(DebugInspectionError::FrameUnavailable);
    }
    Ok(())
}
fn publish_result<T>(
    c: &Captured,
    owner: &str,
    thread: Option<i32>,
    deadline: Instant,
    op: DebugInspectionOperation,
    publish: impl FnOnce(&mut SessionData) -> IResult<T>,
) -> IResult<T> {
    let _p = lock(&c.entry.publication);
    let mut data = lock(&c.entry.data);
    remaining(deadline, op)?;
    final_recheck(c, owner, thread, &data, op)?;
    publish(&mut data)
}
fn final_recheck(
    c: &Captured,
    owner: &str,
    thread: Option<i32>,
    data: &SessionData,
    op: DebugInspectionOperation,
) -> IResult<()> {
    if c.entry.owner_session_id != owner {
        return Err(DebugInspectionError::SessionAccessDenied);
    }
    if matches!(
        data.state,
        DebugSessionState::Terminating | DebugSessionState::Ended(_)
    ) {
        return Err(invalidation_error(data, op).unwrap_or(DebugInspectionError::SessionTerminal));
    }
    if c.entry.closed.load(Ordering::Acquire) || data.transport.is_none() {
        return Err(invalidation_error(data, op).unwrap_or(DebugInspectionError::SessionClosed));
    }
    if data.execution_revision != c.revision {
        return Err(DebugInspectionError::LifecycleInvalidated { operation: op });
    }
    if data.transport_revision != c.transport_revision {
        return Err(DebugInspectionError::ClientReplaced { operation: op });
    }
    if !data
        .transport
        .as_ref()
        .is_some_and(|transport| transport.client.is_exact(&c.client))
    {
        return Err(DebugInspectionError::ClientReplaced { operation: op });
    }
    match &data.state {
        DebugSessionState::Stopped(s)
            if thread.is_none()
                || s.all_threads_stopped
                || s.thread_id == thread.map(i64::from) =>
        {
            Ok(())
        }
        _ => Err(DebugInspectionError::LifecycleInvalidated { operation: op }),
    }
}
fn captured_invalidation_error(
    captured: &Captured,
    operation: DebugInspectionOperation,
) -> Option<DebugInspectionError> {
    let _publication = lock(&captured.entry.publication);
    invalidation_error(&lock(&captured.entry.data), operation)
}
fn invalidation_error(
    data: &SessionData,
    operation: DebugInspectionOperation,
) -> Option<DebugInspectionError> {
    data.inspection_invalidation.map(|cause| match cause {
        InspectionInvalidation::AdapterOrTargetExited => {
            DebugInspectionError::AdapterOrTargetExited { operation }
        }
        InspectionInvalidation::TransportFailure => {
            DebugInspectionError::TransportFailure { operation }
        }
        InspectionInvalidation::ManagerOrOwnerShutdown => {
            DebugInspectionError::ManagerOrOwnerShutdown { operation }
        }
    })
}
fn map_captured_transport(
    captured: &Captured,
    error: DapError,
    operation: DebugInspectionOperation,
) -> DebugInspectionError {
    captured_invalidation_error(captured, operation).unwrap_or_else(|| {
        if captured.owns_adapter
            && matches!(
                captured.client.status().close_cause,
                Some(crate::client::ClientCloseCause::ReaderEof)
            )
        {
            DebugInspectionError::AdapterOrTargetExited { operation }
        } else {
            map_transport(error, operation)
        }
    })
}
fn ensure_frame_issued(c: &Captured, frame: &DebugStackFrameHandle) -> IResult<()> {
    let _publication = lock(&c.entry.publication);
    match lock(&c.entry.data).frame_identities.get(&frame.frame_id) {
        Some((thread, _)) if *thread == frame.thread_id => Ok(()),
        _ => Err(DebugInspectionError::FrameUnavailable),
    }
}
fn operation_deadline(
    m: &DebugSessionManager,
    operation: DebugInspectionOperation,
) -> IResult<Instant> {
    Instant::now()
        .checked_add(m.core.operations.operation_timeout)
        .ok_or(DebugInspectionError::InternalFailure { operation })
}
async fn operation_gate<'a>(
    entry: &'a SessionEntry,
    deadline: Instant,
    operation: DebugInspectionOperation,
) -> IResult<tokio::sync::MutexGuard<'a, ()>> {
    tokio::time::timeout_at(deadline, entry.operation.lock())
        .await
        .map_err(|_| DebugInspectionError::Timeout { operation })
}
fn remaining(
    deadline: Instant,
    operation: DebugInspectionOperation,
) -> IResult<std::time::Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(DebugInspectionError::Timeout { operation })
}
fn validate_frame(
    m: &DebugSessionManager,
    id: DebugSessionId,
    h: &DebugStackFrameHandle,
    revision: u64,
) -> IResult<()> {
    if h.manager_id != m.core.manager_id {
        Err(DebugInspectionError::HandleManagerMismatch)
    } else if h.session_id != id {
        Err(DebugInspectionError::HandleSessionMismatch)
    } else if h.execution_revision.0 != revision {
        Err(DebugInspectionError::StaleHandle)
    } else {
        Ok(())
    }
}
fn validate_variable(
    m: &DebugSessionManager,
    id: DebugSessionId,
    h: &DebugVariableHandle,
    revision: u64,
) -> IResult<()> {
    if h.manager_id != m.core.manager_id {
        Err(DebugInspectionError::HandleManagerMismatch)
    } else if h.session_id != id {
        Err(DebugInspectionError::HandleSessionMismatch)
    } else if h.execution_revision.0 != revision {
        Err(DebugInspectionError::StaleHandle)
    } else {
        Ok(())
    }
}
#[path = "inspection_parse.rs"]
mod inspection_parse;
use inspection_parse::*;
