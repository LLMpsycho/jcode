//! Bounded client-side scheduling for reviews of successfully completed turns.
//!
//! Only the client that submitted a user turn arms these flags. Observing clients
//! and system continuations must not multiply review agents. This queue is not a
//! durable daemon scheduler: reconnecting or starting new work can discard it.

use super::{
    App, DisplayMessage, ReviewModelSelection, active_session_id, build_autojudge_startup_message,
    build_autoreview_startup_message, current_feedback_target_session_id,
    is_analysis_feedback_session_title, review_split_pending,
};
use std::time::Instant;

pub(in crate::tui::app) struct PendingAutomaticReview {
    source_session_id: String,
    parent_session_id: String,
    label: &'static str,
    startup_message: String,
    selection: ReviewModelSelection,
}

pub(in crate::tui::app) fn cancel_automatic_reviews(app: &mut App) {
    app.autoreview_after_current_turn = false;
    app.autojudge_after_current_turn = false;
    app.pending_automatic_reviews.clear();
}

/// Called only after accepting the matching main-turn Done, never a control
/// reply, interrupted turn, provider error, or synthetic startup completion.
pub(in crate::tui::app) fn queue_completed_turn_reviews(app: &mut App) {
    let review_armed = std::mem::take(&mut app.autoreview_after_current_turn);
    let judge_armed = std::mem::take(&mut app.autojudge_after_current_turn);
    if !app.is_remote || app.is_replay || (!review_armed && !judge_armed) {
        return;
    }
    app.pending_automatic_reviews.clear();
    if is_analysis_feedback_session_title(app.session.title.as_deref()) {
        return;
    }
    let source_session_id = active_session_id(app);
    let parent_session_id = current_feedback_target_session_id(app);
    for (enabled, label) in [
        (review_armed && app.autoreview_enabled, "Autoreview"),
        (judge_armed && app.autojudge_enabled, "Autojudge"),
    ] {
        if !enabled {
            continue;
        }
        let selection = match ReviewModelSelection::for_role(app, label) {
            Ok(selection) => selection,
            Err(error) => {
                app.push_display_message(DisplayMessage::error(format!(
                    "Failed to queue {}: {error}",
                    label.to_lowercase()
                )));
                continue;
            }
        };
        let startup_message = if label == "Autoreview" {
            build_autoreview_startup_message(&parent_session_id)
        } else {
            build_autojudge_startup_message(&parent_session_id)
        };
        app.pending_automatic_reviews
            .push_back(PendingAutomaticReview {
                source_session_id: source_session_id.clone(),
                parent_session_id: parent_session_id.clone(),
                label,
                startup_message,
                selection,
            });
    }
}

/// Use the existing serialized split handshake. A manual launch already in
/// flight owns its metadata until its response arrives; automatic jobs wait.
pub(in crate::tui::app) fn stage_next_automatic_review(app: &mut App) {
    if !app.is_remote || app.is_replay || app.is_processing || review_split_pending(app) {
        return;
    }
    while let Some(review) = app.pending_automatic_reviews.pop_front() {
        let enabled = if review.label == "Autoreview" {
            app.autoreview_enabled
        } else {
            app.autojudge_enabled
        };
        if !enabled || review.source_session_id != active_session_id(app) {
            continue;
        }
        app.pending_split_parent_session_id = Some(review.parent_session_id);
        app.pending_split_startup_message = Some(review.startup_message);
        app.pending_split_model_override = None;
        app.pending_split_provider_key_override = None;
        app.pending_split_role_selection = Some(review.selection);
        app.pending_split_label = Some(review.label.to_owned());
        app.pending_split_started_at = Some(Instant::now());
        app.pending_split_request = true;
        app.set_status_notice(format!("{} queued", review.label));
        break;
    }
}
