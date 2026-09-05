use super::{InlineInteractiveLayout, InlineInteractiveSchema, InlineInteractiveState, PickerAction};
use crate::protocol::AdvisorRequest;

pub(super) fn schema() -> InlineInteractiveSchema {
    InlineInteractiveSchema {
        layout: InlineInteractiveLayout::ThreeColumn,
        primary_label: "ADVISOR MODEL / EFFORT",
        secondary_label: "PROVIDER",
        secondary_preview_label: "PROVIDER",
        tertiary_label: "METHOD",
        preview_submit_hint: "  ↵ select",
        active_submit_hint: "  ↑↓ · type to filter · ↵ select · Esc cancel",
        shows_default_shortcut_hint: false,
        preview_activation_column: 0,
    }
}

impl InlineInteractiveState {
    pub fn is_advisor_picker(&self) -> bool {
        !self.entries.is_empty()
            && self.entries.iter().all(|entry| matches!(entry.action, PickerAction::Advisor(_)))
    }
}

pub(super) fn request_bytes(request: Option<&AdvisorRequest>) -> usize {
    let (selection, effort) = match request {
        Some(AdvisorRequest::SelectModel { selection, reasoning_effort }) => (Some(selection), reasoning_effort.as_ref()),
        Some(AdvisorRequest::ModelOptions { selection }) => (selection.as_ref(), None),
        Some(AdvisorRequest::Dismiss { note_id } | AdvisorRequest::Acknowledge { note_id }) => return note_id.capacity(),
        _ => return 0,
    };
    selection.map(|selection| selection.model.capacity() + selection.provider_label.capacity()
        + selection.api_method.capacity() + selection.detail.capacity()).unwrap_or(0)
        + effort.map(|effort| effort.capacity()).unwrap_or(0)
}
