//! Session advisor controls use the server's authenticated model catalog. The
//! primary model picker and its defaults are deliberately separate state.
use super::App;
use crate::protocol::{AdvisorControlResult, AdvisorModelOptions, AdvisorRequest};
use crate::provider::RouteSelection;
use crate::tui::backend::RemoteConnection;
use crate::tui::{InlineInteractiveState, PickerAction, PickerEntry, PickerKind, PickerOption};
use jcode_tui_messages::DisplayMessage;

#[derive(Default)]
pub(super) struct AdvisorPickerState {
    pending: Option<AdvisorRequest>,
    request_id: Option<u64>,
    session_id: Option<String>,
    in_flight: std::collections::BTreeMap<u64, AdvisorInFlight>,
}

struct AdvisorInFlight {
    session_id: Option<String>,
    opens_picker: bool,
}

pub(super) fn command(input: &str) -> Option<Result<AdvisorRequest, &'static str>> {
    let mut words = input.split_whitespace();
    if words.next()? != "/advisor" {
        return None;
    }
    let action = words.next();
    let request = match action {
        None | Some("model" | "models") => AdvisorRequest::ModelOptions { selection: None },
        Some("inherit") => AdvisorRequest::UsePrimary,
        Some("status") => AdvisorRequest::Status,
        Some("inspect") => AdvisorRequest::Inspect,
        Some("on") => AdvisorRequest::Enable,
        Some("off") => AdvisorRequest::Disable,
        Some("dismiss") => AdvisorRequest::Dismiss {
            note_id: words.next().unwrap_or("").into(),
        },
        Some("ack") => AdvisorRequest::Acknowledge {
            note_id: words.next().unwrap_or("").into(),
        },
        _ => return Some(Err(usage())),
    };
    if words.next().is_some()
        || matches!(&request, AdvisorRequest::Dismiss { note_id } | AdvisorRequest::Acknowledge { note_id } if note_id.is_empty())
    {
        return Some(Err(usage()));
    }
    Some(Ok(request))
}

fn usage() -> &'static str {
    "Usage: /advisor (model + effort), /advisor inherit|status|inspect|dismiss <id>|ack <id>|on|off"
}

fn entry(
    name: String,
    provider: String,
    method: String,
    detail: String,
    request: Option<AdvisorRequest>,
    current: bool,
) -> PickerEntry {
    PickerEntry {
        name,
        options: vec![PickerOption {
            provider,
            api_method: method,
            available: request.is_some(),
            detail,
            estimated_reference_cost_micros: None,
        }],
        action: PickerAction::Advisor(request),
        selected_option: 0,
        is_current: current,
        is_default: false,
        is_favorite: false,
        recommended: false,
        recommendation_rank: usize::MAX,
        usage_score: 0,
        old: false,
        created_date: None,
        effort: None,
    }
}

impl App {
    pub(super) fn cancel_advisor_picker(&mut self) {
        self.advisor_picker.pending = None;
        self.advisor_picker.request_id = None;
        self.advisor_picker.session_id = None;
        // Retain request correlation until the reply arrives, even when a
        // session switch cancels the picker. A late generic Error must never
        // be mistaken for a failure of the new session's main turn.
        if self
            .inline_interactive_state
            .as_ref()
            .is_some_and(|picker| picker.is_advisor_picker())
        {
            self.inline_interactive_state = None;
        }
    }

    pub(super) fn disconnect_advisor_picker(&mut self) {
        let had_requests = self.advisor_picker.pending.is_some()
            || !self.advisor_picker.in_flight.is_empty();
        self.cancel_advisor_picker();
        self.advisor_picker.in_flight.clear();
        if had_requests {
            self.push_display_message(DisplayMessage::system(
                "Advisor connection interrupted. After reconnecting, use /advisor status to check saved settings or /advisor to reopen the picker.",
            ));
        }
    }

    fn show_advisor_entries(&mut self, entries: Vec<PickerEntry>) {
        let selected = entries
            .iter()
            .position(|entry| entry.is_current)
            .unwrap_or(0);
        self.inline_view_state = None;
        self.pending_model_picker_load = None;
        self.inline_interactive_state = Some(InlineInteractiveState {
            kind: PickerKind::Model,
            filtered: (0..entries.len()).collect(),
            entries,
            selected,
            column: 0,
            filter: String::new(),
            preview: false,
        });
        self.input.clear();
        self.cursor_pos = 0;
    }

    pub(super) fn queue_advisor_request(&mut self, request: AdvisorRequest) {
        self.advisor_picker.request_id = None;
        self.advisor_picker.session_id = self.remote_session_id.clone();
        if let AdvisorRequest::ModelOptions { selection } = &request {
            self.show_advisor_entries(vec![entry(
                if selection.is_some() {
                    "Loading reasoning efforts…"
                } else {
                    "Loading advisor models…"
                }
                .into(),
                "Signed-in providers".into(),
                String::new(),
                "Esc to cancel".into(),
                None,
                false,
            )]);
        } else {
            self.inline_interactive_state = None;
        }
        self.advisor_picker.pending = Some(request);
    }

