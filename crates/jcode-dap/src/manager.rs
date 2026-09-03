use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot, watch};
use tokio::task::JoinHandle;

use crate::client::DapClientStatus;
use crate::launch::{AdapterProfile, ResolvedLaunch, resolve_program, revalidate_program};
use crate::process::{OwnedChildObserver, OwnedTargetProcess};
use crate::session::{OutputRing, SessionEvent, next_manager_id, parse_event};
use crate::{
    AdapterCommand, AdapterProcess, Capabilities, DapClient, DapError, DebugAdapterConfig,
    DebugCleanupFailure, DebugCleanupReport, DebugLaunchRequest, DebugOutputCursor,
    DebugOutputPage, DebugOwnedAttachRequest, DebugSessionEnd, DebugSessionEndReason,
    DebugSessionId, DebugSessionManagerConfig, DebugSessionSnapshot, DebugSessionStart,
    DebugSessionState, DebugSessionStateKind, DebugStartOperation, DebugStartupPhase,
    DebugWorkspaceKey, OwnerCleanupCause, ProcessStatus, Response, Result,
};

#[derive(Clone)]
pub struct DebugSessionManager {
    core: Arc<ManagerCore>,
}

impl DebugSessionManager {
    pub fn new(config: DebugSessionManagerConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            core: Arc::new(ManagerCore {
                config,
                manager_id: next_manager_id()?,
                registry: Mutex::new(Registry::default()),
            }),
        })
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
        adapter.revalidate()?;
        let process = deadline_result(
            deadline,
            DebugStartupPhase::Initialize,
            AdapterProcess::spawn(&AdapterCommand::new(
                adapter.executable(),
                workspace.canonical_root(),
            )),
        )
        .await?;
        reservation.attach_process(process, DebugTerminationPolicy::AdapterLaunched)?;
        let result = async {
            initialize(&reservation, AdapterProfile::LldbDap, deadline).await?;
            revalidate_program(&workspace, &resolved.target)?;
            start_after_initialize(
                &reservation,
                "launch",
                AdapterProfile::LldbDap.launch_arguments(&resolved),
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
        adapter.revalidate()?;
        let process = deadline_result(
            deadline,
            DebugStartupPhase::Initialize,
            AdapterProcess::spawn(&AdapterCommand::new(
                adapter.executable(),
                workspace.canonical_root(),
            )),
        )
        .await?;
        reservation.attach_process(process, DebugTerminationPolicy::OwnedAttach)?;
        if let Err(error) = initialize(&reservation, AdapterProfile::LldbDap, deadline).await {
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
                AdapterProfile::LldbDap.attach_arguments(pid),
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
            }),
            finalization: AsyncMutex::new(()),
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

#[allow(dead_code)]
impl DebugSessionReservation {
    pub(crate) fn id(&self) -> DebugSessionId {
        self.id
    }

    #[allow(dead_code)]
    pub(crate) fn attach_process(
        &mut self,
        process: AdapterProcess,
        termination_policy: DebugTerminationPolicy,
    ) -> Result<()> {
        let client = process.client().clone();
        self.attach_transport(client, Some(process), Some(termination_policy))
    }

    #[cfg(test)]
    pub(crate) fn attach_client(&mut self, client: DapClient) -> Result<()> {
        self.attach_transport(client, None, None)
    }

    #[cfg(test)]
    pub(crate) fn attach_start_client(
        &mut self,
        client: DapClient,
        policy: DebugTerminationPolicy,
    ) -> Result<()> {
        self.attach_transport(client, None, Some(policy))
    }

    fn attach_transport(
        &mut self,
        client: DapClient,
        process: Option<AdapterProcess>,
        termination_policy: Option<DebugTerminationPolicy>,
    ) -> Result<()> {
        let supervised_process = process.as_ref().map(AdapterProcess::observer);
        let entry = self.core.entry(self.id)?;
        let events = client.subscribe_events();
        let status = client.subscribe_status();
        let observed_status = *status.borrow();
        {
            let mut data = lock(&entry.data);
            transition(
                &entry,
                &mut data,
                DebugSessionState::Initializing,
                "attach adapter",
            )?;
            data.transport = Some(SessionTransport {
                client: client.clone(),
                adapter: process,
                target: None,
                termination_policy,
            });
        }
        let core = Arc::downgrade(&self.core);
        let supervised = entry.clone();
        let (start, ready) = oneshot::channel();
        let handle = tokio::spawn(async move {
            if ready.await.is_err() {
                return;
            }
            supervise(
                core,
                supervised,
                events,
                status,
                observed_status,
                supervised_process,
            )
            .await;
        });
        lock(&entry.data).supervisor = Some(handle);
        let _ignored = start.send(());
        Ok(())
    }

    pub(crate) fn attach_target(&mut self, target: OwnedTargetProcess) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let mut data = lock(&entry.data);
        if data
            .transport
            .as_ref()
            .is_none_or(|transport| transport.target.is_some())
        {
            return Err(invalid_transition(&entry, &data, "attach target"));
        }
        data.transport.as_mut().unwrap().target = Some(target);
        Ok(())
    }

    fn client(&self) -> Result<DapClient> {
        let entry = self.core.entry(self.id)?;
        let data = lock(&entry.data);
        data.transport
            .as_ref()
            .map(|transport| transport.client.clone())
            .ok_or_else(|| invalid_transition(&entry, &data, "access transport"))
    }

    pub(crate) async fn wait_until_configurable(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<DebugSessionStateKind> {
        let entry = self.core.entry(self.id)?;
        let mut changed = entry.changed.subscribe();
        loop {
            {
                let data = lock(&entry.data);
                if data.initialized_seen {
                    return Ok(data.state.kind());
                }
                if matches!(
                    data.state,
                    DebugSessionState::Terminating | DebugSessionState::Ended(_)
                ) {
                    return Err(DapError::SessionEndedDuringStartup {
                        session_id: self.id,
                        message: format!("state is {:?}", data.state.kind()),
                    });
                }
            }
            tokio::time::timeout_at(deadline, changed.changed())
                .await
                .map_err(|_| DapError::DebugStartupTimeout {
                    phase: DebugStartupPhase::AwaitInitialized,
                })?
                .map_err(|_| DapError::TransportClosed)?;
        }
    }

    pub(crate) fn complete_start(&self) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let mut data = lock(&entry.data);
        match data.state {
            DebugSessionState::Stopped(_) | DebugSessionState::Running => Ok(()),
            DebugSessionState::Configuring => transition(
                &entry,
                &mut data,
                DebugSessionState::Running,
                "complete start",
            ),
            _ => Err(invalid_transition(&entry, &data, "complete start")),
        }
    }

    pub(crate) async fn cancel_start(mut self) -> Result<()> {
        self.committed = true;
        let entry = self.core.entry(self.id)?;
        self.core
            .finalize(entry, DebugSessionEndReason::LaunchCancelled, true, true)
            .await
    }

    pub(crate) fn set_capabilities(&self, capabilities: Capabilities) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let client = {
            let mut data = lock(&entry.data);
            if !matches!(
                data.state,
                DebugSessionState::Initializing | DebugSessionState::Configuring
            ) {
                return Err(invalid_transition(&entry, &data, "set capabilities"));
            }
            let client = data
                .transport
                .as_ref()
                .map(|transport| transport.client.clone());
            data.capabilities = capabilities.clone();
            notify(&entry, &mut data);
            client
        };
        if let Some(client) = client {
            client
                .set_supports_cancel_request(capabilities.supports_cancel_request.unwrap_or(false));
        }
        Ok(())
    }

    pub(crate) fn mark_configuring(&self) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let mut data = lock(&entry.data);
        if matches!(data.state, DebugSessionState::Configuring) {
            return Ok(());
        }
        transition(
            &entry,
            &mut data,
            DebugSessionState::Configuring,
            "configure",
        )
    }

    pub(crate) fn mark_running(&self) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let mut data = lock(&entry.data);
        if matches!(data.state, DebugSessionState::Running) {
            return Ok(());
        }
        transition(&entry, &mut data, DebugSessionState::Running, "run")
    }

    pub(crate) fn commit(mut self) -> Result<DebugSessionId> {
        let entry = self.core.entry(self.id)?;
        let data = lock(&entry.data);
        if matches!(
            data.state,
            DebugSessionState::Terminating | DebugSessionState::Ended(_)
        ) {
            return Err(DapError::SessionEndedDuringStartup {
                session_id: self.id,
                message: format!("state is {:?}", data.state.kind()),
            });
        }
        drop(data);
        self.committed = true;
        Ok(self.id)
    }
}

