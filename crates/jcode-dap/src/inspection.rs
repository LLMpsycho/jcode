use std::fmt;
use thiserror::Error;

use crate::{DebugExecutionRevision, DebugSessionId, DebugThreadId};

#[derive(Clone, Debug)]
pub struct DebugInspectionConfig {
    pub max_stack_frames_per_response: usize,
    pub max_frame_handles_per_execution_revision: usize,
    pub max_scopes_per_response: usize,
    pub max_variables_per_response: usize,
    pub max_name_bytes: usize,
    pub max_type_bytes: usize,
    pub max_variable_value_bytes: usize,
    pub max_evaluate_expression_bytes: usize,
    pub max_evaluate_result_bytes: usize,
    pub max_source_name_bytes: usize,
    pub max_source_path_bytes: usize,
    pub max_presentation_string_bytes: usize,
    pub max_presentation_attributes: usize,
    pub max_variable_evaluate_name_bytes: usize,
    pub max_confidential_adapter_message_bytes: usize,
    pub max_public_text_bytes_per_result: usize,
}

impl Default for DebugInspectionConfig {
    fn default() -> Self {
        Self {
            max_stack_frames_per_response: 256,
            max_frame_handles_per_execution_revision: 4096,
            max_scopes_per_response: 128,
            max_variables_per_response: 512,
            max_name_bytes: 4 * 1024,
            max_type_bytes: 4 * 1024,
            max_variable_value_bytes: 64 * 1024,
            max_evaluate_expression_bytes: 64 * 1024,
            max_evaluate_result_bytes: 64 * 1024,
            max_source_name_bytes: 4 * 1024,
            max_source_path_bytes: 16 * 1024,
            max_presentation_string_bytes: 4 * 1024,
            max_presentation_attributes: 32,
            max_variable_evaluate_name_bytes: 64 * 1024,
            max_confidential_adapter_message_bytes: 4 * 1024,
            max_public_text_bytes_per_result: 4 * 1024 * 1024,
        }
    }
}

impl DebugInspectionConfig {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        let invalid = || crate::DapError::InvalidManagerConfiguration {
            message: "invalid debug inspection configuration".to_owned(),
        };
        let positive = [
            self.max_stack_frames_per_response,
            self.max_frame_handles_per_execution_revision,
            self.max_scopes_per_response,
            self.max_variables_per_response,
            self.max_name_bytes,
            self.max_type_bytes,
            self.max_variable_value_bytes,
            self.max_evaluate_expression_bytes,
            self.max_evaluate_result_bytes,
            self.max_source_name_bytes,
            self.max_source_path_bytes,
            self.max_presentation_string_bytes,
            self.max_presentation_attributes,
            self.max_variable_evaluate_name_bytes,
            self.max_confidential_adapter_message_bytes,
            self.max_public_text_bytes_per_result,
        ];
        if positive.contains(&0) {
            return Err(invalid());
        }

        let max_u32 = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
        if self.max_stack_frames_per_response > max_u32 || self.max_variables_per_response > max_u32
        {
            return Err(invalid());
        }

        let returned_text_atoms = [
            self.max_name_bytes,
            self.max_type_bytes,
            self.max_variable_value_bytes,
            self.max_evaluate_result_bytes,
            self.max_source_name_bytes,
            self.max_source_path_bytes,
            self.max_presentation_string_bytes,
            self.max_variable_evaluate_name_bytes,
        ];
        if returned_text_atoms
            .iter()
            .any(|maximum| *maximum > self.max_public_text_bytes_per_result)
        {
            return Err(invalid());
        }

        let presentation_bytes = self
            .max_presentation_attributes
            .checked_mul(self.max_presentation_string_bytes)
            .ok_or_else(&invalid)?;
        if presentation_bytes > self.max_public_text_bytes_per_result {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct DebugStackFrameHandle {
    pub(crate) manager_id: u64,
    pub(crate) session_id: DebugSessionId,
    pub(crate) execution_revision: DebugExecutionRevision,
    pub(crate) thread_id: i32,
    pub(crate) frame_id: i32,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct DebugStepInTargetHandle {
    pub(crate) manager_id: u64,
    pub(crate) session_id: DebugSessionId,
    pub(crate) execution_revision: DebugExecutionRevision,
    pub(crate) thread_id: i32,
    pub(crate) frame_id: i32,
    pub(crate) target_id: i32,
}

impl fmt::Debug for DebugStepInTargetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-step-in-target-handle>")
    }
}

impl fmt::Display for DebugStepInTargetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-step-in-target-handle>")
    }
}

