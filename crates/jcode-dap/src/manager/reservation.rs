use super::*;

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

    pub(super) fn attach_transport(
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
            let _publication = lock(&entry.publication);
            entry.invalidate_inspections(InspectionTokenCause::ClientReplaced);
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
            data.transport_revision = data.transport_revision.saturating_add(1);
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
        let _publication = lock(&entry.publication);
        let mut data = lock(&entry.data);
        if data
            .transport
            .as_ref()
            .is_none_or(|transport| transport.target.is_some())
        {
            return Err(invalid_transition(&entry, &data, "attach target"));
        }
        let Some(transport) = data.transport.as_mut() else {
            return Err(invalid_transition(&entry, &data, "attach target"));
        };
        transport.target = Some(target);
        Ok(())
    }

    pub(super) fn client(&self) -> Result<DapClient> {
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
        let _publication = lock(&entry.publication);
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
            let _publication = lock(&entry.publication);
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
        let _publication = lock(&entry.publication);
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
        let _publication = lock(&entry.publication);
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
