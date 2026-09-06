use crate::advisor::{AdvisorManager, AdvisorNoteDisposition, roster};
use crate::agent::Agent;
use crate::config::AdvisorConfig;
use crate::protocol::{AdvisorControlResult, AdvisorRequest, ServerEvent};
use crate::provider::Provider;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

pub(super) fn handle_advisor(
    id: u64,
    request: AdvisorRequest,
    session: &str,
    agent: &Arc<Mutex<Agent>>,
    events: &mpsc::UnboundedSender<ServerEvent>,
) {
    handle_with_manager(
        id,
        request,
        session,
        agent,
        events,
        crate::advisor::advisor_manager(),
    );
}

fn handle_with_manager(
    id: u64,
    request: AdvisorRequest,
    session: &str,
    agent: &Arc<Mutex<Agent>>,
    events: &mpsc::UnboundedSender<ServerEvent>,
    manager: Arc<AdvisorManager>,
) {
    let global = crate::advisor::config_for_current_session();
    if matches!(request, AdvisorRequest::Disable) {
        let outcome = roster::disable_all(&manager, session, &global);
        match outcome {
            Ok(()) => {
                if (events.send(ServerEvent::AdvisorResult {
                    id,
                    result: AdvisorControlResult {
                        message: "All advisors disabled for this session.".into(),
                        ..Default::default()
                    },
                }))
                .is_err()
                {
                    crate::logging::debug("Event recipient disconnected before delivery");
                }
            }
            Err(error) => send_error(id, events, error.to_string()),
        }
        return;
    }
    let working_dir = match agent.try_lock() {
        Ok(agent) => agent.working_dir().map(std::path::PathBuf::from),
        Err(_) => {
            // The owner roster cache retains project configuration during an active turn.
            crate::logging::debug(
                "Advisor control using cached project roster while session is busy",
            );
            None
        }
    };
    let config = match roster::config_for_owner(session, &global, working_dir.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            send_error(id, events, error.to_string());
            return;
        }
    };
    handle_with_config(id, request, session, agent, events, manager, config);
}

fn handle_with_config(
    id: u64,
    request: AdvisorRequest,
    owner_session: &str,
    agent: &Arc<Mutex<Agent>>,
    events: &mpsc::UnboundedSender<ServerEvent>,
    manager: Arc<AdvisorManager>,
    config: AdvisorConfig,
) {
    if matches!(request, AdvisorRequest::Enable) {
        if let Err(error) = roster::enable_owner(&manager, owner_session) {
            send_error(id, events, error.to_string());
            return;
        }
    }
    let (session_key, mut config, request) = match target_request(&config, owner_session, request) {
        Ok(target) => target,
        Err(error) => {
            send_error(id, events, error.to_string());
            return;
        }
    };
    let session = session_key.as_str();
    manager.resume(owner_session);
    config.enabled &= roster::owner_enabled(&manager, owner_session);
    manager.resume(session);
    if matches!(request, AdvisorRequest::ModelOptions { .. }) {
        // Catalog reads and effort previews must remain available during a
        // primary turn, including when this connection attached to a busy
        // session whose provider differs from the connection template.
        let result = match super::session_provider::for_agent(agent) {
            Some(provider) => {
                model_request(&manager, session, provider.as_ref(), &config, request, 0)
            }
            None => {
                let message = "Advisor model catalog is unavailable for this session; retry /advisor when the current turn finishes.".to_string();
                AdvisorControlResult {
                    error: Some(message.clone()),
                    message,
                    ..AdvisorControlResult::default()
                }
            }
        };
        if (events.send(ServerEvent::AdvisorResult { id, result })).is_err() {
            crate::logging::debug("Event recipient disconnected before delivery");
        }
        return;
    }
    if matches!(
        request,
        AdvisorRequest::SelectModel { .. }
            | AdvisorRequest::UsePrimary
            | AdvisorRequest::ModelOptions { .. }
    ) {
        let selection_id = if matches!(
            request,
            AdvisorRequest::SelectModel { .. } | AdvisorRequest::UsePrimary
        ) {
            manager.begin_model_selection(session)
        } else {
            0
        };
        if let Ok(agent) = agent.try_lock() {
            let provider = agent.provider_handle();
            drop(agent);
            let result = model_request(
                &manager,
                session,
                provider.as_ref(),
                &config,
                request,
                selection_id,
            );
            if (events.send(ServerEvent::AdvisorResult { id, result })).is_err() {
                crate::logging::debug("Event recipient disconnected before delivery");
            }
        } else {
            // Waiting for a primary turn must not hold up cancel, acknowledge,
            // dismiss, or disable requests on this connection. Capture the
            // current session's Agent, rather than a connection template that
            // may refer to a previously attached session.
            let agent = Arc::clone(agent);
            let events = events.clone();
            let session = session.to_string();
            tokio::spawn(async move {
                let provider = agent.lock().await.provider_handle();
                let result = model_request(
                    &manager,
                    &session,
                    provider.as_ref(),
                    &config,
                    request,
                    selection_id,
                );
                if (events.send(ServerEvent::AdvisorResult { id, result })).is_err() {
                    crate::logging::debug("Event recipient disconnected before delivery");
                }
            });
        }
        return;
    }
    let is_status = matches!(request, AdvisorRequest::Status);
    let mut result = if config.roster.is_empty() {
        control_request(&manager, session, &config, request)
    } else {
        roster_control_request(&manager, owner_session, &config, request)
    };
    if is_status {
        result.model_settings = Some(match agent.try_lock() {
            Ok(agent) => manager.model_settings(session, agent.provider_handle().as_ref(), &config),
            Err(_) => manager.saved_model_settings(session, &config),
        });
    }
    if (events.send(ServerEvent::AdvisorResult { id, result })).is_err() {
        crate::logging::debug("Event recipient disconnected before delivery");
    }
}