impl fmt::Debug for DebugStackFrameHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-stack-frame-handle>")
    }
}
impl fmt::Display for DebugStackFrameHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-stack-frame-handle>")
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct DebugVariableHandle {
    pub(crate) manager_id: u64,
    pub(crate) session_id: DebugSessionId,
    pub(crate) execution_revision: DebugExecutionRevision,
    pub(crate) thread_id: Option<i32>,
    pub(crate) variables_reference: i32,
}

impl DebugVariableHandle {
    pub fn is_expandable(&self) -> bool {
        self.variables_reference != 0
    }
}
impl fmt::Debug for DebugVariableHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-variable-handle>")
    }
}
impl fmt::Display for DebugVariableHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<debug-variable-handle>")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStackTraceRequest {
    pub thread_id: Option<DebugThreadId>,
    pub start_frame: u32,
    pub levels: u32,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}

impl DebugStackTraceRequest {
    pub fn new(levels: u32) -> Self {
        Self {
            thread_id: None,
            start_frame: 0,
            levels,
            expected_execution_revision: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStackTraceResult {
    pub thread_id: DebugThreadId,
    pub execution_revision: DebugExecutionRevision,
    pub frames: Vec<DebugStackFrame>,
    pub total_frames: Option<u32>,
    pub next_start_frame: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStackFrame {
    pub handle: DebugStackFrameHandle,
    pub name: String,
    pub location: DebugStackFrameLocation,
    pub presentation_hint: Option<DebugStackFramePresentationHint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStackFrameLocation {
    pub source: Option<DebugSource>,
    pub line: u64,
    pub column: u64,
    pub end_line: Option<u64>,
    pub end_column: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStepInTargetsRequest {
    pub frame: DebugStackFrameHandle,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStepInTargetsResult {
    pub execution_revision: DebugExecutionRevision,
    pub targets: Vec<DebugStepInTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugStepInTarget {
    pub handle: DebugStepInTargetHandle,
    pub label: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub end_line: Option<u64>,
    pub end_column: Option<u64>,
    pub instruction_pointer_reference: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugStackFramePresentationHint {
    Normal,
    Label,
    Subtle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugScopesRequest {
    pub frame: DebugStackFrameHandle,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugScopesResult {
    pub execution_revision: DebugExecutionRevision,
    pub scopes: Vec<DebugScope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugScope {
    pub name: String,
    pub presentation_hint: Option<DebugScopePresentationHint>,
    pub variables: DebugVariableHandle,
    pub named_variables: Option<u32>,
    pub indexed_variables: Option<u32>,
    pub expensive: bool,
    pub location: DebugScopeLocation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugScopeLocation {
    pub source: Option<DebugSource>,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub end_line: Option<u64>,
    pub end_column: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugScopePresentationHint {
    Arguments,
    Locals,
    Registers,
    ReturnValue,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugVariablesRequest {
    pub variables: DebugVariableHandle,
    pub filter: Option<DebugVariableFilter>,
    pub start: u32,
    pub count: u32,
    pub expected_execution_revision: Option<DebugExecutionRevision>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugVariableFilter {
    Named,
    Indexed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugVariablesResult {
    pub execution_revision: DebugExecutionRevision,
    pub variables: Vec<DebugVariable>,
    pub next_start: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugVariable {
    pub name: String,
    pub value: DebugVariableValue,
    pub type_name: Option<String>,
    pub presentation_hint: Option<DebugVariablePresentationHint>,
    pub evaluate_name: Option<String>,
    pub variables: DebugVariableHandle,
    pub named_variables: Option<u32>,
    pub indexed_variables: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugVariableValue {
    pub text: String,
    pub omitted_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugVariablePresentationHint {
    pub kind: Option<DebugVariableKind>,
    pub attributes: Vec<DebugVariableAttribute>,
    pub visibility: Option<DebugVariableVisibility>,
    pub lazy: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugVariableKind {
    Property,
    Method,
    Class,
    Data,
    Event,
    BaseClass,
    InnerClass,
    Interface,
    MostDerivedClass,
    Virtual,
    DataBreakpoint,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugVariableAttribute {
    Static,
    Constant,
    ReadOnly,
    RawString,
    HasObjectId,
    CanHaveObjectId,
    HasSideEffects,
    HasDataBreakpoint,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugVariableVisibility {
    Public,
    Private,
    Protected,
    Internal,
    Final,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugSource {
    pub name: Option<String>,
    pub path: Option<String>,
    pub presentation_hint: Option<DebugSourcePresentationHint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugSourcePresentationHint {
    Normal,
    Emphasize,
    Deemphasize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugEvaluateRequest {
    pub expression: String,
    pub context: DebugEvaluateContext,
    pub target: DebugEvaluateTarget,
    pub expected_execution_revision: DebugExecutionRevision,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugEvaluateContext {
    #[default]
    Unspecified,
    Watch,
    Repl,
    Hover,
    Clipboard,
    Variables,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugEvaluateTarget {
    Global,
    Frame(DebugStackFrameHandle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugEvaluateRisk {
    PotentiallyMutating,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugEvaluateOutcome {
    Known(DebugEvaluateResult),
    Unknown(DebugEvaluateOutcomeUnknown),
}

impl DebugEvaluateOutcome {
    pub const fn risk(&self) -> DebugEvaluateRisk {
        DebugEvaluateRisk::PotentiallyMutating
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugEvaluateResult {
    pub execution_revision: DebugExecutionRevision,
    pub result: String,
    pub type_name: Option<String>,
    pub presentation_hint: Option<DebugVariablePresentationHint>,
    pub variables: DebugVariableHandle,
    pub named_variables: Option<u32>,
    pub indexed_variables: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugEvaluateOutcomeUnknown {
    pub reason: DebugEvaluateUnknownReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugEvaluateUnknownReason {
    Timeout,
    MalformedSuccess,
    OversizedSuccess,
    AdapterRejected,
    CancelledByAdapter,
    CommandMismatch,
    TransportFailure,
    AdapterOrTargetExited,
    LifecycleInvalidated,
    ClientReplaced,
    ManagerOrOwnerShutdown,
    CorrelationFailure,
    InternalFailureAfterAdmission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionOperation {
    StackTrace,
    StepInTargets,
    Scopes,
    Variables,
    Evaluate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionCapability {
    DelayedStackTraceLoading,
    StepInTargets,
    EvaluateForHovers,
    ClipboardContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionRequestReason {
    ZeroPageSize,
    PageSizeExceedsConfiguredMaximum,
    PageOffsetOverflow,
    EmptyExpression,
    ExpressionTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionLimit {
    StackFrames,
    StepInTargets,
    FrameHandlesPerExecutionRevision,
    Scopes,
    Variables,
    NameBytes,
    TypeBytes,
    VariableValueBytes,
    EvaluateResultBytes,
    SourceNameBytes,
    SourcePathBytes,
    PresentationStringBytes,
    PresentationAttributes,
    VariableEvaluateNameBytes,
    AggregatePublicTextBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionResponseReason {
    MissingBody,
    MissingRequiredField,
    InvalidInteger,
    InvalidPresentationHint,
    DuplicateFrameId,
    DuplicateTargetId,
    ReturnedMoreThanRequested,
    TooManyItems,
    AggregateTextTooLarge,
    CommandMismatch,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum DebugInspectionError {
    #[error("debug session was not found")]
    SessionNotFound,
    #[error("access to debug session is denied")]
    SessionAccessDenied,
    #[error("debug session is closed")]
    SessionClosed,
    #[error("debug session is terminal")]
    SessionTerminal,
    #[error("debug session is not stopped")]
    SessionNotStopped,
    #[error("the expected execution revision is stale")]
    StaleExecutionRevision,
    #[error("no stopped thread is available")]
    StoppedThreadUnavailable,
    #[error("stopped thread selection is ambiguous")]
    AmbiguousStoppedThread,
    #[error("the selected stopped thread was not found")]
    ThreadNotFound,
    #[error("the stack frame is unavailable")]
    FrameUnavailable,
    #[error("the handle was issued by another manager")]
    HandleManagerMismatch,
    #[error("the handle was issued for another debug session")]
    HandleSessionMismatch,
    #[error("the handle is stale")]
    StaleHandle,
    #[error("the variable handle is a non-expandable leaf")]
    VariableNotExpandable,
    #[error("{operation:?} requires unsupported capability {capability:?}")]
    UnsupportedCapability {
        operation: DebugInspectionOperation,
        capability: DebugInspectionCapability,
    },
    #[error("invalid {operation:?} request: {reason:?}")]
    InvalidRequest {
        operation: DebugInspectionOperation,
        reason: DebugInspectionRequestReason,
    },
    #[error("{operation:?} exceeded the configured {limit:?} limit")]
    LimitExceeded {
        operation: DebugInspectionOperation,
        limit: DebugInspectionLimit,
    },
    #[error("invalid {operation:?} response: {reason:?}")]
    InvalidResponse {
        operation: DebugInspectionOperation,
        reason: DebugInspectionResponseReason,
    },
    #[error("the adapter rejected {operation:?}")]
    AdapterRejected { operation: DebugInspectionOperation },
    #[error("{operation:?} timed out")]
    Timeout { operation: DebugInspectionOperation },
    #[error("{operation:?} was cancelled")]
    CancelledByAdapter { operation: DebugInspectionOperation },
    #[error("the transport failed during {operation:?}")]
    TransportFailure { operation: DebugInspectionOperation },
    #[error("the adapter or target exited during {operation:?}")]
    AdapterOrTargetExited { operation: DebugInspectionOperation },
    #[error("the debug session changed during {operation:?}")]
    LifecycleInvalidated { operation: DebugInspectionOperation },
    #[error("the debug adapter client changed during {operation:?}")]
    ClientReplaced { operation: DebugInspectionOperation },
    #[error("the manager or owner shut down during {operation:?}")]
    ManagerOrOwnerShutdown { operation: DebugInspectionOperation },
    #[error("response correlation failed during {operation:?}")]
    CorrelationFailure { operation: DebugInspectionOperation },
    #[error("an internal inspection failure occurred during {operation:?}")]
    InternalFailure { operation: DebugInspectionOperation },
}

#[cfg(test)]
mod phase30f_contract_tests {
    use super::*;

    #[test]
    fn inspection_config_defaults_match_binding_contract() {
        let config = DebugInspectionConfig::default();
        assert_eq!(config.max_stack_frames_per_response, 256);
        assert_eq!(config.max_frame_handles_per_execution_revision, 4096);
        assert_eq!(config.max_scopes_per_response, 128);
        assert_eq!(config.max_variables_per_response, 512);
        assert_eq!(config.max_variable_value_bytes, 64 * 1024);
        assert_eq!(config.max_public_text_bytes_per_result, 4 * 1024 * 1024);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn inspection_config_rejects_each_zero_positive_limit() {
        let mut configs = Vec::new();
        macro_rules! zero {
            ($field:ident) => {{
                let mut config = DebugInspectionConfig::default();
                config.$field = 0;
                configs.push(config);
            }};
        }
        zero!(max_stack_frames_per_response);
        zero!(max_frame_handles_per_execution_revision);
        zero!(max_scopes_per_response);
        zero!(max_variables_per_response);
        zero!(max_name_bytes);
        zero!(max_type_bytes);
        zero!(max_variable_value_bytes);
        zero!(max_evaluate_expression_bytes);
        zero!(max_evaluate_result_bytes);
        zero!(max_source_name_bytes);
        zero!(max_source_path_bytes);
        zero!(max_presentation_string_bytes);
        zero!(max_presentation_attributes);
        zero!(max_variable_evaluate_name_bytes);
        zero!(max_confidential_adapter_message_bytes);
        zero!(max_public_text_bytes_per_result);
        assert!(configs.into_iter().all(|config| config.validate().is_err()));
    }

    #[test]
    fn inspection_config_rejects_invalid_limit_relationships() {
        let atom = DebugInspectionConfig {
            max_name_bytes: DebugInspectionConfig::default().max_public_text_bytes_per_result + 1,
            ..DebugInspectionConfig::default()
        };
        assert!(atom.validate().is_err());

        let presentation = DebugInspectionConfig {
            max_presentation_attributes: usize::MAX,
            ..DebugInspectionConfig::default()
        };
        assert!(presentation.validate().is_err());
    }

    #[test]
    fn inspection_config_checked_arithmetic_never_wraps() {
        let config = DebugInspectionConfig {
            max_presentation_attributes: usize::MAX,
            max_presentation_string_bytes: 2,
            max_public_text_bytes_per_result: usize::MAX,
            ..DebugInspectionConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn evaluate_is_always_potentially_mutating() {
        let outcome = DebugEvaluateOutcome::Unknown(DebugEvaluateOutcomeUnknown {
            reason: DebugEvaluateUnknownReason::Timeout,
        });
        assert_eq!(outcome.risk(), DebugEvaluateRisk::PotentiallyMutating);
    }
}
