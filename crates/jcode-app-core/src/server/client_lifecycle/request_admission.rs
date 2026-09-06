//! Request admission.

use super::*;

pub(super) fn required_subscribe_working_dir(
    working_dir: Option<&str>,
) -> std::result::Result<&str, String> {
    let working_dir = working_dir
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .ok_or_else(|| "Subscribe requires the client's working directory".to_string())?;
    if !Path::new(working_dir).is_absolute() {
        return Err("Subscribe working_dir must be an absolute path".to_string());
    }
    Ok(working_dir)
}

pub(super) fn initial_subscribe_working_dir(
    request: &Request,
) -> std::result::Result<String, String> {
    match request {
        Request::Subscribe { working_dir, .. } => {
            required_subscribe_working_dir(working_dir.as_deref()).map(str::to_string)
        }
        _ => Err(
            "Client must Subscribe with a working_dir before sending stateful requests".to_string(),
        ),
    }
}

pub(super) fn initial_subscribe_terminal_env(request: &Request) -> Vec<(String, String)> {
    match request {
        Request::Subscribe { terminal_env, .. } => terminal_env.clone(),
        _ => Vec::new(),
    }
}

pub(super) fn reject_if_agent_busy_for_request(
    request_id: u64,
    request_kind: &'static str,
    client_session_id: &str,
    client_is_processing: bool,
    agent: &Arc<Mutex<Agent>>,
    client_event_tx: &mpsc::UnboundedSender<ServerEvent>,
) -> bool {
    if agent.try_lock().is_ok() {
        return false;
    }

    send_agent_busy_error(
        request_id,
        request_kind,
        client_session_id,
        client_is_processing,
        client_event_tx,
    );
    true
}

pub(super) fn send_agent_busy_error(
    request_id: u64,
    request_kind: &'static str,
    client_session_id: &str,
    client_is_processing: bool,
    client_event_tx: &mpsc::UnboundedSender<ServerEvent>,
) {
    crate::logging::event_warn(
        "SERVER_REQUEST_BUSY_AGENT_REJECTED",
        vec![
            ("request_id", request_id.to_string()),
            ("request_kind", request_kind.to_string()),
            ("session_id", client_session_id.to_string()),
            ("client_processing", client_is_processing.to_string()),
            ("reason", "agent_busy".to_string()),
        ],
    );
    if (client_event_tx.send(ServerEvent::Error {
        id: request_id,
        message: format!(
            "Cannot handle {request_kind} while the session is busy. Try again after the current turn finishes."
        ),
        retry_after_secs: Some(1),
    })).is_err() {
 crate::logging::debug("Event recipient disconnected before delivery");
}
}