fn send_error(id: u64, events: &mpsc::UnboundedSender<ServerEvent>, message: String) {
    let message = crate::message::redact_secrets(&message);
    if (events.send(ServerEvent::AdvisorResult {
        id,
        result: AdvisorControlResult {
            error: Some(message.clone()),
            message,
            ..Default::default()
        },
    }))
    .is_err()
    {
        crate::logging::debug("Event recipient disconnected before delivery");
    }
}

fn target_request(
    config: &AdvisorConfig,
    owner: &str,
    request: AdvisorRequest,
) -> anyhow::Result<(String, AdvisorConfig, AdvisorRequest)> {
    let requested = match request {
        AdvisorRequest::ForAdvisor { name, request } => {
            anyhow::ensure!(
                !matches!(*request, AdvisorRequest::ForAdvisor { .. }),
                "nested advisor targeting is not supported"
            );
            Some((name, *request))
        }
        request
            if matches!(
                request,
                AdvisorRequest::ModelOptions { .. }
                    | AdvisorRequest::SelectModel { .. }
                    | AdvisorRequest::UsePrimary
            ) =>
        {
            let entries = roster::entries(config)?;
            let name = entries
                .iter()
                .find(|entry| entry.name == roster::DEFAULT_ADVISOR)
                .or_else(|| entries.first())
                .ok_or_else(|| anyhow::anyhow!("advisor roster is empty"))?
                .name
                .clone();
            Some((name, request))
        }
        request => {
            roster::entries(config)?;
            return Ok((owner.into(), config.clone(), request));
        }
    };
    let (name, request) = requested.ok_or_else(|| anyhow::anyhow!("advisor target missing"))?;
    let entry = roster::entry(config, &name)?;
    Ok((
        roster::runtime_session_key(owner, &name),
        entry.config,
        request,
    ))
}

fn roster_control_request(
    manager: &AdvisorManager,
    owner: &str,
    config: &AdvisorConfig,
    request: AdvisorRequest,
) -> AdvisorControlResult {
    let outcome = (|| -> anyhow::Result<String> {
        let entries = roster::entries(config)?;
        if matches!(request, AdvisorRequest::Enable) {
            roster::enable_owner(manager, owner)?;
        }
        let note_id = match &request {
            AdvisorRequest::Acknowledge { note_id } | AdvisorRequest::Dismiss { note_id } => {
                Some(note_id)
            }
            _ => None,
        };
        let mut messages = Vec::new();
        for mut entry in entries {
            entry.config.enabled &= roster::owner_enabled(manager, owner);
            let key = roster::runtime_session_key(owner, &entry.name);
            manager.resume(&key);
            if let Some(note_id) = note_id {
                if !manager.notes(&key).iter().any(|note| note.id == *note_id) {
                    continue;
                }
            }
            let result = control_request(manager, &key, &entry.config, request.clone());
            anyhow::ensure!(result.error.is_none(), "{}", result.message);
            messages.push(format!("{}: {}", entry.name, result.message));
        }
        anyhow::ensure!(!messages.is_empty(), "advisor note was not found");
        Ok(messages.join("\n"))
    })();
    match outcome {
        Ok(message) => AdvisorControlResult {
            message,
            ..Default::default()
        },
        Err(error) => {
            let message = crate::message::redact_secrets(&error.to_string());
            AdvisorControlResult {
                error: Some(message.clone()),
                message,
                ..Default::default()
            }
        }
    }
}

