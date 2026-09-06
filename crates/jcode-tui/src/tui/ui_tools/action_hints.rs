pub(super) fn infer_bg_action_from_intent_for_display(
    intent: Option<&str>,
) -> Option<&'static str> {
    let intent = intent?.trim().to_ascii_lowercase();
    if intent.is_empty() {
        return None;
    }

    if intent.contains("wait") || intent.contains("await") {
        Some("wait")
    } else if intent.contains("tail") {
        Some("tail")
    } else if intent.contains("output") || intent.contains("log") {
        Some("output")
    } else if intent.contains("status") || intent.contains("progress") || intent.contains("check") {
        Some("status")
    } else if intent.contains("cancel") || intent.contains("stop") {
        Some("cancel")
    } else if intent.contains("clean") {
        Some("cleanup")
    } else if intent.contains("list") || intent.contains("show") {
        Some("list")
    } else {
        None
    }
}

pub(super) fn infer_selfdev_action_from_display_text(text: Option<&str>) -> Option<&'static str> {
    let text = text?.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }

    if text.contains("build-reload") || text.contains("build_reload") {
        Some("build-reload")
    } else if text.contains("reload") || text.contains("restart") {
        Some("reload")
    } else if text.contains("build") || text.contains("compile") {
        Some("build")
    } else if text.contains("test") || text.contains("check") || text.contains("validate") {
        Some("test")
    } else if text.contains("cancel") || text.contains("stop") {
        Some("cancel-build")
    } else if text.contains("status") || text.contains("queue") || text.contains("progress") {
        Some("status")
    } else if text.contains("socket") {
        Some("socket-info")
    } else if text.contains("enter") {
        Some("enter")
    } else {
        None
    }
}
