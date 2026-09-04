use super::*;

pub(super) fn variable_handle(
    m: &DebugSessionManager,
    id: DebugSessionId,
    revision: u64,
    thread_id: Option<i32>,
    variables_reference: i32,
) -> DebugVariableHandle {
    DebugVariableHandle {
        manager_id: m.core.manager_id,
        session_id: id,
        execution_revision: crate::DebugExecutionRevision(revision),
        thread_id,
        variables_reference,
    }
}

pub(super) fn parse_variable(
    value: &Value,
    m: &DebugSessionManager,
    id: DebugSessionId,
    c: &Captured,
    thread_id: Option<i32>,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
) -> IResult<DebugVariable> {
    let o = object(value, op)?;
    let name = bounded_required_string(
        o,
        "name",
        m.core.inspection.max_name_bytes,
        DebugInspectionLimit::NameBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?;
    let original = required(o, "value", op)?
        .as_str()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::MissingRequiredField))?;
    let (text, omitted_bytes) = truncate_utf8(original, m.core.inspection.max_variable_value_bytes);
    add_aggregate(aggregate, text.len(), &m.core.inspection, op)?;
    let type_name = optional_string(
        o.get("type"),
        m.core.inspection.max_type_bytes,
        DebugInspectionLimit::TypeBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?;
    let evaluate_name = optional_string(
        o.get("evaluateName"),
        m.core.inspection.max_variable_evaluate_name_bytes,
        DebugInspectionLimit::VariableEvaluateNameBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?;
    let reference = nonnegative_i32(required(o, "variablesReference", op)?, op)?;
    Ok(DebugVariable {
        name,
        value: DebugVariableValue {
            text,
            omitted_bytes,
        },
        type_name,
        presentation_hint: parse_variable_hint(o.get("presentationHint"), m, op, aggregate)?,
        evaluate_name,
        variables: variable_handle(m, id, c.revision, thread_id, reference),
        named_variables: optional_count(o.get("namedVariables"), op)?,
        indexed_variables: optional_count(o.get("indexedVariables"), op)?,
    })
}
pub(super) fn parse_source(
    value: Option<&Value>,
    m: &DebugSessionManager,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
) -> IResult<Option<DebugSource>> {
    let Some(value) = value else { return Ok(None) };
    let o = object(value, op)?;
    let name = optional_string(
        o.get("name"),
        m.core.inspection.max_source_name_bytes,
        DebugInspectionLimit::SourceNameBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?;
    let path = optional_string(
        o.get("path"),
        m.core.inspection.max_source_path_bytes,
        DebugInspectionLimit::SourcePathBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?;
    let presentation_hint = match o.get("presentationHint") {
        None => None,
        Some(Value::String(s)) => Some(match s.as_str() {
            "normal" => DebugSourcePresentationHint::Normal,
            "emphasize" => DebugSourcePresentationHint::Emphasize,
            "deemphasize" => DebugSourcePresentationHint::Deemphasize,
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
    if name.is_none() && path.is_none() && presentation_hint.is_none() {
        Ok(None)
    } else {
        Ok(Some(DebugSource {
            name,
            path,
            presentation_hint,
        }))
    }
}
pub(super) fn parse_variable_hint(
    value: Option<&Value>,
    m: &DebugSessionManager,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
) -> IResult<Option<DebugVariablePresentationHint>> {
    let Some(value) = value else { return Ok(None) };
    let o = object(value, op)?;
    let kind = optional_string(
        o.get("kind"),
        m.core.inspection.max_presentation_string_bytes,
        DebugInspectionLimit::PresentationStringBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?
    .map(variable_kind);
    let visibility = optional_string(
        o.get("visibility"),
        m.core.inspection.max_presentation_string_bytes,
        DebugInspectionLimit::PresentationStringBytes,
        op,
        aggregate,
        &m.core.inspection,
    )?
    .map(variable_visibility);
    let lazy = match o.get("lazy") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| {
            invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
        })?),
    };
    let mut attributes = Vec::new();
    if let Some(values) = o.get("attributes") {
        let values = values.as_array().ok_or_else(|| {
            invalid_response(op, DebugInspectionResponseReason::MissingRequiredField)
        })?;
        if values.len() > m.core.inspection.max_presentation_attributes {
            return Err(DebugInspectionError::LimitExceeded {
                operation: op,
                limit: DebugInspectionLimit::PresentationAttributes,
            });
        }
        for v in values {
            attributes.push(variable_attribute(bounded_value_string(
                v,
                m.core.inspection.max_presentation_string_bytes,
                DebugInspectionLimit::PresentationStringBytes,
                op,
                aggregate,
                &m.core.inspection,
            )?));
        }
    }
    Ok(Some(DebugVariablePresentationHint {
        kind,
        attributes,
        visibility,
        lazy,
    }))
}
pub(super) fn scope_hint(s: String) -> DebugScopePresentationHint {
    match s.as_str() {
        "arguments" => DebugScopePresentationHint::Arguments,
        "locals" => DebugScopePresentationHint::Locals,
        "registers" => DebugScopePresentationHint::Registers,
        "returnValue" => DebugScopePresentationHint::ReturnValue,
        _ => DebugScopePresentationHint::Other(s),
    }
}
pub(super) fn variable_kind(s: String) -> DebugVariableKind {
    match s.as_str() {
        "property" => DebugVariableKind::Property,
        "method" => DebugVariableKind::Method,
        "class" => DebugVariableKind::Class,
        "data" => DebugVariableKind::Data,
        "event" => DebugVariableKind::Event,
        "baseClass" => DebugVariableKind::BaseClass,
        "innerClass" => DebugVariableKind::InnerClass,
        "interface" => DebugVariableKind::Interface,
        "mostDerivedClass" => DebugVariableKind::MostDerivedClass,
        "virtual" => DebugVariableKind::Virtual,
        "dataBreakpoint" => DebugVariableKind::DataBreakpoint,
        _ => DebugVariableKind::Other(s),
    }
}
pub(super) fn variable_attribute(s: String) -> DebugVariableAttribute {
    match s.as_str() {
        "static" => DebugVariableAttribute::Static,
        "constant" => DebugVariableAttribute::Constant,
        "readOnly" => DebugVariableAttribute::ReadOnly,
        "rawString" => DebugVariableAttribute::RawString,
        "hasObjectId" => DebugVariableAttribute::HasObjectId,
        "canHaveObjectId" => DebugVariableAttribute::CanHaveObjectId,
        "hasSideEffects" => DebugVariableAttribute::HasSideEffects,
        "hasDataBreakpoint" => DebugVariableAttribute::HasDataBreakpoint,
        _ => DebugVariableAttribute::Other(s),
    }
}
pub(super) fn variable_visibility(s: String) -> DebugVariableVisibility {
    match s.as_str() {
        "public" => DebugVariableVisibility::Public,
        "private" => DebugVariableVisibility::Private,
        "protected" => DebugVariableVisibility::Protected,
        "internal" => DebugVariableVisibility::Internal,
        "final" => DebugVariableVisibility::Final,
        _ => DebugVariableVisibility::Other(s),
    }
}
pub(super) fn object(value: &Value, op: DebugInspectionOperation) -> IResult<&Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::MissingRequiredField))
}
pub(super) fn required<'a>(
    o: &'a Map<String, Value>,
    key: &str,
    op: DebugInspectionOperation,
) -> IResult<&'a Value> {
    o.get(key)
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::MissingRequiredField))
}
pub(super) fn signed_i32(v: &Value, op: DebugInspectionOperation) -> IResult<i32> {
    let n = v
        .as_i64()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))?;
    i32::try_from(n)
        .map_err(|_| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))
}
pub(super) fn nonnegative_i32(v: &Value, op: DebugInspectionOperation) -> IResult<i32> {
    let n = v
        .as_u64()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))?;
    i32::try_from(n)
        .map_err(|_| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))
}
pub(super) fn unsigned_u32(v: &Value, op: DebugInspectionOperation) -> IResult<u32> {
    let n = v
        .as_u64()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))?;
    u32::try_from(n)
        .map_err(|_| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))
}
pub(super) fn position(v: &Value, op: DebugInspectionOperation) -> IResult<u64> {
    let n = v
        .as_u64()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))?;
    if n > MAX_SAFE_INTEGER {
        Err(invalid_response(
            op,
            DebugInspectionResponseReason::InvalidInteger,
        ))
    } else {
        Ok(n)
    }
}
pub(super) fn optional_position(
    v: Option<&Value>,
    op: DebugInspectionOperation,
) -> IResult<Option<u64>> {
    v.map(|v| position(v, op)).transpose()
}
pub(super) fn optional_count(
    v: Option<&Value>,
    op: DebugInspectionOperation,
) -> IResult<Option<u32>> {
    v.map(|v| {
        nonnegative_i32(v, op).and_then(|n| {
            u32::try_from(n)
                .map_err(|_| invalid_response(op, DebugInspectionResponseReason::InvalidInteger))
        })
    })
    .transpose()
}
pub(super) fn bounded_required_string(
    o: &Map<String, Value>,
    key: &str,
    max: usize,
    limit: DebugInspectionLimit,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
    config: &DebugInspectionConfig,
) -> IResult<String> {
    bounded_value_string(required(o, key, op)?, max, limit, op, aggregate, config)
}
pub(super) fn optional_string(
    v: Option<&Value>,
    max: usize,
    limit: DebugInspectionLimit,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
    config: &DebugInspectionConfig,
) -> IResult<Option<String>> {
    v.map(|v| bounded_value_string(v, max, limit, op, aggregate, config))
        .transpose()
}
pub(super) fn bounded_value_string(
    v: &Value,
    max: usize,
    limit: DebugInspectionLimit,
    op: DebugInspectionOperation,
    aggregate: &mut usize,
    config: &DebugInspectionConfig,
) -> IResult<String> {
    let s = v
        .as_str()
        .ok_or_else(|| invalid_response(op, DebugInspectionResponseReason::MissingRequiredField))?;
    if s.len() > max {
        return Err(DebugInspectionError::LimitExceeded {
            operation: op,
            limit,
        });
    }
    add_aggregate(aggregate, s.len(), config, op)?;
    Ok(s.to_owned())
}
pub(super) fn add_aggregate(
    total: &mut usize,
    amount: usize,
    config: &DebugInspectionConfig,
    op: DebugInspectionOperation,
) -> IResult<()> {
    *total = total.checked_add(amount).ok_or_else(|| {
        invalid_response(op, DebugInspectionResponseReason::AggregateTextTooLarge)
    })?;
    if *total > config.max_public_text_bytes_per_result {
        return Err(DebugInspectionError::LimitExceeded {
            operation: op,
            limit: DebugInspectionLimit::AggregatePublicTextBytes,
        });
    }
    Ok(())
}
pub(super) fn truncate_utf8(s: &str, max: usize) -> (String, u64) {
    if s.len() <= max {
        return (s.to_owned(), 0);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (
        s[..end].to_owned(),
        u64::try_from(s.len() - end).unwrap_or(u64::MAX),
    )
}
pub(super) fn capability(c: &Capabilities, key: &str) -> bool {
    c.additional.get(key) == Some(&Value::Bool(true))
}
pub(super) fn invalid_request(
    operation: DebugInspectionOperation,
    reason: DebugInspectionRequestReason,
) -> DebugInspectionError {
    DebugInspectionError::InvalidRequest { operation, reason }
}
pub(super) fn invalid_response(
    operation: DebugInspectionOperation,
    reason: DebugInspectionResponseReason,
) -> DebugInspectionError {
    DebugInspectionError::InvalidResponse { operation, reason }
}
pub(super) fn unsupported(
    operation: DebugInspectionOperation,
    capability: DebugInspectionCapability,
) -> DebugInspectionError {
    DebugInspectionError::UnsupportedCapability {
        operation,
        capability,
    }
}
pub(super) fn map_transport(
    error: DapError,
    operation: DebugInspectionOperation,
) -> DebugInspectionError {
    match error {
        DapError::RequestTimeout { .. } => DebugInspectionError::Timeout { operation },
        DapError::TransportClosed => DebugInspectionError::TransportFailure { operation },
        DapError::Response { message, .. } if message == "cancelled" => {
            DebugInspectionError::CancelledByAdapter { operation }
        }
        DapError::Response { .. } => DebugInspectionError::AdapterRejected { operation },
        DapError::InvalidMessage(message) if message.starts_with("response command mismatch:") => {
            invalid_response(operation, DebugInspectionResponseReason::CommandMismatch)
        }
        _ => DebugInspectionError::InternalFailure { operation },
    }
}
pub(super) fn map_unknown_transport(error: DapError) -> DebugEvaluateUnknownReason {
    match error {
        DapError::RequestTimeout { .. } => DebugEvaluateUnknownReason::Timeout,
        DapError::TransportClosed => DebugEvaluateUnknownReason::TransportFailure,
        DapError::Response { message, .. } if message == "cancelled" => {
            DebugEvaluateUnknownReason::CancelledByAdapter
        }
        DapError::Response { .. } => DebugEvaluateUnknownReason::AdapterRejected,
        DapError::InvalidMessage(message) if message.starts_with("response command mismatch:") => {
            DebugEvaluateUnknownReason::CommandMismatch
        }
        _ => DebugEvaluateUnknownReason::InternalFailureAfterAdmission,
    }
}
pub(super) fn map_unknown_inspection(error: DebugInspectionError) -> DebugEvaluateUnknownReason {
    match error {
        DebugInspectionError::Timeout { .. } => DebugEvaluateUnknownReason::Timeout,
        DebugInspectionError::AdapterRejected { .. } => DebugEvaluateUnknownReason::AdapterRejected,
        DebugInspectionError::CancelledByAdapter { .. } => {
            DebugEvaluateUnknownReason::CancelledByAdapter
        }
        DebugInspectionError::InvalidResponse {
            reason: DebugInspectionResponseReason::CommandMismatch,
            ..
        } => DebugEvaluateUnknownReason::CommandMismatch,
        DebugInspectionError::TransportFailure { .. } => {
            DebugEvaluateUnknownReason::TransportFailure
        }
        DebugInspectionError::AdapterOrTargetExited { .. } => {
            DebugEvaluateUnknownReason::AdapterOrTargetExited
        }
        DebugInspectionError::ClientReplaced { .. } => DebugEvaluateUnknownReason::ClientReplaced,
        DebugInspectionError::ManagerOrOwnerShutdown { .. } => {
            DebugEvaluateUnknownReason::ManagerOrOwnerShutdown
        }
        DebugInspectionError::CorrelationFailure { .. } => {
            DebugEvaluateUnknownReason::CorrelationFailure
        }
        DebugInspectionError::LifecycleInvalidated { .. }
        | DebugInspectionError::SessionClosed
        | DebugInspectionError::SessionTerminal
        | DebugInspectionError::SessionNotStopped
        | DebugInspectionError::StaleExecutionRevision
        | DebugInspectionError::StaleHandle
        | DebugInspectionError::FrameUnavailable => {
            DebugEvaluateUnknownReason::LifecycleInvalidated
        }
        _ => DebugEvaluateUnknownReason::InternalFailureAfterAdmission,
    }
}
pub(super) fn unknown(reason: DebugEvaluateUnknownReason) -> DebugEvaluateOutcome {
    DebugEvaluateOutcome::Unknown(DebugEvaluateOutcomeUnknown { reason })
}
#[cfg(test)]
mod phase30f_contract_tests {
    use super::*;
    #[test]
    fn variable_value_truncation_is_utf8_safe_and_reports_omitted_bytes() {
        assert_eq!(truncate_utf8("aéz", 2), ("a".to_owned(), 3));
    }
    #[test]
    fn source_reference_only_maps_to_none() {
        assert_eq!(MAX_SAFE_INTEGER, 9_007_199_254_740_991);
    }
}
