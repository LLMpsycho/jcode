use super::*;

impl DebugSessionReservation {
    pub(super) async fn live_adapter_pid(&self) -> Result<u32> {
        let (pid, observer) = {
            let entry = self.core.entry(self.id)?;
            let data = lock(&entry.data);
            let adapter = data
                .transport
                .as_ref()
                .and_then(|value| value.adapter.as_ref())
                .ok_or_else(|| invalid_transition(&entry, &data, "inspect adapter"))?;
            let pid = adapter
                .pid()
                .ok_or_else(|| DapError::InvalidAdapterConfiguration {
                    message: "adapter has no live process identifier".to_owned(),
                })?;
            (pid, adapter.observer())
        };
        match observer.status().await? {
            Some(ProcessStatus::Running) => Ok(pid),
            Some(ProcessStatus::Exited { .. }) | None => {
                Err(DapError::InvalidAdapterConfiguration {
                    message: "adapter exited during owned-attach startup".to_owned(),
                })
            }
        }
    }

    pub(super) fn adapter_stderr(&self) -> Option<String> {
        let entry = self.core.entry(self.id).ok()?;
        let data = lock(&entry.data);
        let bytes = data.transport.as_ref()?.adapter.as_ref()?.recent_stderr();
        (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn target_pid(&self) -> Option<u32> {
        let entry = self.core.entry(self.id).ok()?;
        lock(&entry.data).transport.as_ref()?.target.as_ref()?.pid()
    }

    pub(super) async fn target_status(&self) -> Result<ProcessStatus> {
        let target = {
            let entry = self.core.entry(self.id)?;
            let data = lock(&entry.data);
            data.transport
                .as_ref()
                .and_then(|value| value.target.clone())
                .ok_or_else(|| invalid_transition(&entry, &data, "inspect target"))?
        };
        target.status().await
    }

    pub(super) async fn wait_for_target_exit(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<Option<i32>> {
        loop {
            if let ProcessStatus::Exited { code } = self.target_status().await? {
                return Ok(code);
            }
            tokio::time::timeout_at(
                deadline,
                tokio::time::sleep(self.core.config.process_poll_interval),
            )
            .await
            .map_err(|_| DapError::DebugStartupTimeout {
                phase: DebugStartupPhase::StartRequest,
            })?;
        }
    }

    pub(crate) fn set_start(&self, start: DebugSessionStart) -> Result<()> {
        let entry = self.core.entry(self.id)?;
        let _publication = lock(&entry.publication);
        let mut data = lock(&entry.data);
        data.start = Some(start);
        notify(&entry, &mut data);
        Ok(())
    }
}

pub(super) fn deny_unsupported(operation: DebugStartOperation) -> Result<()> {
    #[cfg(windows)]
    return Err(DapError::ProcessContainmentUnavailable {
        operation,
        platform: "windows",
    });
    #[cfg(not(windows))]
    {
        let _ = operation;
        Ok(())
    }
}

pub(super) fn startup_deadline(timeout: Duration) -> Result<tokio::time::Instant> {
    tokio::time::Instant::now()
        .checked_add(timeout)
        .ok_or(DapError::InvalidManagerConfiguration {
            message: "startup timeout exceeds the supported instant range".to_owned(),
        })
}

fn remaining(deadline: tokio::time::Instant, phase: DebugStartupPhase) -> Result<Duration> {
    deadline
        .checked_duration_since(tokio::time::Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(DapError::DebugStartupTimeout { phase })
}

pub(super) async fn deadline_result<T>(
    deadline: tokio::time::Instant,
    phase: DebugStartupPhase,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| DapError::DebugStartupTimeout { phase })?
}

pub(super) async fn initialize(
    reservation: &DebugSessionReservation,
    profile: AdapterProfile,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let response = reservation
        .client()?
        .request(
            "initialize",
            Some(
                serde_json::to_value(profile.initialize_arguments())
                    .map_err(|error| DapError::InvalidMessage(error.to_string()))?,
            ),
            remaining(deadline, DebugStartupPhase::Initialize)?,
        )
        .await
        .map_err(|error| match error {
            DapError::RequestTimeout { .. } => DapError::DebugStartupTimeout {
                phase: DebugStartupPhase::Initialize,
            },
            other => other,
        })?;
    let body = response.body.unwrap_or_else(|| serde_json::json!({}));
    let capabilities =
        serde_json::from_value(body).map_err(|error| DapError::InvalidInitializeResponse {
            message: error.to_string(),
        })?;
    reservation.set_capabilities(capabilities)
}

#[cfg(test)]
pub(crate) async fn start_protocol(
    reservation: &DebugSessionReservation,
    profile: AdapterProfile,
    command: &str,
    arguments: Value,
    deadline: tokio::time::Instant,
) -> Result<()> {
    initialize(reservation, profile, deadline).await?;
    start_after_initialize(reservation, command, arguments, deadline).await
}

pub(super) async fn start_after_initialize(
    reservation: &DebugSessionReservation,
    command: &str,
    arguments: Value,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let client = reservation.client()?;
    let request = client.request(
        command,
        Some(arguments),
        remaining(deadline, DebugStartupPhase::StartRequest)?,
    );
    tokio::pin!(request);
    let configurable = reservation.wait_until_configurable(deadline);
    tokio::pin!(configurable);
    let mut response = None;
    tokio::select! {
        result = &mut request => response = Some(result.map_err(|error| match error {
            DapError::RequestTimeout { .. } => DapError::DebugStartupTimeout { phase: DebugStartupPhase::StartRequest },
            other => other,
        })?),
        result = &mut configurable => { result?; }
    }
    if response.is_some() {
        configurable.await?;
    }
    let capabilities = {
        let entry = reservation.core.entry(reservation.id)?;
        lock(&entry.data).capabilities.clone()
    };
    if capabilities.supports_configuration_done_request == Some(true) {
        client
            .request(
                "configurationDone",
                Some(serde_json::json!({})),
                remaining(deadline, DebugStartupPhase::ConfigurationDone)?,
            )
            .await
            .map_err(|error| match error {
                DapError::RequestTimeout { .. } => DapError::DebugStartupTimeout {
                    phase: DebugStartupPhase::ConfigurationDone,
                },
                other => other,
            })?;
    }
    if response.is_none() {
        request.await.map_err(|error| match error {
            DapError::RequestTimeout { .. } => DapError::DebugStartupTimeout {
                phase: DebugStartupPhase::StartRequest,
            },
            other => other,
        })?;
    }
    reservation.complete_start()
}

pub(crate) async fn finish_start(
    reservation: DebugSessionReservation,
    result: Result<()>,
) -> Result<DebugSessionSnapshot> {
    match result {
        Ok(()) => {
            let core = Arc::clone(&reservation.core);
            let owner = core.entry(reservation.id)?.owner_session_id.clone();
            let id = reservation.commit()?;
            core.authorized_entry(&owner, id)?
                .snapshot()
                .ok_or(DapError::SessionNotFound { session_id: id })
        }
        Err(error) => {
            let error = match reservation.adapter_stderr() {
                Some(adapter_stderr) => DapError::DebugStartupFailed {
                    message: error.to_string(),
                    adapter_stderr,
                },
                None => error,
            };
            match reservation.cancel_start().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(DapError::DebugStartupFailed {
                    message: format!("{error}; cleanup also failed: {cleanup}"),
                    adapter_stderr: String::new(),
                }),
            }
        }
    }
}
