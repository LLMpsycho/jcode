use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Semaphore, broadcast, oneshot, watch};
use tokio::task::JoinHandle;

use crate::client::DapClientStatus;
use crate::launch::{AdapterProfile, ResolvedLaunch, resolve_program, revalidate_program};
use crate::process::{OwnedChildObserver, OwnedTargetProcess};
use crate::session::{OutputRing, SessionEvent, parse_event};
mod initialization;
mod inspection_runtime;
#[cfg(test)]
mod inspection_tests;
#[cfg(test)]
mod lifecycle_tests;
mod source_hash;
mod supervision;

use supervision::{invalid_transition, lock, notify, supervise, transition};

use crate::{
    AdapterCommand, AdapterProcess, Capabilities, DapClient, DapError, DebugAdapterConfig,
    DebugCleanupFailure, DebugCleanupReport, DebugInspectionConfig, DebugLaunchRequest,
    DebugOperationConfig, DebugOutputCursor, DebugOutputPage, DebugOwnedAttachRequest,
    DebugSessionEnd, DebugSessionEndReason, DebugSessionId, DebugSessionManagerConfig,
    DebugSessionSnapshot, DebugSessionStart, DebugSessionState, DebugSessionStateKind,
    DebugStartOperation, DebugStartupPhase, DebugWorkspaceKey, OwnerCleanupCause, ProcessStatus,
    Response, Result,
};

#[derive(Clone)]
pub struct DebugSessionManager {
    core: Arc<ManagerCore>,
}

trait ManagerInitialization: Sized {
    fn initialize(
        config: DebugSessionManagerConfig,
        operations: DebugOperationConfig,
        inspection: DebugInspectionConfig,
    ) -> Result<Self>;
}

impl DebugSessionManager {
    pub fn new(config: DebugSessionManagerConfig) -> Result<Self> {
        Self::new_with_operation_config(config, DebugOperationConfig::default())
    }

    pub fn new_with_operation_config(
        config: DebugSessionManagerConfig,
        operations: DebugOperationConfig,
    ) -> Result<Self> {
        Self::new_with_operation_and_inspection_config(
            config,
            operations,
            DebugInspectionConfig::default(),
        )
    }

    pub fn new_with_operation_and_inspection_config(
        _manager: DebugSessionManagerConfig,
        _operations: DebugOperationConfig,
        _inspection: DebugInspectionConfig,
    ) -> Result<Self> {
        Self::initialize(_manager, _operations, _inspection)
    }

    pub fn sessions(&self, owner_session_id: &str) -> Vec<DebugSessionSnapshot> {
        let entries = {
            let registry = lock(&self.core.registry);
            registry
                .ids_by_owner
                .get(owner_session_id)
                .into_iter()
                .flatten()
                .filter_map(|id| registry.entries.get(id).cloned())
                .collect::<Vec<_>>()
        };
        entries
            .iter()
            .filter_map(|entry| entry.snapshot())
            .collect()
    }

    pub fn snapshot(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
    ) -> Result<DebugSessionSnapshot> {
        self.core
            .authorized_entry(owner_session_id, id)?
            .snapshot()
            .ok_or(DapError::SessionNotFound { session_id: id })
    }