impl Drop for DebugSessionReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.core.cancel_reservation_drop(self.id);
        }
    }
}

struct ManagerCore {
    config: DebugSessionManagerConfig,
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
    finalization: AsyncMutex<()>,
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

mod startup;
pub(crate) use startup::finish_start;
#[cfg(test)]
pub(crate) use startup::start_protocol;
use startup::{
    deadline_result, deny_unsupported, initialize, start_after_initialize, startup_deadline,
};

impl SessionEntry {
    fn snapshot(&self) -> Option<DebugSessionSnapshot> {
        let data = lock(&self.data);
        Some(DebugSessionSnapshot {
            id: self.id,
            workspace: self.workspace.clone(),
            adapter_id: self.adapter_id.clone(),
            start: data.start.clone()?,
            state: data.state.clone(),
            capabilities: data.capabilities.clone(),
            output: data.output.status(),
        })
    }
}

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
        let mut registry = lock(&self.registry);
        let ids = registry.ids_by_owner.remove(owner).unwrap_or_default();
        registry.active_by_owner.remove(owner);
        registry.terminal_order.retain(|id| !ids.contains(id));
        ids.into_iter()
            .filter_map(|id| registry.entries.remove(&id))
            .collect()
    }

    fn drain_all(&self) -> Vec<Arc<SessionEntry>> {
        let mut registry = lock(&self.registry);
        let entries = registry.entries.drain().map(|(_, entry)| entry).collect();
        registry.active_by_owner.clear();
        registry.ids_by_owner.clear();
        registry.terminal_order.clear();
        entries
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
        let _finalization = entry.finalization.lock().await;
        let (transport, supervisor) = {
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

async fn supervise(
    core: Weak<ManagerCore>,
    entry: Arc<SessionEntry>,
    mut events: broadcast::Receiver<crate::Event>,
    mut status: watch::Receiver<DapClientStatus>,
    mut observed: DapClientStatus,
    process: Option<OwnedChildObserver>,
) {
    if observed.dropped_output_events > 0 {
        let mut data = lock(&entry.data);
        data.output.add_source_loss(observed.dropped_output_events);
        notify(&entry, &mut data);
    }
    let initial_end = if observed.dropped_non_output_events > 0 {
        Some(DebugSessionEndReason::ProtocolError {
            message: format!(
                "{} oversized non-output DAP event(s) were lost",
                observed.dropped_non_output_events
            ),
        })
    } else if observed.closed {
        Some(DebugSessionEndReason::TransportClosed)
    } else {
        None
    };
    if let Some(reason) = initial_end {
        if let Some(core) = core.upgrade() {
            let _ignored = core.finalize(entry, reason, true, false).await;
        }
        return;
    }
    let Some(initial_core) = core.upgrade() else {
        return;
    };
    let mut interval = tokio::time::interval(initial_core.config.process_poll_interval);
    drop(initial_core);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let target = {
            let data = lock(&entry.data);
            data.transport
                .as_ref()
                .and_then(|transport| transport.target.as_ref().map(OwnedTargetProcess::observer))
        };
        let end = tokio::select! {
            event = events.recv() => match event {
                Ok(event) => apply_event(&entry, event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => Some(DebugSessionEndReason::EventStreamLagged { skipped }),
                Err(broadcast::error::RecvError::Closed) => Some(DebugSessionEndReason::TransportClosed),
            },
            changed = status.changed() => {
                if changed.is_err() {
                    Some(DebugSessionEndReason::TransportClosed)
                } else {
                    let current = *status.borrow_and_update();
                    let output_loss = current.dropped_output_events.saturating_sub(observed.dropped_output_events);
                    let non_output_loss = current.dropped_non_output_events.saturating_sub(observed.dropped_non_output_events);
                    if output_loss > 0 {
                        let mut data = lock(&entry.data);
                        data.output.add_source_loss(output_loss);
                        notify(&entry, &mut data);
                    }
                    observed = current;
                    if non_output_loss > 0 {
                        Some(DebugSessionEndReason::ProtocolError { message: format!("{non_output_loss} oversized non-output DAP event(s) were lost") })
                    } else if current.closed {
                        Some(DebugSessionEndReason::TransportClosed)
                    } else {
                        None
                    }
                }
            },
            _ = interval.tick(), if process.is_some() || target.is_some() => {
                if let Some(target) = target {
                    match target.status().await {
                        Ok(Some(ProcessStatus::Exited { code })) => Some(DebugSessionEndReason::DebuggeeExited { exit_code: code.map(i64::from) }),
                        Err(error) => Some(DebugSessionEndReason::ProtocolError { message: error.to_string() }),
                        Ok(Some(ProcessStatus::Running)) => match process.as_ref() {
                            Some(process) => match process.status().await {
                                Ok(Some(ProcessStatus::Running)) | Ok(None) => None,
                                Ok(Some(ProcessStatus::Exited { code })) => Some(DebugSessionEndReason::AdapterExited { exit_code: code }),
                                Err(error) => Some(DebugSessionEndReason::ProtocolError { message: error.to_string() }),
                            },
                            None => None,
                        },
                        Ok(None) => None,
                    }
                } else {
                    match process.as_ref() {
                        Some(process) => match process.status().await {
                            Ok(Some(ProcessStatus::Running)) | Ok(None) => None,
                            Ok(Some(ProcessStatus::Exited { code })) => Some(DebugSessionEndReason::AdapterExited { exit_code: code }),
                            Err(error) => Some(DebugSessionEndReason::ProtocolError { message: error.to_string() }),
                        },
                        None => None,
                    }
                }
            }
        };
        if let Some(reason) = end {
            if let Some(core) = core.upgrade() {
                let _ignored = core.finalize(entry.clone(), reason, true, false).await;
            }
            return;
        }
    }
}

fn apply_event(entry: &SessionEntry, event: crate::Event) -> Option<DebugSessionEndReason> {
    let event = match parse_event(event) {
        Ok(event) => event,
        Err(error) => {
            return Some(DebugSessionEndReason::ProtocolError {
                message: error.to_string(),
            });
        }
    };
    let mut data = lock(&entry.data);
    if matches!(
        data.state,
        DebugSessionState::Terminating | DebugSessionState::Ended(_)
    ) {
        return None;
    }
    match event {
        SessionEvent::Output(category, output) => data.output.push(category, output),
        SessionEvent::Initialized => {
            data.initialized_seen = true;
            if matches!(data.state, DebugSessionState::Initializing) {
                data.state = DebugSessionState::Configuring;
            }
        }
        SessionEvent::Stopped(stopped) => {
            if matches!(
                data.state,
                DebugSessionState::Initializing
                    | DebugSessionState::Configuring
                    | DebugSessionState::Running
                    | DebugSessionState::Stopped(_)
            ) {
                data.state = DebugSessionState::Stopped(stopped);
            } else {
                return Some(DebugSessionEndReason::ProtocolError {
                    message: "stopped event arrived in an invalid session state".to_owned(),
                });
            }
        }
        SessionEvent::Continued => {
            if matches!(
                data.state,
                DebugSessionState::Stopped(_) | DebugSessionState::Configuring
            ) {
                data.state = DebugSessionState::Running;
            } else if !matches!(data.state, DebugSessionState::Running) {
                return Some(DebugSessionEndReason::ProtocolError {
                    message: "continued event arrived in an invalid session state".to_owned(),
                });
            }
        }
        SessionEvent::Terminated => {
            return Some(DebugSessionEndReason::DebuggeeExited { exit_code: None });
        }
        SessionEvent::Exited(exit_code) => {
            return Some(DebugSessionEndReason::DebuggeeExited { exit_code });
        }
        SessionEvent::Ignore => return None,
    }
    notify(entry, &mut data);
    None
}

fn transition(
    entry: &SessionEntry,
    data: &mut SessionData,
    next: DebugSessionState,
    operation: &'static str,
) -> Result<()> {
    let legal = matches!(
        (&data.state, &next),
        (DebugSessionState::Reserved, DebugSessionState::Initializing)
            | (
                DebugSessionState::Initializing,
                DebugSessionState::Configuring
            )
            | (DebugSessionState::Configuring, DebugSessionState::Running)
            | (
                DebugSessionState::Configuring,
                DebugSessionState::Stopped(_)
            )
            | (DebugSessionState::Running, DebugSessionState::Stopped(_))
            | (DebugSessionState::Stopped(_), DebugSessionState::Running)
    );
    if !legal {
        return Err(invalid_transition(entry, data, operation));
    }
    data.state = next;
    notify(entry, data);
    Ok(())
}

fn invalid_transition(
    entry: &SessionEntry,
    data: &SessionData,
    operation: &'static str,
) -> DapError {
    DapError::InvalidSessionTransition {
        session_id: entry.id,
        state: data.state.kind(),
        operation,
    }
}

fn notify(entry: &SessionEntry, data: &mut SessionData) {
    data.change_generation = data.change_generation.saturating_add(1);
    entry.changed.send_replace(data.change_generation);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
