use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

#[derive(Clone)]
pub(crate) struct ClientInstance(Arc<()>);

impl ClientInstance {
    pub(crate) fn new() -> Self {
        Self(Arc::new(()))
    }

    pub(crate) fn is_exact(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum AdmissionPhase {
    PreAdmission,
    PendingInstalled,
    Admitted,
    Settled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Admission {
    PreAdmission,
    PostAdmission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettlementReason {
    CallerDrop,
    Deadline,
    Invalidated,
    QueueFailure,
    TransportFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutcomeState {
    Pending,
    Response,
    NotDispatched(SettlementReason),
    Failed(SettlementReason),
    AbandonedCaller { admission: Admission },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum CancelState {
    EligibleNotQueued,
    Queued,
    Ineligible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CorrelationState {
    PreAdmission,
    AwaitingResponse {
        request_seq: i64,
        cancel_state: CancelState,
    },
    ResponseRouted,
    SettledWithoutResponse(SettlementReason),
    Settled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransactionSnapshot {
    pub outcome: OutcomeState,
    pub correlation: CorrelationState,
    pub admission_phase: AdmissionPhase,
}

struct TransactionState {
    snapshot: TransactionSnapshot,
    pending_seq: Option<i64>,
    cancel_eligible: bool,
}

pub(crate) struct RequestTransaction {
    instance: ClientInstance,
    state: Mutex<TransactionState>,
    changed: watch::Sender<TransactionSnapshot>,
}

impl RequestTransaction {
    pub(crate) fn new(instance: ClientInstance, cancel_eligible: bool) -> Arc<Self> {
        let snapshot = TransactionSnapshot {
            outcome: OutcomeState::Pending,
            correlation: CorrelationState::PreAdmission,
            admission_phase: AdmissionPhase::PreAdmission,
        };
        let (changed, _) = watch::channel(snapshot);
        Arc::new(Self {
            instance,
            state: Mutex::new(TransactionState {
                snapshot,
                pending_seq: None,
                cancel_eligible,
            }),
            changed,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn observer(self: &Arc<Self>) -> AdmissionObserver {
        AdmissionObserver {
            transaction: Arc::clone(self),
            changed: self.changed.subscribe(),
        }
    }

    pub(crate) fn instance(&self) -> &ClientInstance {
        &self.instance
    }

    #[allow(dead_code)]
    pub(crate) fn snapshot(&self) -> TransactionSnapshot {
        lock(&self.state).snapshot
    }

    pub(crate) fn commit_admission(&self, request_seq: i64, commit: impl FnOnce() -> bool) -> bool {
        let mut state = lock(&self.state);
        if state.snapshot.outcome != OutcomeState::Pending
            || state.snapshot.correlation != CorrelationState::PreAdmission
            || state.snapshot.admission_phase != AdmissionPhase::PreAdmission
        {
            return false;
        }
        state.pending_seq = Some(request_seq);
        state.snapshot.admission_phase = AdmissionPhase::PendingInstalled;
        self.publish(state.snapshot);
        if !commit() {
            return false;
        }
        state.snapshot.correlation = CorrelationState::AwaitingResponse {
            request_seq,
            cancel_state: if state.cancel_eligible {
                CancelState::EligibleNotQueued
            } else {
                CancelState::Ineligible
            },
        };
        state.snapshot.admission_phase = AdmissionPhase::Admitted;
        self.publish(state.snapshot);
        true
    }

    pub(crate) fn route_response(
        &self,
        instance: &ClientInstance,
        request_seq: i64,
        claim: impl FnOnce() -> bool,
    ) -> bool {
        if !self.instance.is_exact(instance) {
            return false;
        }
        let mut state = lock(&self.state);
        if state.snapshot.outcome != OutcomeState::Pending {
            return false;
        }
        let CorrelationState::AwaitingResponse {
            request_seq: expected,
            ..
        } = state.snapshot.correlation
        else {
            return false;
        };
        if expected != request_seq {
            return false;
        }
        if !claim() {
            return false;
        }
        state.snapshot.correlation = CorrelationState::ResponseRouted;
        self.publish(state.snapshot);
        true
    }

    pub(crate) fn settle_response(&self) -> bool {
        let mut state = lock(&self.state);
        if state.snapshot.outcome != OutcomeState::Pending
            || state.snapshot.correlation != CorrelationState::ResponseRouted
        {
            return false;
        }
        state.snapshot.outcome = OutcomeState::Response;
        state.snapshot.correlation = CorrelationState::Settled;
        state.snapshot.admission_phase = AdmissionPhase::Settled;
        state.pending_seq = None;
        self.publish(state.snapshot);
        true
    }

    #[allow(dead_code)]
    pub(crate) fn settle_caller_drop(&self) -> Settlement {
        self.settle_without_response(SettlementReason::CallerDrop, |_, _| {})
    }

    #[allow(dead_code)]
    pub(crate) fn settle_deadline(&self) -> Settlement {
        self.settle_without_response(SettlementReason::Deadline, |_, _| {})
    }

    pub(crate) fn settle_transport_failure(&self) -> Settlement {
        self.settle_without_response(SettlementReason::TransportFailure, |_, _| {})
    }

    pub(crate) fn settle_with(
        &self,
        reason: SettlementReason,
        cleanup: impl FnOnce(Option<i64>, Option<i64>),
    ) -> Settlement {
        self.settle_without_response(reason, cleanup)
    }

    fn settle_without_response(
        &self,
        reason: SettlementReason,
        cleanup: impl FnOnce(Option<i64>, Option<i64>),
    ) -> Settlement {
        let mut state = lock(&self.state);
        if state.snapshot.outcome != OutcomeState::Pending
            || state.snapshot.correlation == CorrelationState::ResponseRouted
            || state.snapshot.correlation == CorrelationState::Settled
            || matches!(
                state.snapshot.correlation,
                CorrelationState::SettledWithoutResponse(_)
            )
        {
            return Settlement::lost();
        }
        let admission = if state.snapshot.admission_phase == AdmissionPhase::Admitted {
            Admission::PostAdmission
        } else {
            Admission::PreAdmission
        };
        let cancel_request_seq = match state.snapshot.correlation {
            CorrelationState::AwaitingResponse {
                request_seq,
                cancel_state: CancelState::EligibleNotQueued,
            } => Some(request_seq),
            _ => None,
        };
        cleanup(state.pending_seq, cancel_request_seq);
        state.snapshot.outcome = if reason == SettlementReason::CallerDrop {
            OutcomeState::AbandonedCaller { admission }
        } else if admission == Admission::PreAdmission {
            OutcomeState::NotDispatched(reason)
        } else {
            OutcomeState::Failed(reason)
        };
        state.snapshot.correlation = CorrelationState::SettledWithoutResponse(reason);
        state.snapshot.admission_phase = AdmissionPhase::Settled;
        state.pending_seq = None;
        self.publish(state.snapshot);
        Settlement { won: true }
    }

    fn publish(&self, snapshot: TransactionSnapshot) {
        self.changed.send_replace(snapshot);
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AdmissionObserver {
    transaction: Arc<RequestTransaction>,
    changed: watch::Receiver<TransactionSnapshot>,
}

impl AdmissionObserver {
    #[allow(dead_code)]
    pub(crate) fn snapshot(&self) -> TransactionSnapshot {
        *self.changed.borrow()
    }

    #[allow(dead_code)]
    pub(crate) fn is_admitted(&self) -> bool {
        let snapshot = self.snapshot();
        match snapshot.outcome {
            OutcomeState::NotDispatched(_)
            | OutcomeState::AbandonedCaller {
                admission: Admission::PreAdmission,
            } => false,
            _ => snapshot.admission_phase >= AdmissionPhase::Admitted,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_exact_client(&self, instance: &ClientInstance) -> bool {
        self.transaction.instance.is_exact(instance)
    }
}

pub(crate) struct Settlement {
    pub(crate) won: bool,
}

impl Settlement {
    fn lost() -> Self {
        Self { won: false }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