    pub fn output(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
        after: Option<DebugOutputCursor>,
        limit: usize,
    ) -> Result<DebugOutputPage> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        let data = lock(&entry.data);
        Ok(data
            .output
            .page(after, limit.min(self.core.config.output_page_limit)))
    }

    pub async fn terminate(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
    ) -> Result<DebugSessionSnapshot> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        self.core
            .finalize(entry.clone(), DebugSessionEndReason::Requested, true, true)
            .await?;
        entry
            .snapshot()
            .ok_or(DapError::SessionNotFound { session_id: id })
    }

    pub async fn cleanup_owner(
        &self,
        owner_session_id: &str,
        cause: OwnerCleanupCause,
    ) -> DebugCleanupReport {
        let entries = self.core.drain_owner(owner_session_id);
        let reason = match cause {
            OwnerCleanupCause::Disconnected => DebugSessionEndReason::OwnerDisconnected,
            OwnerCleanupCause::Expired => DebugSessionEndReason::OwnerExpired,
        };
        self.core.finalize_many(entries, reason).await
    }

    pub async fn shutdown_all(&self) -> DebugCleanupReport {
        let entries = self.core.drain_all();
        self.core
            .finalize_many(entries, DebugSessionEndReason::ServerShutdown)
            .await
    }

    pub async fn launch(
        &self,
        owner_session_id: &str,
        workspace: DebugWorkspaceKey,
        adapter: &DebugAdapterConfig,
        request: DebugLaunchRequest,
    ) -> Result<DebugSessionSnapshot> {
        deny_unsupported(DebugStartOperation::Launch)?;
        adapter.revalidate()?;
        let target = resolve_program(&workspace, request.program(), request.args(), request.cwd())?;
        let start = DebugSessionStart::Launch {
            program: target.program.clone(),
            cwd: target.cwd.clone(),
        };
        let resolved = ResolvedLaunch {
            target,
            stop_on_entry: request.stop_on_entry(),
        };
        let mut reservation = self.reserve(NewDebugSession {
            owner_session_id: owner_session_id.to_owned(),
            workspace: workspace.clone(),
            adapter_id: adapter.adapter_id().to_owned(),
            start: Some(start),
        })?;
        let deadline = startup_deadline(self.core.config.startup_timeout)?;
        let profile = AdapterProfile::from_kind(adapter.kind());
        adapter.revalidate()?;
        let mut command = AdapterCommand::new(adapter.executable(), workspace.canonical_root());
        for argument in profile.command_arguments() {
            command = command.with_arg(*argument);
        }
        let process = deadline_result(
            deadline,
            DebugStartupPhase::Initialize,
            AdapterProcess::spawn(&command),
        )
        .await?;
        reservation.attach_process(process, DebugTerminationPolicy::AdapterLaunched)?;
        let result = async {
            initialize(&reservation, profile, deadline).await?;
            revalidate_program(&workspace, &resolved.target)?;
            start_after_initialize(
                &reservation,
                "launch",
                profile.launch_arguments(&resolved),
                deadline,
            )
            .await
        }
        .await;
        finish_start(reservation, result).await
    }

    pub async fn spawn_and_attach(
        &self,
        owner_session_id: &str,
        workspace: DebugWorkspaceKey,
        adapter: &DebugAdapterConfig,
        request: DebugOwnedAttachRequest,
    ) -> Result<DebugSessionSnapshot> {
        deny_unsupported(DebugStartOperation::OwnedAttach)?;
        adapter.revalidate()?;
        let target_spec =
            resolve_program(&workspace, request.program(), request.args(), request.cwd())?;
        let mut reservation = self.reserve(NewDebugSession {
            owner_session_id: owner_session_id.to_owned(),
            workspace: workspace.clone(),
            adapter_id: adapter.adapter_id().to_owned(),
            start: None,
        })?;
        let deadline = startup_deadline(self.core.config.startup_timeout)?;
        let profile = AdapterProfile::from_kind(adapter.kind());
        adapter.revalidate()?;
        let mut command = AdapterCommand::new(adapter.executable(), workspace.canonical_root());
        for argument in profile.command_arguments() {
            command = command.with_arg(*argument);
        }
        let process = deadline_result(
            deadline,
            DebugStartupPhase::Initialize,
            AdapterProcess::spawn(&command),
        )
        .await?;
        reservation.attach_process(process, DebugTerminationPolicy::OwnedAttach)?;
        if let Err(error) = initialize(&reservation, profile, deadline).await {
            return finish_start(reservation, Err(error)).await;
        }
        let adapter_pid = match reservation.live_adapter_pid().await {
            Ok(pid) => pid,
            Err(error) => return finish_start(reservation, Err(error)).await,
        };
        if let Err(error) = revalidate_program(&workspace, &target_spec) {
            return finish_start(reservation, Err(error)).await;
        }
        let target = match deadline_result(
            deadline,
            DebugStartupPhase::StartRequest,
            OwnedTargetProcess::spawn(&target_spec, Some(adapter_pid)),
        )
        .await
        {
            Ok(target) => target,
            Err(error) => return finish_start(reservation, Err(error)).await,
        };
        reservation.attach_target(target)?;
        match reservation.live_adapter_pid().await {
            Ok(pid) if pid == adapter_pid => {}
            Ok(_) => {
                return finish_start(
                    reservation,
                    Err(DapError::InvalidAdapterConfiguration {
                        message: "adapter process identity changed during startup".to_owned(),
                    }),
                )
                .await;
            }
            Err(error) => return finish_start(reservation, Err(error)).await,
        }
        if let ProcessStatus::Exited { code } = reservation.target_status().await? {
            return finish_start(
                reservation,
                Err(DapError::DebugTargetExitedBeforeAttach { exit_code: code }),
            )
            .await;
        }
        let pid = reservation
            .target_pid()
            .ok_or(DapError::DebugTargetExitedBeforeAttach { exit_code: None })?;
        reservation.set_start(DebugSessionStart::OwnedAttach {
            program: target_spec.program.clone(),
            cwd: target_spec.cwd.clone(),
            pid,
        })?;
        let result = {
            let start = start_after_initialize(
                &reservation,
                "attach",
                profile.attach_arguments(pid),
                deadline,
            );
            tokio::pin!(start);
            let target_exit = reservation.wait_for_target_exit(deadline);
            tokio::pin!(target_exit);
            tokio::select! {
                biased;
                exit = &mut target_exit => match exit {
                    Ok(code) => Err(DapError::DebugTargetExitedBeforeAttach { exit_code: code }),
                    Err(error) => Err(error),
                },
                result = &mut start => result,
            }
        };
        let result = match result {
            Ok(()) => match reservation.target_status().await {
                Ok(ProcessStatus::Running) => Ok(()),
                Ok(ProcessStatus::Exited { code }) => {
                    Err(DapError::DebugTargetExitedBeforeAttach { exit_code: code })
                }
                Err(error) => Err(error),
            },
            error => error,
        };
        finish_start(reservation, result).await
    }

    #[allow(dead_code)]
    pub(crate) fn reserve(&self, spec: NewDebugSession) -> Result<DebugSessionReservation> {
        if spec.owner_session_id.trim().is_empty() || spec.adapter_id.trim().is_empty() {
            return Err(DapError::InvalidMessage(
                "owner session and adapter identifiers must not be empty".to_owned(),
            ));
        }
        let mut registry = lock(&self.core.registry);
        if let Some(id) = registry.active_by_owner.get(&spec.owner_session_id) {
            return Err(DapError::OwnerAlreadyHasActiveSession {
                owner_session_id: spec.owner_session_id,
                session_id: *id,
            });
        }
        if registry.active_by_owner.len() >= self.core.config.max_active_sessions {
            return Err(DapError::SessionCapacityExceeded {
                limit: self.core.config.max_active_sessions,
            });
        }
        let sequence = registry.next_sequence;
        registry.next_sequence = sequence
            .checked_add(1)
            .ok_or(DapError::SessionIdExhausted)?;
        let id = DebugSessionId::new(self.core.manager_id, sequence);
        let (changed, _) = watch::channel(0);
        let owner = spec.owner_session_id;
        let entry = Arc::new(SessionEntry {
            id,
            owner_session_id: owner.clone(),
            workspace: spec.workspace,
            adapter_id: spec.adapter_id,
            data: Mutex::new(SessionData {
                state: DebugSessionState::Reserved,
                start: spec.start,
                initialized_seen: false,
                capabilities: Capabilities::default(),
                transport: None,
                output: OutputRing::new(
                    self.core.config.output_max_events,
                    self.core.config.output_max_bytes,
                ),
                supervisor: None,
                change_generation: 0,
                breakpoints: breakpoints::BreakpointRegistry::default(),
                execution_revision: 0,
                frame_identities: HashMap::new(),
                transport_revision: 0,
                inspection_invalidation: None,
            }),
            finalization: tokio::sync::Mutex::new(()),
            operation: tokio::sync::Mutex::new(()),
            source_hash: Arc::new(Semaphore::new(1)),
            breakpoint_validation: Arc::new(Semaphore::new(1)),
            inspection_parse: Arc::new(Semaphore::new(1)),
            publication: Mutex::new(()),
            active_inspections: Mutex::new(Vec::new()),
            #[cfg(test)]
            breakpoint_test_gates: breakpoints::BreakpointTestGates::default(),
            closed: AtomicBool::new(false),
            operations: Arc::clone(&self.core.operations),
            changed,
        });
        registry.entries.insert(id, entry);
        registry.active_by_owner.insert(owner.clone(), id);
        registry.ids_by_owner.entry(owner).or_default().insert(id);
        Ok(DebugSessionReservation {
            core: Arc::clone(&self.core),
            id,
            committed: false,
        })
    }

    #[allow(dead_code)]
    pub(crate) async fn request(
        &self,
        owner_session_id: &str,
        id: DebugSessionId,
        command: &str,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let entry = self.core.authorized_entry(owner_session_id, id)?;
        let client = {
            let data = lock(&entry.data);
            if data.state.is_terminal() || matches!(data.state, DebugSessionState::Terminating) {
                return Err(invalid_transition(&entry, &data, "request"));
            }
            data.transport
                .as_ref()
                .map(|transport| transport.client.clone())
                .ok_or_else(|| invalid_transition(&entry, &data, "request"))?
        };
        client.request(command, arguments, timeout).await
    }
}

