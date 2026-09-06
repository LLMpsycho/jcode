use super::*;

pub(super) async fn record_processing_completion(
    done_session: Option<&str>,
    result: Result<()>,
    completion_report: Option<String>,
    swarm: &SwarmStatusRefs<'_>,
) {
    match result {
        Ok(()) => {
            if let Some(session_id) = done_session {
                update_member_status_with_report(
                    session_id,
                    "ready",
                    None,
                    completion_report,
                    swarm.members,
                    swarm.swarms_by_id,
                    Some(swarm.event_history),
                    Some(swarm.event_counter),
                    Some(swarm.event_tx),
                )
                .await;
            }
        }
        Err(e) => {
            if let Some(session_id) = done_session {
                update_member_status(
                    session_id,
                    "failed",
                    Some(truncate_detail(&e.to_string(), 120)),
                    swarm.members,
                    swarm.swarms_by_id,
                    Some(swarm.event_history),
                    Some(swarm.event_counter),
                    Some(swarm.event_tx),
                )
                .await;
            }
            let retry_after_secs = e
                .downcast_ref::<StreamError>()
                .and_then(|se| se.retry_after_secs);
            if retry_after_secs.is_some() {
                crate::telemetry::record_error(crate::telemetry::ErrorCategory::RateLimited);
            } else {
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                if lower.contains("timeout") {
                    crate::telemetry::record_error(
                        crate::telemetry::ErrorCategory::ProviderTimeout,
                    );
                } else if crate::provider::error_looks_like_credential_failure(&msg)
                    || lower.contains("403 forbidden")
                {
                    // Use the shared credential-failure classifier instead of a
                    // bare `contains("auth")`: that substring also matched
                    // unrelated errors (e.g. any message mentioning "author" or
                    // OAuth flow noise) and inflated the auth_failed telemetry
                    // counter.
                    crate::telemetry::record_error(crate::telemetry::ErrorCategory::AuthFailed);
                }
            }
        }
    }
}

