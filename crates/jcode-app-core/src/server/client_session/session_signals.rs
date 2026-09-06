//! Session signals.

use super::*;

pub(super) fn mark_remote_reload_started(request_id: &str) {
    crate::server::write_reload_state(
        request_id,
        jcode_build_meta::version(),
        crate::server::ReloadPhase::Starting,
        None,
    );
}

pub(super) async fn rename_shutdown_signal(
    shutdown_signals: &Arc<RwLock<HashMap<String, InterruptSignal>>>,
    old_session_id: &str,
    new_session_id: &str,
) {
    if old_session_id == new_session_id {
        return;
    }

    let mut signals = shutdown_signals.write().await;
    if let Some(signal) = signals.remove(old_session_id) {
        signals.insert(new_session_id.to_string(), signal);
    }
    drop(signals);
    rename_background_tool_signal(old_session_id, new_session_id);
    // In-flight turns are registered in the process-global cancel registry by
    // session id. Attaching to / resuming a session renames it underneath a
    // still-streaming turn, so the registration must follow, or a later Esc
    // finds no active-turn signal for the new id and the model keeps
    // generating (issue #732, regression of issue #428).
    crate::turn_cancel_registry::rename_active_turns(old_session_id, new_session_id);
}

pub(super) fn log_ignored_subscribe_working_dir(session_id: &str, current: &str, reported: &str) {
    crate::logging::warn(&format!(
        "Ignoring subscribe working_dir {} for session {}: it is the home directory while the session is already bound to {} (issue #481)",
        reported, session_id, current
    ));
}

pub(super) async fn subscribe_should_mark_ready(
    client_session_id: &str,
    swarm_members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
) -> bool {
    let members = swarm_members.read().await;
    members
        .get(client_session_id)
        .is_none_or(|member| member.status != "running")
}

pub(in crate::server) fn session_was_interrupted_by_reload(agent: &Agent) -> bool {
    let messages = agent.messages();
    let Some(last) = messages.last() else {
        return false;
    };

    last.content.iter().any(|block| match block {
        ContentBlock::Text { text, .. } => {
            text.ends_with("[generation interrupted - server reloading]")
        }
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            content == "Reload initiated. Process restarting..."
                || (is_error.unwrap_or(false)
                    && (content.contains("interrupted by server reload")
                        || content.contains("Skipped - server reloading")))
        }
        _ => false,
    })
}