#[allow(dead_code)]
pub(crate) struct NewDebugSession {
    pub owner_session_id: String,
    pub workspace: DebugWorkspaceKey,
    pub adapter_id: String,
    pub start: Option<DebugSessionStart>,
}

pub(crate) struct DebugSessionReservation {
    core: Arc<ManagerCore>,
    id: DebugSessionId,
    committed: bool,
}

mod reservation;

struct ManagerCore {
    config: DebugSessionManagerConfig,
    operations: Arc<DebugOperationConfig>,
    #[allow(dead_code)]
    inspection: Arc<DebugInspectionConfig>,
    #[allow(dead_code)]
    manager_id: u64,
    registry: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    entries: HashMap<DebugSessionId, Arc<SessionEntry>>,
    active_by_owner: HashMap<String, DebugSessionId>,
    ids_by_owner: HashMap<String, BTreeSet<DebugSessionId>>,
    terminal_order: VecDeque<DebugSessionId>,
    #[allow(dead_code)]
    next_sequence: u64,
}

struct SessionEntry {
    id: DebugSessionId,
    owner_session_id: String,
    workspace: DebugWorkspaceKey,
    adapter_id: String,
    data: Mutex<SessionData>,
    finalization: tokio::sync::Mutex<()>,
    operation: tokio::sync::Mutex<()>,
    source_hash: Arc<Semaphore>,
    breakpoint_validation: Arc<Semaphore>,
    inspection_parse: Arc<Semaphore>,
    publication: Mutex<()>,
    active_inspections: Mutex<Vec<Weak<InspectionToken>>>,
    #[cfg(test)]
    breakpoint_test_gates: breakpoints::BreakpointTestGates,
    closed: AtomicBool,
    operations: Arc<DebugOperationConfig>,
    changed: watch::Sender<u64>,
}

