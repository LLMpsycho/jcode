use super::*;

pub(super) async fn supervise(
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

pub(super) fn transition(
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

pub(super) fn invalid_transition(
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
