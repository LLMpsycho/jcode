//! One connected command path for Enter, transcript input and other submitters.
use super::super::commands_review;
use super::{
    App, DisplayMessage, RemoteConnection, begin_remote_split_launch, finish_remote_split_launch,
};
use crate::protocol::FeatureToggle;

pub(super) async fn dispatch(app: &mut App, remote: &mut RemoteConnection, input: &str) -> bool {
    let mut words = input.split_whitespace();
    let Some(command) = words.next() else {
        return false;
    };
    let (label, automatic, judge) = match command {
        "/autoreview" => ("Autoreview", true, false),
        "/autojudge" => ("Autojudge", true, true),
        "/review" => ("Review", false, false),
        "/judge" => ("Judge", false, true),
        _ => return false,
    };
    // A bare command requests the default action; no argument is an empty token.
    let arg = words.next().unwrap_or("");
    let usage = if automatic {
        format!("Usage: {command} [on|off|status|now]")
    } else {
        format!("Usage: {command}")
    };
    if words.next().is_some() || (!automatic && !arg.is_empty()) {
        app.push_display_message(DisplayMessage::error(usage));
        return true;
    }
    if automatic && matches!(arg, "" | "status") {
        let status = if judge {
            commands_review::autojudge_status_message(app)
        } else {
            commands_review::autoreview_status_message(app)
        };
        app.push_display_message(DisplayMessage::system(status));
        return true;
    }
    if automatic && matches!(arg, "on" | "off") {
        let enabled = arg == "on";
        let feature = if judge {
            FeatureToggle::Autojudge
        } else {
            FeatureToggle::Autoreview
        };
        if let Err(error) = remote.set_feature(feature, enabled).await {
            // A control failure must not become a failed main-model turn.
            app.push_display_message(DisplayMessage::error(format!(
                "Failed to change {}: {error}",
                label.to_lowercase()
            )));
            return true;
        }
        if judge {
            app.set_autojudge_feature_enabled(enabled);
        } else {
            app.set_autoreview_feature_enabled(enabled);
        }
        app.set_status_notice(format!("{label}: {}", if enabled { "ON" } else { "OFF" }));
        app.push_display_message(DisplayMessage::system(format!(
            "{label} {} for this session.",
            if enabled { "enabled" } else { "disabled" }
        )));
        return true;
    }
    if automatic && arg != "now" {
        app.push_display_message(DisplayMessage::error(usage));
        return true;
    }
    let parent = commands_review::current_feedback_target_session_id(app);
    let startup = match (automatic, judge) {
        (true, false) => commands_review::build_autoreview_startup_message(&parent),
        (true, true) => commands_review::build_autojudge_startup_message(&parent),
        (false, false) => commands_review::build_review_startup_message(&parent),
        (false, true) => commands_review::build_judge_startup_message(&parent),
    };
    if let Err(error) = commands_review::queue_review_spawn_remote(app, label, parent, startup) {
        app.push_display_message(DisplayMessage::error(format!(
            "Failed to queue {}: {error}",
            label.to_lowercase()
        )));
        return true;
    }
    dispatch_pending_split(app, remote).await;
    true
}

pub(super) async fn dispatch_pending_split(app: &mut App, remote: &mut RemoteConnection) -> bool {
    if !app.pending_split_request || app.is_processing {
        return false;
    }
    app.pending_split_request = false;
    let flow_label = app
        .pending_split_label
        .clone()
        .unwrap_or_else(|| "Split".to_string());
    begin_remote_split_launch(app, &flow_label);
    let split_result = remote.split().await;
    if let Ok(id) = &split_result {
        app.pending_split_request_id = Some(*id);
    }
    if let Err(error) = split_result {
        app.workspace_client.cancel_pending_split();
        finish_remote_split_launch(app);
        let had_startup = app.pending_split_startup_message.take().is_some();
        app.pending_split_parent_session_id = None;
        let had_prompt = app.pending_split_prompt.take().is_some();
        let label = app.pending_split_label.take();
        app.pending_split_model_override = None;
        app.pending_split_role_selection = None;
        app.pending_split_provider_key_override = None;
        let flow_label = label.unwrap_or(flow_label);
        app.push_display_message(DisplayMessage::error(format!(
            "Failed to launch {} session: {}",
            flow_label.to_lowercase(),
            error
        )));
        if had_startup || had_prompt {
            app.set_status_notice(format!("{} launch failed", flow_label));
        }
    }
    true
}