struct SessionData {
    state: DebugSessionState,
    start: Option<DebugSessionStart>,
    initialized_seen: bool,
    capabilities: Capabilities,
    transport: Option<SessionTransport>,
    output: OutputRing,
    supervisor: Option<JoinHandle<()>>,
    change_generation: u64,
    breakpoints: breakpoints::BreakpointRegistry,
    execution_revision: u64,
    frame_identities: HashMap<i32, (i32, u32)>,
    transport_revision: u64,
    inspection_invalidation: Option<InspectionInvalidation>,
}

#[derive(Clone, Copy)]
enum InspectionInvalidation {
    AdapterOrTargetExited,
    TransportFailure,
    ManagerOrOwnerShutdown,
}

const INSPECTION_OPEN: u8 = 0;
const INSPECTION_ADMITTED: u8 = 1;
const INSPECTION_RESPONSE_WON: u8 = 2;
const INSPECTION_SETTLED: u8 = 3;
const INSPECTION_PRE_INVALIDATED_BASE: u8 = 16;
const INSPECTION_POST_INVALIDATED_BASE: u8 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InspectionTokenCause {
    AdapterOrTargetExited,
    TransportFailure,
    ClientReplaced,
    ManagerOrOwnerShutdown,
    Revision,
}

impl InspectionTokenCause {
    fn code(self) -> u8 {
        match self {
            Self::AdapterOrTargetExited => 0,
            Self::TransportFailure => 1,
            Self::ClientReplaced => 2,
            Self::ManagerOrOwnerShutdown => 3,
            Self::Revision => 4,
        }
    }
}