    pub(super) async fn forward_pending_advisor_request(&mut self, remote: &mut RemoteConnection) {
        let Some(request) = self.advisor_picker.pending.take() else {
            return;
        };
        if self.advisor_picker.session_id != self.remote_session_id {
            return;
        }
        let opens_picker = matches!(request, AdvisorRequest::ModelOptions { .. });
        match remote.advisor(request).await {
            Ok(id) => {
                self.advisor_picker.request_id = opens_picker.then_some(id);
                self.advisor_picker.in_flight.insert(
                    id,
                    AdvisorInFlight {
                        session_id: self.remote_session_id.clone(),
                        opens_picker,
                    },
                );
                while self.advisor_picker.in_flight.len() > 64 {
                    self.advisor_picker.in_flight.pop_first();
                }
            }
            Err(error) => {
                self.inline_interactive_state = None;
                self.push_display_message(DisplayMessage::error(format!(
                    "Advisor request failed: {error}"
                )));
            }
        }
    }

    pub(super) fn handle_advisor_result(&mut self, id: u64, result: AdvisorControlResult) {
        // A deferred control can finish after this client attaches to another
        // session. Its success or failure must not be attributed to that session.
        let Some(request) = self.advisor_picker.in_flight.remove(&id) else {
            return;
        };
        if request.session_id != self.remote_session_id {
            return;
        }
        let expected = self.advisor_picker.request_id == Some(id);
        let picker_open = self
            .inline_interactive_state
            .as_ref()
            .is_some_and(|picker| picker.is_advisor_picker());
        let same_session = self.advisor_picker.session_id == self.remote_session_id;
        if expected {
            self.advisor_picker.request_id = None;
        }
        if request.opens_picker && !(expected && picker_open && same_session) {
            return;
        }
        if let Some(error) = result.error {
            if expected && picker_open && same_session {
                self.inline_interactive_state = None;
            }
            self.push_display_message(DisplayMessage::error(error));
        } else if let Some(options) = result.model_options {
            if expected && picker_open && same_session {
                let follows_primary = result
                    .model_settings
                    .as_ref()
                    .is_some_and(|settings| settings.follows_primary);
                let current = result
                    .model_settings
                    .as_ref()
                    .and_then(|settings| settings.selection.as_ref());
                self.show_advisor_options(options, current, follows_primary);
            }
        } else if !result.message.is_empty() {
            self.push_display_message(DisplayMessage::system(result.message));
        }
    }

    pub(super) fn handle_advisor_request_error(&mut self, id: u64, message: &str) -> bool {
        if !self.advisor_picker.in_flight.contains_key(&id) {
            return false;
        }
        self.handle_advisor_result(
            id,
            AdvisorControlResult {
                error: Some(format!("Advisor request failed: {message}")),
                ..Default::default()
            },
        );
        true
    }

    fn show_advisor_options(
        &mut self,
        options: AdvisorModelOptions,
        current: Option<&RouteSelection>,
        follows_primary: bool,
    ) {
        let entries = if let Some(selection) = options.selection {
            let mut efforts = options
                .available_efforts
                .into_iter()
                .filter(|effort| !matches!(effort.as_str(), "swarm" | "swarm-deep"))
                .map(Some)
                .collect::<Vec<_>>();
            if efforts.is_empty() {
                efforts.push(None);
            }
            efforts
                .into_iter()
                .map(|effort| {
                    let label = effort.as_deref().unwrap_or("no effort setting");
                    entry(
                        format!("{} · {}", selection.model, label),
                        selection.provider_label.clone(),
                        selection.api_method.clone(),
                        "Enable advisor with this model and effort".into(),
                        Some(AdvisorRequest::SelectModel {
                            selection: selection.clone(),
                            reasoning_effort: effort.clone(),
                        }),
                        effort == options.reasoning_effort,
                    )
                })
                .collect()
        } else {
            let mut entries = vec![entry(
                "Follow main model and effort".into(),
                "Session".into(),
                String::new(),
                "Enable advisor and follow your main model".into(),
                Some(AdvisorRequest::UsePrimary),
                follows_primary,
            )];
            // New servers provide canonical runtime identities, including
            // named compatible profiles. Deriving identities from display
            // labels is only a compatibility fallback for older daemons.
            let mut selections = options.available_selections;
            if selections.is_empty() {
                selections = options
                    .available_routes
                    .iter()
                    .filter(|route| route.available)
                    .map(RouteSelection::from_model_route)
                    .collect();
            }
            selections.sort_by(|a, b| {
                (&a.model, &a.provider_label, &a.api_method)
                    .cmp(&(&b.model, &b.provider_label, &b.api_method))
            });
            if selections.is_empty() {
                entries.push(entry(
                    "No available advisor models".into(),
                    "Check /login and advisor permissions".into(),
                    String::new(),
                    "Sign in or allow a model, then reopen /advisor".into(),
                    None,
                    false,
                ));
            }
            entries.extend(selections.into_iter().map(|mut selection| {
                selection.detail.clear();
                let selected = !follows_primary
                    && current.is_some_and(|current| {
                        current.model == selection.model
                            && current.runtime_key == selection.runtime_key
                            && current.api_method == selection.api_method
                            && current.provider_label == selection.provider_label
                    });
                entry(
                    selection.model.clone(),
                    selection.provider_label.clone(),
                    selection.api_method.clone(),
                    "Choose reasoning effort next".into(),
                    Some(AdvisorRequest::ModelOptions {
                        selection: Some(selection),
                    }),
                    selected,
                )
            }));
            entries
        };
        self.show_advisor_entries(entries);
    }
}
