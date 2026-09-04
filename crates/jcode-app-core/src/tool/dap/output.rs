use jcode_dap::{
    DebugControlOperation, DebugEvaluateUnknownReason, DebugOutputCategory, DebugSessionStateKind,
};

pub(super) fn session_state(v: DebugSessionStateKind) -> &'static str {
    match v {
        DebugSessionStateKind::Reserved => "reserved",
        DebugSessionStateKind::Initializing => "initializing",
        DebugSessionStateKind::Configuring => "configuring",
        DebugSessionStateKind::Running => "running",
        DebugSessionStateKind::Stopped => "stopped",
        DebugSessionStateKind::Terminating => "terminating",
        DebugSessionStateKind::Ended => "ended",
    }
}
pub(super) fn control_operation(v: DebugControlOperation) -> &'static str {
    match v {
        DebugControlOperation::Continue => "continue",
        DebugControlOperation::Pause => "pause",
        DebugControlOperation::StepOver => "step_over",
        DebugControlOperation::StepIn => "step_in",
        DebugControlOperation::StepOut => "step_out",
    }
}
pub(super) fn output_category(v: DebugOutputCategory) -> &'static str {
    match v {
        DebugOutputCategory::Console => "console",
        DebugOutputCategory::Important => "important",
        DebugOutputCategory::Stdout => "stdout",
        DebugOutputCategory::Stderr => "stderr",
        DebugOutputCategory::Telemetry => "telemetry",
        DebugOutputCategory::Other => "other",
    }
}
pub(super) fn evaluate_unknown_reason(v: DebugEvaluateUnknownReason) -> &'static str {
    match v {
        DebugEvaluateUnknownReason::Timeout => "timeout",
        DebugEvaluateUnknownReason::MalformedSuccess => "malformed_success",
        DebugEvaluateUnknownReason::OversizedSuccess => "oversized_success",
        DebugEvaluateUnknownReason::AdapterRejected => "adapter_rejected",
        DebugEvaluateUnknownReason::CancelledByAdapter => "cancelled_by_adapter",
        DebugEvaluateUnknownReason::CommandMismatch => "command_mismatch",
        DebugEvaluateUnknownReason::TransportFailure => "transport_failure",
        DebugEvaluateUnknownReason::AdapterOrTargetExited => "adapter_or_target_exited",
        DebugEvaluateUnknownReason::LifecycleInvalidated => "lifecycle_invalidated",
        DebugEvaluateUnknownReason::ClientReplaced => "client_replaced",
        DebugEvaluateUnknownReason::ManagerOrOwnerShutdown => "manager_or_owner_shutdown",
        DebugEvaluateUnknownReason::CorrelationFailure => "correlation_failure",
        DebugEvaluateUnknownReason::InternalFailureAfterAdmission => {
            "internal_failure_after_admission"
        }
        _ => "unknown",
    }
}
pub(super) fn execution_class(action: &str) -> &'static str {
    match action {
        "launch" | "attach" | "evaluate" => "exec",
        "set_breakpoint" | "remove_breakpoint" | "continue" | "pause" | "step_over" | "step_in"
        | "step_out" | "terminate" => "mutating",
        _ => "read",
    }
}
