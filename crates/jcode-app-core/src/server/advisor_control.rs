use crate::advisor::{AdvisorManager, AdvisorNoteDisposition};
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
    let config = crate::advisor::config_for_current_session();
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
            let _ = events.send(ServerEvent::AdvisorResult { id, result });
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
                let _ = events.send(ServerEvent::AdvisorResult { id, result });
            });
        }
        return;
    }
    let is_status = matches!(request, AdvisorRequest::Status);
    let mut result = control_request(&manager, session, &config, request);
    if is_status {
        result.model_settings = Some(match agent.try_lock() {
            Ok(agent) => manager.model_settings(session, agent.provider_handle().as_ref(), &config),
            Err(_) => manager.saved_model_settings(session, &config),
        });
    }
    let _ = events.send(ServerEvent::AdvisorResult { id, result });
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
        result.message = result.error.clone().unwrap_or_default();
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
                    "Advisor: {} ({:?}); {} unresolved blocking note(s), {} retained note(s); {}",
                    if enabled { "on" } else { "off" },
                    snapshot.status,
                    snapshot.unresolved_blocking_notes,
                    manager.notes(session).len(),
                    snapshot.last_error.as_deref().unwrap_or("no error")
                ),
                None => format!(
                    "Advisor: {} (idle); no retained notes",
                    if enabled { "on" } else { "off" }
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