fn model_request(
    manager: &AdvisorManager,
    session: &str,
    provider: &dyn Provider,
    config: &AdvisorConfig,
    request: AdvisorRequest,
    selection_id: u64,
) -> AdvisorControlResult {
    let mut result = AdvisorControlResult::default();
    let outcome = match request {
        AdvisorRequest::ModelOptions { selection } => manager
            .model_options(session, provider, config, selection.as_ref())
            .map(|options| {
                result.message = "Choose an advisor model using your existing Jcode login.".into();
                result.model_options = Some(options);
            }),
        AdvisorRequest::SelectModel {
            selection,
            reasoning_effort,
        } => manager
            .select_model(
                session,
                provider,
                config,
                selection,
                reasoning_effort,
                selection_id,
            )
            .map(|settings| {
                result.message = format!(
                    "Advisor enabled for this session: {}.",
                    settings_label(&settings)
                );
            }),
        AdvisorRequest::UsePrimary => manager
            .use_primary_model(session, provider, config, selection_id)
            .map(|_| {
                result.message =
                    "Advisor enabled and following the primary model and effort for this session."
                        .into();
            }),
        _ => Err(anyhow::anyhow!("invalid advisor model request")),
    };
    result.model_settings = Some(manager.model_settings(session, provider, config));
    if let Err(error) = outcome {
        result.error = Some(crate::message::redact_secrets(&error.to_string()));
        result.message = result.error.clone().unwrap_or_else(String::new);
    }
    result
}

fn settings_label(settings: &crate::protocol::AdvisorModelSettings) -> String {
    let model = settings
        .selection
        .as_ref()
        .map(|route| {
            format!(
                "{} via {} ({})",
                route.model, route.provider_label, route.api_method
            )
        })
        .unwrap_or_else(|| "primary model".into());
    match settings.reasoning_effort.as_deref() {
        Some(effort) => format!("{model}, effort {effort}"),
        None => model,
    }
}

fn control_request(
    manager: &AdvisorManager,
    session: &str,
    config: &AdvisorConfig,
    request: AdvisorRequest,
) -> AdvisorControlResult {
    let is_status = matches!(request, AdvisorRequest::Status);
    let outcome = match request {
        AdvisorRequest::Status => {
            let enabled = manager.is_enabled(session, config.enabled);
            Ok(match manager.snapshot(session) {
                Some(snapshot) => format!(
                    "Advisor: {} ({:?}); {} unresolved blocking note(s), {} retained note(s); reviews {}/{}; context {} message(s); {} suppressed; {}",
                    if enabled { "on" } else { "off" },
                    snapshot.status,
                    snapshot.unresolved_blocking_notes,
                    manager.notes(session).len(),
                    snapshot.cursor,
                    config.max_reviews_per_session,
                    snapshot.history_messages,
                    snapshot.suppressed_notes,
                    snapshot.last_error.as_deref().unwrap_or("no error")
                ),
                None => format!(
                    "Advisor: {} (idle); no retained notes; reviews 0/{}; context 0 message(s)",
                    if enabled { "on" } else { "off" },
                    config.max_reviews_per_session
                ),
            })
        }
        AdvisorRequest::Inspect => {
            let notes = manager.notes(session);
            Ok(if notes.is_empty() {
                "Advisor has no retained notes.".into()
            } else {
                notes
                    .into_iter()
                    .map(|note| {
                        format!(
                            "{} [{:?}/{:?}] {}\nRecommended: {}{}",
                            note.id,
                            note.severity,
                            note.disposition,
                            note.summary,
                            note.recommended_action,
                            if note.evidence.is_empty() {
                                String::new()
                            } else {
                                format!("\nEvidence: {}", note.evidence.join(" | "))
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
        }
        AdvisorRequest::Dismiss { note_id } => manager
            .resolve_note(session, &note_id, AdvisorNoteDisposition::Dismissed)
            .and_then(|found| {
                anyhow::ensure!(found, "Advisor note {note_id} was not found.");
                Ok(format!("Dismissed advisor note {note_id}."))
            }),
        AdvisorRequest::Acknowledge { note_id } => manager
            .resolve_note(session, &note_id, AdvisorNoteDisposition::Acknowledged)
            .and_then(|found| {
                anyhow::ensure!(found, "Advisor note {note_id} was not found.");
                Ok(format!("Acknowledged advisor note {note_id}."))
            }),
        AdvisorRequest::Enable => manager
            .set_enabled(session, true)
            .map(|()| "Advisor enabled for this session.".into()),
        AdvisorRequest::Disable => manager
            .set_enabled(session, false)
            .map(|()| "Advisor disabled for this session; future risky tools are released.".into()),
        _ => Err(anyhow::anyhow!("Invalid advisor control request.")),
    };
    let (message, error) = match outcome {
        Ok(message) => (message, None),
        Err(error) => {
            let message = crate::message::redact_secrets(&error.to_string());
            (message.clone(), Some(message))
        }
    };
    let message = if is_status {
        format!("{message}; {}", manager.model_summary(session, config))
    } else {
        message
    };
    AdvisorControlResult {
        message,
        error,
        ..AdvisorControlResult::default()
    }
}

#[cfg(test)]
#[path = "advisor_control_tests.rs"]
mod tests;
