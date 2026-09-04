use super::*;

const MAX_STEP_IN_TARGETS_PER_RESPONSE: usize = 256;

pub(super) async fn step_in_targets_owned(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    request: DebugStepInTargetsRequest,
) -> IResult<DebugStepInTargetsResult> {
    let op = DebugInspectionOperation::StepInTargets;
    let deadline = operation_deadline(manager, op)?;
    let captured = capture(manager, owner, id, request.expected_execution_revision, op)?;
    validate_frame(manager, id, &request.frame, captured.revision)?;
    ensure_frame_issued(&captured, &request.frame)?;
    if !capability(&captured.capabilities, "supportsStepInTargetsRequest") {
        return Err(unsupported(op, DebugInspectionCapability::StepInTargets));
    }
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
        "stepInTargets",
        Some(json!({"frameId": request.frame.frame_id})),
        deadline,
        op,
    )
    .await
    .map_err(|failure| failure.error)?;
    validate_response(&response, "stepInTargets", op)?;
    let body = parse_body(&captured.entry, response.body, deadline, op).await?;
    let parse_manager = manager.clone();
    let parse_captured = captured.clone();
    let thread_id = request.frame.thread_id;
    let frame_id = request.frame.frame_id;
    let targets = parse_bounded(&captured.entry, deadline, op, move || {
        let values = body
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
            })?;
        if values.len() > MAX_STEP_IN_TARGETS_PER_RESPONSE {
            return Err(DebugInspectionError::LimitExceeded {
                operation: op,
                limit: DebugInspectionLimit::StepInTargets,
            });
        }
        let mut aggregate = 0;
        let mut seen = HashSet::new();
        let mut targets = Vec::with_capacity(values.len());
        for value in values {
            let object = object(value, op)?;
            let target_id = signed_i32(required(object, "id", op)?, op)?;
            if !seen.insert(target_id) {
                return Err(invalid_response(
                    op,
                    DebugInspectionResponseReason::DuplicateTargetId,
                ));
            }
            let label = bounded_required_string(
                object,
                "label",
                parse_manager.core.inspection.max_name_bytes,
                DebugInspectionLimit::NameBytes,
                op,
                &mut aggregate,
                &parse_manager.core.inspection,
            )?;
            let instruction_pointer_reference = optional_string(
                object.get("instructionPointerReference"),
                parse_manager.core.inspection.max_presentation_string_bytes,
                DebugInspectionLimit::PresentationStringBytes,
                op,
                &mut aggregate,
                &parse_manager.core.inspection,
            )?;
            targets.push(DebugStepInTarget {
                handle: DebugStepInTargetHandle {
                    manager_id: parse_manager.core.manager_id,
                    session_id: id,
                    execution_revision: DebugExecutionRevision(parse_captured.revision),
                    thread_id,
                    frame_id,
                    target_id,
                },
                label,
                line: optional_position(object.get("line"), op)?,
                column: optional_position(object.get("column"), op)?,
                end_line: optional_position(object.get("endLine"), op)?,
                end_column: optional_position(object.get("endColumn"), op)?,
                instruction_pointer_reference,
            });
        }
        Ok(targets)
    })
    .await?;
    publish_result(
        &captured,
        owner,
        Some(request.frame.thread_id),
        deadline,
        op,
        |_| {
            Ok(DebugStepInTargetsResult {
                execution_revision: DebugExecutionRevision(captured.revision),
                targets,
            })
        },
    )
}