struct InspectionToken {
    client: DapClient,
    state: AtomicU8,
    invalidator: Mutex<Option<crate::client::TrackedRequestInvalidator>>,
}

impl InspectionToken {
    fn new(client: DapClient) -> Arc<Self> {
        Arc::new(Self {
            client,
            state: AtomicU8::new(INSPECTION_OPEN),
            invalidator: Mutex::new(None),
        })
    }

    fn admit(&self) -> bool {
        self.state
            .compare_exchange(
                INSPECTION_OPEN,
                INSPECTION_ADMITTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn invalidate(&self, cause: InspectionTokenCause) {
        let pre = INSPECTION_PRE_INVALIDATED_BASE + cause.code();
        let post = INSPECTION_POST_INVALIDATED_BASE + cause.code();
        let won = self.client.synchronize_admission(|| {
            loop {
                let current = self.state.load(Ordering::Acquire);
                let next = match current {
                    INSPECTION_OPEN => pre,
                    INSPECTION_ADMITTED => post,
                    _ => return false,
                };
                if self
                    .state
                    .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return true;
                }
            }
        });
        if !won {
            return;
        }
        let invalidator = lock(&self.invalidator).clone();
        if let Some(invalidator) = invalidator {
            invalidator.invalidate();
        }
    }

    fn attach(&self, invalidator: crate::client::TrackedRequestInvalidator) {
        *lock(&self.invalidator) = Some(invalidator.clone());
        if self.invalidation().is_some() {
            invalidator.invalidate();
        }
    }

    fn mark_response_won(&self) {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current == INSPECTION_RESPONSE_WON || current == INSPECTION_SETTLED {
                return;
            }
            match self.state.compare_exchange(
                current,
                INSPECTION_RESPONSE_WON,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(next) => current = next,
            }
        }
    }

    fn settle(&self) {
        if self.state.load(Ordering::Acquire) != INSPECTION_RESPONSE_WON {
            self.state.store(INSPECTION_SETTLED, Ordering::Release);
        }
    }