pub(super) async fn append_context_message(
    id: u64,
    content: &str,
    images: Vec<(String, String)>,
    client_session_id: &str,
    client_is_processing: bool,
    agent: &Arc<Mutex<Agent>>,
    client_event_tx: &mpsc::UnboundedSender<ServerEvent>,
) {
    let Ok(mut agent) = agent.try_lock() else {
        send_agent_busy_error(
            id,
            "context_message",
            client_session_id,
            client_is_processing,
            client_event_tx,
        );
        return;
    };
    let result = agent.append_user_context_message(content, images);
    let event = match result {
        Ok(()) => ServerEvent::ContextMessageAdded { id },
        Err(error) => ServerEvent::Error {
            id,
            message: crate::util::format_error_chain(&error),
            retry_after_secs: None,
        },
    };
    if client_event_tx.send(event).is_err() {
        crate::logging::debug("Turn event recipient disconnected");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_processing_message(
    message: ProcessingMessage,
    client_session_id: &str,
    state: &mut ProcessingState<'_>,
    agent: &Arc<Mutex<Agent>>,
    client_event_tx: &mpsc::UnboundedSender<ServerEvent>,
    processing_done_tx: &mpsc::UnboundedSender<ProcessingCompletion>,
    client_terminal_env: Vec<(String, String)>,
    swarm: &SwarmStatusRefs<'_>,
) {
    let ProcessingMessage {
        id,
        content,
        images,
        system_reminder,
        active_skill,
    } = message;
    if server_reload_starting() {
        crate::logging::info(&format!(
            "Rejecting new message for session {} because server reload is starting",
            client_session_id
        ));
        if client_event_tx
            .send(ServerEvent::Reloading { new_socket: None })
            .is_err()
        {
            crate::logging::debug("Turn event recipient disconnected");
        }
        return;
    }

    if *state.client_is_processing {
        if client_event_tx
            .send(ServerEvent::Error {
                id,
                message: "Already processing a message".to_string(),
                retry_after_secs: None,
            })
            .is_err()
        {
            crate::logging::debug("Turn event recipient disconnected");
        }
        return;
    }

    if !agent
        .lock()
        .await
        .set_remote_active_skill(active_skill.clone())
    {
        let skill_name = active_skill.as_deref().unwrap_or("");
        if client_event_tx
            .send(ServerEvent::Error {
                id,
                message: format!("Skill '{skill_name}' is not installed on the server"),
                retry_after_secs: None,
            })
            .is_err()
        {
            crate::logging::debug("Turn event recipient disconnected");
        }
        return;
    }

    *state.client_is_processing = true;
    *state.message_id = Some(id);
    *state.session_id = Some(client_session_id.to_string());

    if let Some(reminder) = system_reminder.as_deref()
        && let Err(error) = crate::server::reload_recovery::mark_delivered_if_matching_continuation(
            client_session_id,
            reminder,
            "client_message_accepted",
        )
    {
        crate::logging::warn(&format!(
            "Failed to mark reload recovery intent delivered for accepted message session={} id={}: {}",
            client_session_id, id, error
        ));
    }

    update_member_status(
        client_session_id,
        "running",
        Some(truncate_detail(&content, 120)),
        swarm.members,
        swarm.swarms_by_id,
        Some(swarm.event_history),
        Some(swarm.event_counter),
        Some(swarm.event_tx),
    )
    .await;

    let start_message_index = {
        let agent_guard = agent.lock().await;
        agent_guard.message_count()
    };
    let agent = Arc::clone(agent);
    let report_agent = Arc::clone(&agent);
    let tx = crate::server::state::session_event_fanout_sender_with_fallback(
        client_session_id.to_string(),
        Arc::clone(swarm.members),
        client_event_tx.clone(),
    );
    let done_tx = processing_done_tx.clone();
    crate::logging::info(&format!("Processing message id={} spawning task", id));
    *state.task = Some(tokio::spawn(async move {
        let event_tx = tx.clone();
        let result = match std::panic::AssertUnwindSafe(crate::hooks::with_client_terminal_env(
            client_terminal_env,
            process_message_streaming_mpsc(agent, &content, images, system_reminder, event_tx),
        ))
        .catch_unwind()
        .await
        {
            Ok(result) => result,
            Err(panic_payload) => {
                let msg = if let Some(text) = panic_payload.downcast_ref::<&str>() {
                    text.to_string()
                } else if let Some(text) = panic_payload.downcast_ref::<String>() {
                    text.clone()
                } else {
                    "unknown panic".to_string()
                };
                crate::logging::error(&format!(
                    "Processing task PANICKED for message id={}: {}",
                    id, msg
                ));
                Err(anyhow::anyhow!("Processing task panicked: {}", msg))
            }
        };
        match &result {
            Ok(()) => crate::logging::info(&format!(
                "Processing task completed OK for message id={}",
                id
            )),
            Err(error) => crate::logging::warn(&format!(
                "Processing task completed with error for message id={}: {}",
                id, error
            )),
        }
        let completion_report = if result.is_ok() {
            let agent = report_agent.lock().await;
            agent.latest_assistant_text_after(start_message_index)
        } else {
            None
        };
        // Keep the terminal event on the same ordered fanout channel as the
        // stream. Sending it later from the owning client's event loop could
        // race ahead of the final MessageEnd for newly attached clients.
        let terminal_event = match &result {
            Ok(()) => ServerEvent::Done { id },
            Err(error) => ServerEvent::Error {
                id,
                message: crate::util::format_error_chain(error),
                retry_after_secs: error
                    .downcast_ref::<StreamError>()
                    .and_then(|stream_error| stream_error.retry_after_secs),
            },
        };
        processing_completion::publish(
            id,
            result,
            completion_report,
            terminal_event,
            &tx,
            &done_tx,
        )
        .await;
    }));
}

pub(super) async fn cancel_processing_message(
    state: &mut ProcessingState<'_>,
    session_control: &SessionControlHandle,
    client_event_tx: &mpsc::UnboundedSender<ServerEvent>,
    swarm: &SwarmStatusRefs<'_>,
    request_id: Option<u64>,
    request_decoded_at: Option<Instant>,
) {
    let cancel_start = Instant::now();
    let session_label = state
        .session_id
        .as_deref()
        .unwrap_or(session_control.session_id.as_str())
        .to_string();
    crate::logging::info(&format!(
        "SERVER_INTERRUPT_CANCEL_RECEIVED request_id={:?} session={} control_session={} client_processing={} message_id={:?} has_task={} decoded_age_ms={:?}",
        request_id,
        session_label,
        session_control.session_id,
        *state.client_is_processing,
        *state.message_id,
        state.task.is_some(),
        request_decoded_at.map(|instant| instant.elapsed().as_millis())
    ));
    if let Some(mut handle) = state.task.take() {
        if handle.is_finished() {
            crate::logging::info(&format!(
                "SERVER_INTERRUPT_CANCEL_IGNORED_FINISHED request_id={:?} session={} message_id={:?} total_ms={}",
                request_id,
                session_label,
                *state.message_id,
                cancel_start.elapsed().as_millis()
            ));
            *state.task = Some(handle);
            return;
        }
        let cancel_epoch = session_control.request_cancel();
        crate::logging::info(&format!(
            "SERVER_INTERRUPT_CANCEL_SIGNALLED request_id={:?} session={} message_id={:?} wait_ms=500",
            request_id, session_label, *state.message_id
        ));
        match tokio::time::timeout(std::time::Duration::from_millis(500), &mut handle).await {
            Ok(_) => {
                crate::logging::info(&format!(
                    "SERVER_INTERRUPT_CANCEL_COOPERATIVE_DONE request_id={:?} session={} message_id={:?} elapsed_ms={}",
                    request_id,
                    session_label,
                    *state.message_id,
                    cancel_start.elapsed().as_millis()
                ));
            }
            Err(_) => {
                crate::logging::warn(&format!(
                    "SERVER_INTERRUPT_CANCEL_COOPERATIVE_TIMEOUT request_id={:?} session={} message_id={:?} elapsed_ms={} action=abort_task",
                    request_id,
                    session_label,
                    *state.message_id,
                    cancel_start.elapsed().as_millis()
                ));
                handle.abort();
                match tokio::time::timeout(std::time::Duration::from_millis(2000), handle).await {
                    Ok(_) => crate::logging::info(&format!(
                        "SERVER_INTERRUPT_CANCEL_ABORT_RELEASED request_id={:?} session={} elapsed_ms={}",
                        request_id,
                        session_label,
                        cancel_start.elapsed().as_millis()
                    )),
                    Err(_) => crate::logging::warn(&format!(
                        "SERVER_INTERRUPT_CANCEL_ABORT_RELEASE_TIMEOUT request_id={:?} session={} elapsed_ms={} wait_ms=2000",
                        request_id,
                        session_label,
                        cancel_start.elapsed().as_millis()
                    )),
                }
            }
        }
        // Only clear the cancel we fired: a newer cancel (repeated Esc, jade
        // relay, another connection) must not be erased before its target
        // observes it (issue #428).
        session_control.reset_cancel_if_epoch(cancel_epoch);
        *state.task = None;
        *state.client_is_processing = false;
        if let Some(session_id) = state.session_id.take() {
            update_member_status(
                &session_id,
                "stopped",
                Some("cancelled".to_string()),
                swarm.members,
                swarm.swarms_by_id,
                Some(swarm.event_history),
                Some(swarm.event_counter),
                Some(swarm.event_tx),
            )
            .await;
        }
        if let Some(message_id) = state.message_id.take() {
            if client_event_tx.send(ServerEvent::Interrupted).is_err() {
                crate::logging::debug("Turn event recipient disconnected");
            }
            if client_event_tx
                .send(ServerEvent::Done { id: message_id })
                .is_err()
            {
                crate::logging::debug("Turn event recipient disconnected");
            }
            crate::logging::info(&format!(
                "SERVER_INTERRUPT_CANCEL_EVENTS_EMITTED request_id={:?} session={} interrupted=true done_id={} total_ms={}",
                request_id,
                session_label,
                message_id,
                cancel_start.elapsed().as_millis()
            ));
        }
    } else {
        crate::logging::warn(&format!(
            "SERVER_INTERRUPT_CANCEL_NO_LOCAL_TASK request_id={:?} session={} control_session={} client_processing={} message_id={:?}; signalling session cancel handle anyway",
            request_id,
            session_label,
            session_control.session_id,
            *state.client_is_processing,
            *state.message_id
        ));
        // Nothing is running anywhere for this session, so there is no turn to
        // interrupt and arming the signal can only harm the *next* one: the
        // deferred reset below runs 500ms later, and a message sent inside
        // that window starts with the cancel flag already set and dies
        // immediately, with no reply and no error. Report the interrupt and
        // stop. Sessions whose turn is owned by another connection still take
        // the signalling path, since the registry sees those turns.
        if !crate::turn_cancel_registry::has_active_turn(&session_control.session_id) {
            crate::logging::info(&format!(
                "SERVER_INTERRUPT_CANCEL_IDLE_NOOP request_id={:?} session={}",
                request_id, session_label
            ));
            *state.client_is_processing = false;
            if client_event_tx.send(ServerEvent::Interrupted).is_err() {
                crate::logging::debug("Turn event recipient disconnected");
            }
            if let Some(message_id) = state.message_id.take()
                && client_event_tx
                    .send(ServerEvent::Done { id: message_id })
                    .is_err()
            {
                crate::logging::debug("Turn event recipient disconnected");
            }
            return;
        }
        let cancel_epoch = session_control.request_cancel();
        let reset_control = session_control.clone();
        tokio::spawn(async move {
            // The running turn is not owned by this connection (post-reload
            // recovery, server-initiated turn, or attach), so we cannot await
            // it. Clear the flag later so the *next* turn is not aborted by a
            // stale cancel, but only if no newer cancel fired in the meantime:
            // an unconditional reset here used to erase rapid repeated Esc
            // cancels before the busy turn observed them (issue #428).
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            reset_control.reset_cancel_if_epoch(cancel_epoch);
        });
        *state.client_is_processing = false;
        let status_session_id = state
            .session_id
            .take()
            .unwrap_or_else(|| session_control.session_id.clone());
        update_member_status(
            &status_session_id,
            "stopped",
            Some("cancelled".to_string()),
            swarm.members,
            swarm.swarms_by_id,
            Some(swarm.event_history),
            Some(swarm.event_counter),
            Some(swarm.event_tx),
        )
        .await;
        if client_event_tx.send(ServerEvent::Interrupted).is_err() {
            crate::logging::debug("Turn event recipient disconnected");
        }
        if let Some(message_id) = state.message_id.take() {
            if client_event_tx
                .send(ServerEvent::Done { id: message_id })
                .is_err()
            {
                crate::logging::debug("Turn event recipient disconnected");
            }
            crate::logging::info(&format!(
                "SERVER_INTERRUPT_CANCEL_EVENTS_EMITTED request_id={:?} session={} interrupted=true done_id={} total_ms={}",
                request_id,
                session_label,
                message_id,
                cancel_start.elapsed().as_millis()
            ));
        } else {
            crate::logging::info(&format!(
                "SERVER_INTERRUPT_CANCEL_EVENTS_EMITTED request_id={:?} session={} interrupted=true done_id=None total_ms={}",
                request_id,
                session_label,
                cancel_start.elapsed().as_millis()
            ));
        }
    }
}
