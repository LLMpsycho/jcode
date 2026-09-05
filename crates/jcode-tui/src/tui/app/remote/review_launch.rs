//! Correlate split controls without consuming an unrelated main-model turn.
use super::{App, DisplayMessage, finish_remote_split_launch};

fn retire(app: &mut App, id: u64) {
    app.pending_split_request_id = None;
    app.last_completed_split_request_id = Some(id);
}

pub(super) fn clear_launch(app: &mut App) {
    app.workspace_client.cancel_pending_split();
    finish_remote_split_launch(app);
    app.pending_split_request = false;
    app.pending_split_started_at = None;
    app.pending_split_startup_message = None;
    app.pending_split_parent_session_id = None;
    app.pending_split_prompt = None;
    app.pending_split_model_override = None;
    app.pending_split_role_selection = None;
    app.pending_split_provider_key_override = None;
    app.pending_split_label = None;
}

pub(in crate::tui::app) fn handle_error(app: &mut App, id: u64, message: &str) -> bool {
    if app.pending_split_request_id != Some(id) {
        return app.last_completed_split_request_id == Some(id)
            && app.current_message_id != Some(id);
    }
    let label = app
        .pending_split_label
        .clone()
        .unwrap_or_else(|| "Split".into());
    retire(app, id);
    clear_launch(app);
    app.push_display_message(DisplayMessage::error(format!(
        "Failed to launch {} session: {message}",
        label.to_lowercase()
    )));
    app.set_status_notice(format!("{label} launch failed"));
    true
}

pub(in crate::tui::app) fn accept_response(app: &mut App, id: u64) -> bool {
    if app
        .pending_split_request_id
        .is_some_and(|pending| id != pending)
        || app
            .last_completed_split_request_id
            .is_some_and(|completed| id <= completed)
    {
        return false;
    }
    // Existing prompt/workspace split paths also use this response handler.
    // Only dispatcher-owned requests participate in this completion fence.
    if app.pending_split_request_id == Some(id) {
        retire(app, id);
    }
    true
}

pub(in crate::tui::app) fn reset_connection(app: &mut App) {
    super::super::commands_review::cancel_automatic_reviews(app);
    if app.pending_split_request_id.take().is_some() {
        clear_launch(app);
        app.push_display_message(DisplayMessage::system(
            "Connection changed during a session launch. Its outcome is unknown; inspect sessions before retrying. It will not be replayed automatically.",
        ));
    }
    // Request IDs restart on each RemoteConnection. Never compare across sockets.
    app.last_completed_split_request_id = None;
}