    fn invalidation(&self) -> Option<(bool, InspectionTokenCause)> {
        let state = self.state.load(Ordering::Acquire);
        let (post, value) = if state >= INSPECTION_POST_INVALIDATED_BASE {
            (true, state - INSPECTION_POST_INVALIDATED_BASE)
        } else if state >= INSPECTION_PRE_INVALIDATED_BASE {
            (false, state - INSPECTION_PRE_INVALIDATED_BASE)
        } else {
            return None;
        };
        let cause = match value {
            0 => InspectionTokenCause::AdapterOrTargetExited,
            1 => InspectionTokenCause::TransportFailure,
            2 => InspectionTokenCause::ClientReplaced,
            3 => InspectionTokenCause::ManagerOrOwnerShutdown,
            4 => InspectionTokenCause::Revision,
            _ => return None,
        };
        Some((post, cause))
    }

    fn is_post_admission(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        state == INSPECTION_ADMITTED || state >= INSPECTION_POST_INVALIDATED_BASE
    }
}

impl InspectionInvalidation {
    fn from_end_reason(reason: &DebugSessionEndReason) -> Self {
        match reason {
            DebugSessionEndReason::DebuggeeExited { .. }
            | DebugSessionEndReason::AdapterExited { .. } => Self::AdapterOrTargetExited,
            DebugSessionEndReason::TransportClosed
            | DebugSessionEndReason::ProtocolError { .. }
            | DebugSessionEndReason::EventStreamLagged { .. } => Self::TransportFailure,
            DebugSessionEndReason::Requested
            | DebugSessionEndReason::LaunchCancelled
            | DebugSessionEndReason::OwnerDisconnected
            | DebugSessionEndReason::OwnerExpired
            | DebugSessionEndReason::ServerShutdown => Self::ManagerOrOwnerShutdown,
        }
    }
}

struct SessionTransport {
    client: DapClient,
    adapter: Option<AdapterProcess>,
    target: Option<OwnedTargetProcess>,
    termination_policy: Option<DebugTerminationPolicy>,
}

#[derive(Clone, Copy)]
pub(crate) enum DebugTerminationPolicy {
    AdapterLaunched,
    OwnedAttach,
}

mod breakpoints;
mod control;
mod startup;
pub(crate) use startup::finish_start;
#[cfg(test)]
pub(crate) use startup::start_protocol;
use startup::{
    deadline_result, deny_unsupported, initialize, start_after_initialize, startup_deadline,
};

impl ManagerCore {
    fn entry(&self, id: DebugSessionId) -> Result<Arc<SessionEntry>> {
        lock(&self.registry)
            .entries
            .get(&id)
            .cloned()
            .ok_or(DapError::SessionNotFound { session_id: id })
    }

    fn authorized_entry(&self, owner: &str, id: DebugSessionId) -> Result<Arc<SessionEntry>> {
        let entry = self.entry(id)?;
        if entry.owner_session_id != owner {
            return Err(DapError::SessionAccessDenied { session_id: id });
        }
        Ok(entry)
    }

    fn cancel_reservation_drop(self: &Arc<Self>, id: DebugSessionId) {
        let Ok(entry) = self.entry(id) else { return };
        entry.fence_terminal(InspectionInvalidation::ManagerOrOwnerShutdown);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let core = Arc::clone(self);
            handle.spawn(async move {
                let _ = core
                    .finalize_owned(entry, DebugSessionEndReason::LaunchCancelled, false, true)
                    .await;
            });
            return;
        }

