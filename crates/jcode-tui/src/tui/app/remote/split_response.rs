//! Finish the shared split handshake and publish its prepared child session.
use super::{App, DisplayMessage, finish_remote_split_launch, spawn_in_new_terminal};
use crate::tui::app as app_mod;

pub(super) fn handle_split_response(
    app: &mut App,
    id: u64,
    new_session_id: String,
    new_session_name: String,
) -> bool {
    if !super::review_launch::accept_response(app, id) {
        return false;
    }
    if app.workspace_client.handle_split_response(&new_session_id) {
        finish_remote_split_launch(app);
        app.pending_split_request = false;
        app.pending_split_startup_message = None;
        app.pending_split_parent_session_id = None;
        app.pending_split_prompt = None;
        app.pending_split_model_override = None;
        app.pending_split_role_selection = None;
        app.pending_split_provider_key_override = None;
        app.pending_split_label = None;
        app.push_display_message(DisplayMessage::system(format!(
            "Added {} to workspace.",
            new_session_name,
        )));
        app.set_status_notice(format!("Workspace + {}", new_session_name));
        return false;
    }
    finish_remote_split_launch(app);
    app.pending_split_request = false;
    let startup_message = app.pending_split_startup_message.take();
    let parent_session_id_override = app.pending_split_parent_session_id.take();
    let startup_prompt = app.pending_split_prompt.take();
    app.pending_split_model_override = None;
    app.pending_split_provider_key_override = None;
    let role_selection = app.pending_split_role_selection.take();
    let split_label = app.pending_split_label.take();
    if let Some(startup_message) = startup_message {
        if let Err(error) = app_mod::commands::prepare_review_spawned_session(
            &new_session_id,
            startup_message,
            role_selection,
            split_label.clone().map(|label| label.to_ascii_lowercase()),
            parent_session_id_override,
        ) {
            app.push_display_message(DisplayMessage::error(format!(
                "Failed to prepare review session: {error}"
            )));
            app.set_status_notice("Review launch failed");
            return false;
        }
    } else if let Some(startup_prompt) = startup_prompt {
        App::save_startup_submission_for_session(
            &new_session_id,
            startup_prompt.content,
            startup_prompt.images,
        );
    }
    let exe = app_mod::launch_client_executable();
    let (cwd, socket) = app_mod::terminal_launch_context::resolve(&new_session_id);
    match spawn_in_new_terminal(&exe, &new_session_id, &cwd, socket.as_deref()) {
        Ok(true) => {
            if let Some(label) = split_label.as_deref() {
                app.push_display_message(DisplayMessage::system(format!(
                    "🔍 {} launched in {}.",
                    label, new_session_name,
                )));
                app.set_status_notice(format!("{} launched", label));
            } else {
                app.push_display_message(DisplayMessage::system(format!(
                    "✂ Split → {} (opened in new pane/window)",
                    new_session_name,
                )));
                app.set_status_notice(format!("Split → {}", new_session_name));
            }
        }
        Ok(false) => {
            if let Some(label) = split_label.as_deref() {
                app.push_display_message(DisplayMessage::system(format!(
                            "🔍 {} session {} created.\n\nNo terminal found. Resume manually:\n  jcode --resume {}",
                            label, new_session_name, new_session_id,
                        )));
                app.set_status_notice(format!("{} session created", label));
            } else {
                app.push_display_message(DisplayMessage::system(format!(
                    "✂ Split → {}\n\nNo terminal found. Resume manually:\n  jcode --resume {}",
                    new_session_name, new_session_id,
                )));
            }
        }
        Err(e) => {
            if let Some(label) = split_label.as_deref() {
                app.push_display_message(DisplayMessage::error(format!(
                            "{} session {} was created but failed to open a window: {}\n\nResume manually: jcode --resume {}",
                            label, new_session_name, e, new_session_id,
                        )));
                app.set_status_notice(format!("{} open failed", label));
            } else {
                app.push_display_message(DisplayMessage::error(format!(
                            "Split created {} but failed to open window: {}\n\nResume manually: jcode --resume {}",
                            new_session_name, e, new_session_id,
                        )));
            }
        }
    }
    false
}
