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
        Request::Subscribe {
            working_dir,
            continue_on_disconnect,
            ..
        } => validated_subscribe_working_dir(working_dir.as_deref(), *continue_on_disconnect)
            .map(str::to_string),
        _ => Err(
            "Client must Subscribe with a working_dir before sending stateful requests".to_string(),
        ),
    }
}

/// A reattachment names an existing session, not a new client working directory.
/// Resolve an omitted cwd before provisional initialization, never from the
/// daemon/bridge process cwd. Idle empty sessions may exist only in memory.
pub(super) async fn resolve_target_subscribe_working_dir(
    request: &mut Request,
    sessions: &SessionAgents,
    members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
) -> std::result::Result<(), String> {
    let Request::Subscribe {
        working_dir,
        target_session_id: Some(target),
        ..
    } = request
    else {
        return Ok(());
    };
    if working_dir.is_some() {
        return Ok(());
    }
    let live = sessions.read().await.get(target).cloned();
    let resolved = if let Some(live) = live {
        let idle_cwd = live
            .try_lock()
            .map_or_else(
                |_| {
                    crate::logging::debug("Attach resolves busy session cwd from swarm metadata");
                    None
                },
                Some,
            )
            .and_then(|agent| agent.working_dir().map(str::to_string));
        if idle_cwd.is_some() {
            idle_cwd
        } else {
            // A generating Agent owns its mutex. The member records the same
            // session root, so attaching must not wait for the model turn.
            members
                .read()
                .await
                .get(target)
                .and_then(|member| member.working_dir.as_ref())
                .map(|path| path.to_string_lossy().into_owned())
        }
    } else {
        crate::session::Session::load_startup_stub(target)
            .map_or_else(
                |error| {
                    crate::logging::debug(&format!("Attach session stub unavailable: {error}"));
                    None
                },
                Some,
            )
            .and_then(|session| session.working_dir)
    };
    *working_dir = Some(resolved.ok_or_else(|| {
        format!("Unknown session '{target}' or session has no working directory")
    })?);
    Ok(())
}

pub(super) fn validated_subscribe_working_dir(
    working_dir: Option<&str>,
    remote_continuation: bool,
) -> std::result::Result<&str, String> {
    let working_dir = required_subscribe_working_dir(working_dir)?;
    if remote_continuation && !Path::new(working_dir).is_dir() {
        return Err(format!(
            "Remote working directory must exist and be a directory on the server: {working_dir}"
        ));
    }
    Ok(working_dir)
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