        let _finalization = entry.finalization.blocking_lock();
        let transport = {
            let _publication = lock(&entry.publication);
            let mut data = lock(&entry.data);
            if data.transport.is_none() {
                drop(data);
                self.remove_entry(id);
                return;
            }
            if let Some(handle) = data.supervisor.take() {
                handle.abort();
            }
            data.transport.take()
        };
        if let Some(transport) = transport {
            transport.client.close();
            drop(transport);
        }
        self.remove_entry(id);
    }

    fn remove_entry(&self, id: DebugSessionId) -> Option<Arc<SessionEntry>> {
        let mut registry = lock(&self.registry);
        let entry = registry.entries.remove(&id)?;
        if registry.active_by_owner.get(&entry.owner_session_id) == Some(&id) {
            registry.active_by_owner.remove(&entry.owner_session_id);
        }
        if let Some(ids) = registry.ids_by_owner.get_mut(&entry.owner_session_id) {
            ids.remove(&id);
            if ids.is_empty() {
                registry.ids_by_owner.remove(&entry.owner_session_id);
            }
        }
        registry.terminal_order.retain(|candidate| *candidate != id);
        Some(entry)
    }

    fn drain_owner(&self, owner: &str) -> Vec<Arc<SessionEntry>> {
        let entries = {
            let registry = lock(&self.registry);
            registry
                .ids_by_owner
                .get(owner)
                .into_iter()
                .flatten()
                .filter_map(|id| registry.entries.get(id).cloned())
                .collect::<Vec<_>>()
        };
        for entry in &entries {
            entry.fence_terminal(InspectionInvalidation::ManagerOrOwnerShutdown);
        }
        entries
            .into_iter()
            .filter_map(|entry| self.remove_entry(entry.id))
            .collect()
    }

    fn drain_all(&self) -> Vec<Arc<SessionEntry>> {
        let entries = lock(&self.registry)
            .entries
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in &entries {
            entry.fence_terminal(InspectionInvalidation::ManagerOrOwnerShutdown);
        }
        entries
            .into_iter()
            .filter_map(|entry| self.remove_entry(entry.id))
            .collect()
    }

    async fn finalize_many(
        self: &Arc<Self>,
        entries: Vec<Arc<SessionEntry>>,
        reason: DebugSessionEndReason,
    ) -> DebugCleanupReport {
        let session_ids = entries.iter().map(|entry| entry.id).collect::<Vec<_>>();
        let core = Arc::clone(self);
        match tokio::spawn(async move { core.finalize_many_owned(entries, reason).await }).await {
            Ok(report) => report,
            Err(error) => DebugCleanupReport {
                cleaned: 0,
                already_ended: 0,
                failures: session_ids
                    .into_iter()
                    .map(|session_id| DebugCleanupFailure {
                        session_id,
                        message: format!("session cleanup task failed: {error}"),
                    })
                    .collect(),
            },
        }
    }

    async fn finalize_many_owned(
        self: &Arc<Self>,
        entries: Vec<Arc<SessionEntry>>,
        reason: DebugSessionEndReason,
    ) -> DebugCleanupReport {
        let mut report = DebugCleanupReport::default();
        for entry in entries {
            if entry
                .snapshot()
                .is_some_and(|snapshot| snapshot.state.is_terminal())
            {
                report.already_ended += 1;
                continue;
            }
            match self
                .finalize_owned(entry.clone(), reason.clone(), false, true)
                .await
            {
                Ok(()) => report.cleaned += 1,
                Err(error) => {
                    report.cleaned += 1;
                    report.failures.push(DebugCleanupFailure {
                        session_id: entry.id,
                        message: error.to_string(),
                    });
                }
            }
        }
        report
    }

    async fn finalize(
        self: &Arc<Self>,
        entry: Arc<SessionEntry>,
        reason: DebugSessionEndReason,
        retain: bool,
        abort_supervisor: bool,
    ) -> Result<()> {
        let session_id = entry.id;
        let core = Arc::clone(self);
        match tokio::spawn(async move {
            core.finalize_owned(entry, reason, retain, abort_supervisor)
                .await
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(DapError::SessionCleanupFailed {
                session_id,
                message: format!("session cleanup task failed: {error}"),
            }),
        }
    }

    async fn finalize_owned(
        self: &Arc<Self>,
        entry: Arc<SessionEntry>,
        reason: DebugSessionEndReason,
        retain: bool,
        abort_supervisor: bool,
    ) -> Result<()> {
        entry.fence_terminal(InspectionInvalidation::from_end_reason(&reason));
        let _finalization = entry.finalization.lock().await;
        let (transport, supervisor) = {
            let _publication = lock(&entry.publication);
            let mut data = lock(&entry.data);
            if data.state.is_terminal() {
                return Ok(());
            }
            data.state = DebugSessionState::Terminating;
            notify(&entry, &mut data);
            (data.transport.take(), data.supervisor.take())
        };
        if abort_supervisor && let Some(handle) = supervisor {
            handle.abort();
        }
        let mut failures = Vec::new();
        if let Some(mut transport) = transport {
            if let Some(policy) = transport.termination_policy {
                let terminate_debuggee = matches!(policy, DebugTerminationPolicy::AdapterLaunched);
                let disconnect = transport.client.request(
                    "disconnect",
                    Some(serde_json::json!({"restart":false,"terminateDebuggee":terminate_debuggee,"suspendDebuggee":false})),
                    self.config.disconnect_timeout,
                );
                match tokio::time::timeout(self.config.disconnect_timeout, disconnect).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => failures.push(format!("disconnect failed: {error}")),
                    Err(_) => failures.push("disconnect timed out".to_owned()),
                }
            }
            transport.client.close();
            if let Some(target) = transport.target.take()
                && let Err(error) = target.terminate(self.config.termination_grace).await
            {
                failures.push(format!("target cleanup failed: {error}"));
            }
            if let Some(adapter) = transport.adapter.take()
                && let Err(error) = adapter.terminate(self.config.termination_grace).await
            {
                failures.push(format!("adapter cleanup failed: {error}"));
            }
        }
        let cleanup_error = (!failures.is_empty()).then(|| failures.join("; "));
        {
            let _publication = lock(&entry.publication);
            let mut data = lock(&entry.data);
            data.state = DebugSessionState::Ended(DebugSessionEnd {
                reason,
                cleanup_error: cleanup_error.clone(),
            });
            notify(&entry, &mut data);
        }
        self.release_active(&entry);
        if retain {
            self.record_terminal(&entry);
        } else {
            self.remove_entry(entry.id);
        }
        if let Some(message) = cleanup_error {
            Err(DapError::SessionCleanupFailed {
                session_id: entry.id,
                message,
            })
        } else {
            Ok(())
        }
    }

    fn record_terminal(&self, entry: &Arc<SessionEntry>) {
        let mut registry = lock(&self.registry);
        if !registry.entries.contains_key(&entry.id) {
            return;
        }
        if !registry.terminal_order.contains(&entry.id) {
            registry.terminal_order.push_back(entry.id);
        }
        while registry.terminal_order.len() > self.config.max_retained_ended_sessions {
            if let Some(id) = registry.terminal_order.pop_front()
                && let Some(old) = registry.entries.remove(&id)
                && let Some(ids) = registry.ids_by_owner.get_mut(&old.owner_session_id)
            {
                ids.remove(&id);
                if ids.is_empty() {
                    registry.ids_by_owner.remove(&old.owner_session_id);
                }
            }
        }
    }

    fn release_active(&self, entry: &SessionEntry) {
        let mut registry = lock(&self.registry);
        if registry.active_by_owner.get(&entry.owner_session_id) == Some(&entry.id) {
            registry.active_by_owner.remove(&entry.owner_session_id);
        }
    }
}

impl Drop for ManagerCore {
    fn drop(&mut self) {
        let entries = {
            let registry = self
                .registry
                .get_mut()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            registry
                .entries
                .drain()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        };
        for entry in entries {
            entry.fence_terminal(InspectionInvalidation::ManagerOrOwnerShutdown);
            let _publication = lock(&entry.publication);
            let mut data = lock(&entry.data);
            if let Some(handle) = data.supervisor.take() {
                handle.abort();
            }
            if let Some(transport) = data.transport.take() {
                transport.client.close();
            }
        }
    }
}

#[cfg(test)]
mod tests;
