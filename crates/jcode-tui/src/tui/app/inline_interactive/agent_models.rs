use super::helpers::{
    agent_model_target_config_path, agent_model_target_slug, normalize_agent_model_summary,
};
use super::*;
use crate::config::{AgentModelRole, Config, ConfigModelRoute};
use crate::tui::AgentModelTarget;

fn saved_role(target: AgentModelTarget) -> Option<AgentModelRole> {
    Some(match target {
        AgentModelTarget::Main | AgentModelTarget::Advisor => return None,
        AgentModelTarget::Swarm => AgentModelRole::Swarm,
        AgentModelTarget::Review => AgentModelRole::Review,
        AgentModelTarget::Judge => AgentModelRole::Judge,
        AgentModelTarget::Memory => AgentModelRole::Memory,
        AgentModelTarget::Ambient => AgentModelRole::Ambient,
    })
}

fn saved_selection(
    target: AgentModelTarget,
) -> (Option<String>, Option<ConfigModelRoute>, Option<String>) {
    let cfg = Config::load();
    match target {
        AgentModelTarget::Main | AgentModelTarget::Advisor => (None, None, None),
        AgentModelTarget::Swarm => (
            cfg.agents.swarm_model,
            cfg.agents.swarm_route,
            cfg.agents.swarm_effort,
        ),
        AgentModelTarget::Review => (
            cfg.autoreview.model,
            cfg.autoreview.route,
            cfg.autoreview.effort,
        ),
        AgentModelTarget::Judge => (
            cfg.autojudge.model,
            cfg.autojudge.route,
            cfg.autojudge.effort,
        ),
        AgentModelTarget::Memory => (
            cfg.agents.memory_model,
            cfg.agents.memory_route,
            cfg.agents.memory_effort,
        ),
        AgentModelTarget::Ambient => (cfg.ambient.model, cfg.ambient.route, cfg.ambient.effort),
    }
}

pub(super) fn picker_agent_target(picker: &InlineInteractiveState) -> Option<AgentModelTarget> {
    picker.entries.iter().find_map(|entry| match entry.action {
        PickerAction::AgentModelChoice { target, .. } => Some(target),
        _ => None,
    })
}

pub(super) fn picker_uses_model_catalog(picker: &InlineInteractiveState) -> bool {
    picker.kind == PickerKind::Model
        && picker.entries.iter().any(|entry| {
            matches!(
                entry.action,
                PickerAction::Model
                    | PickerAction::AgentModelChoice { .. }
                    | PickerAction::SubagentModelChoice { .. }
            )
        })
}

fn expand_role_efforts(entries: Vec<PickerEntry>) -> Vec<PickerEntry> {
    let mut expanded = Vec::new();
    let mut defaults: Vec<PickerEntry> = Vec::new();
    let mut default_indices = HashMap::new();
    for entry in entries {
        let base = model_entry_base_name(&entry);
        // A plain row means the selected model's default effort. Keep it even
        // when /model only offers explicit efforts, including legacy settings.
        let default = if let Some(&index) = default_indices.get(&base) {
            &mut defaults[index]
        } else {
            let mut row = entry.clone();
            row.name = base.clone();
            row.effort = None;
            row.options.clear();
            row.selected_option = 0;
            let index = defaults.len();
            default_indices.insert(base.clone(), index);
            defaults.push(row);
            &mut defaults[index]
        };
        for option in &entry.options {
            if !default.options.iter().any(|saved| {
                saved.provider == option.provider && saved.api_method == option.api_method
            }) {
                default.options.push(option.clone());
            }
        }
        if entry.effort.is_some() {
            expanded.push(entry);
            continue;
        }
        for option in &entry.options {
            let efforts = match crate::provider::ModelRouteApiMethod::parse(&option.api_method) {
                crate::provider::ModelRouteApiMethod::JcodeSubscription => {
                    jcode_provider_core::OPENROUTER_SELECTABLE_EFFORTS.to_vec()
                }
                crate::provider::ModelRouteApiMethod::OpenAiCompatible { .. } => {
                    inferred_reasoning_efforts(Some(&option.api_method), Some(&base))
                }
                _ => Vec::new(),
            };
            for effort in efforts
                .into_iter()
                .filter(|effort| !matches!(*effort, "swarm" | "swarm-deep"))
            {
                let mut row = entry.clone();
                row.name = format!("{base} ({effort})");
                row.effort = Some(effort.into());
                row.options = vec![option.clone()];
                row.selected_option = 0;
                expanded.push(row);
            }
        }
    }
    defaults.extend(expanded);
    defaults
}

fn informational_entry(
    target: AgentModelTarget,
    name: String,
    detail: &str,
    available: bool,
    current: bool,
) -> PickerEntry {
    PickerEntry {
        name,
        options: vec![PickerOption {
            provider: "saved default".into(),
            api_method: agent_model_target_config_path(target).into(),
            available,
            detail: detail.into(),
            estimated_reference_cost_micros: None,
        }],
        action: PickerAction::AgentModelChoice {
            target,
            clear_override: available,
        },
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
    pub(crate) fn open_agents_picker(&mut self) {
        self.pending_model_picker_load = None;
        let entries: Vec<_> = [
            AgentModelTarget::Main,
            AgentModelTarget::Swarm,
            AgentModelTarget::Advisor,
            AgentModelTarget::Review,
            AgentModelTarget::Judge,
            AgentModelTarget::Memory,
            AgentModelTarget::Ambient,
        ]
        .into_iter()
        .map(|target| {
            let (model, route, effort) = saved_selection(target);
            let mut summary = model
                .clone()
                .or_else(|| route.as_ref().map(|route| route.model.clone()))
                .unwrap_or_else(|| agent_model_default_summary(target, self));
            if let Some(route) = &route {
                summary.push_str(&format!(" · {}", route.provider_label));
            }
            if let Some(effort) = effort {
                summary.push_str(&format!(" · {effort}"));
            }
            let scope = if saved_role(target).is_some() {
                "saved default"
            } else {
                "current session"
            };
            let mut entry = informational_entry(
                target,
                agent_model_target_label(target).into(),
                &format!(
                    "/agents {} · {scope} · choose model and effort",
                    agent_model_target_slug(target)
                ),
                true,
                false,
            );
            if target == AgentModelTarget::Swarm {
                entry.options[0]
                    .detail
                    .push_str(" · routing: /swarm-prompt");
            }
            entry.options[0].provider = summary;
            entry.action = PickerAction::AgentTarget(target);
            entry.is_default = model.is_some() || route.is_some();
            entry
        })
        .collect();
        self.inline_view_state = None;
        self.inline_interactive_state = Some(InlineInteractiveState {
            kind: PickerKind::Model,
            filtered: (0..entries.len()).collect(),
            entries,
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        });
        self.input.clear();
        self.cursor_pos = 0;
    }

    pub(crate) fn open_agent_model_picker(&mut self, target: AgentModelTarget) {
        match target {
            AgentModelTarget::Main => self.open_model_picker(),
            AgentModelTarget::Advisor => {
                if self.is_remote {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.pending_model_picker_load = None;
                    self.queue_advisor_request(crate::protocol::AdvisorRequest::ModelOptions {
                        selection: None,
                    });
                } else {
                    self.inline_interactive_state = None;
                    self.handle_unavailable_advisor_command("/advisor");
                }
            }
            _ => {
                self.open_model_picker();
                self.configure_agent_model_picker(target);
                self.set_status_notice(format!(
                    "{}: choose a model, connection and effort",
                    agent_model_target_label(target)
                ));
            }
        }
    }

    pub(super) fn configure_agent_model_picker(&mut self, target: AgentModelTarget) {
        let (model, route, effort) = saved_selection(target);
        let mut picker =
            self.inline_interactive_state
                .take()
                .unwrap_or_else(|| InlineInteractiveState {
                    kind: PickerKind::Model,
                    entries: Vec::new(),
                    filtered: Vec::new(),
                    selected: 0,
                    column: 0,
                    filter: String::new(),
                    preview: false,
                });
        // Only transform raw catalog rows. Repeated refreshes cannot duplicate controls.
        picker
            .entries
            .retain(|entry| matches!(entry.action, PickerAction::Model));
        picker.entries = expand_role_efforts(picker.entries);
        let mut found = false;
        for entry in &mut picker.entries {
            let selected = entry.options.iter().position(|option| {
                if let Some(saved) = &route {
                    saved.model == model_entry_base_name(entry)
                        && saved.api_method == option.api_method
                        && saved.provider_label == option.provider
                } else {
                    model.as_deref().is_some_and(|saved| {
                        picker_route_model_spec(entry, option) == saved
                            || model_entry_base_name(entry) == saved
                    })
                }
            });
            entry.is_current = selected.is_some() && entry.effort == effort;
            if entry.is_current {
                entry.selected_option = selected.unwrap_or(0);
                found = true;
            }
            for option in &mut entry.options {
                if matches!(option.api_method.as_str(), "current" | "remote-catalog") {
                    option.available = false;
                    option.detail = "Waiting for an authenticated model catalog".into();
                }
            }
            entry.action = PickerAction::AgentModelChoice {
                target,
                clear_override: false,
            };
            entry.is_default = false;
            entry.is_favorite = false;
        }
        if !found
            && let Some(model) = model
                .as_ref()
                .or_else(|| route.as_ref().map(|route| &route.model))
        {
            let label = effort
                .as_ref()
                .map_or_else(|| model.clone(), |effort| format!("{model} ({effort})"));
            picker.entries.insert(0, informational_entry(target, label,
                "Saved selection is unavailable in this catalog; sign in or choose another model", false, true));
        }
        let primary = self
            .remote_provider_model
            .clone()
            .or_else(|| Some(self.provider.model()));
        let inherit = normalize_agent_model_summary(
            target,
            match target {
                AgentModelTarget::Memory => None,
                AgentModelTarget::Ambient => Some("provider default".into()),
                AgentModelTarget::Swarm => self.session.subagent_model.clone().or(primary),
                _ => primary,
            },
        );
        picker.entries.insert(
            0,
            informational_entry(
                target,
                format!("inherit ({inherit})"),
                "Clear saved model, connection and effort",
                true,
                model.is_none() && route.is_none(),
            ),
        );
        picker.filtered = (0..picker.entries.len()).collect();
        picker.selected = picker
            .entries
            .iter()
            .position(|entry| entry.is_current)
            .unwrap_or(0);
        picker.column = 0;
        picker.filter.clear();
        self.inline_interactive_state = Some(picker);
    }

    pub(super) fn apply_agent_model_choice(
        &mut self,
        target: AgentModelTarget,
        clear: bool,
        entry: &PickerEntry,
    ) {
        let Some(role) = saved_role(target) else {
            return;
        };
        let result = if clear {
            Config::set_agent_model_selection(role, None, None, None)
        } else if let Some(option) = entry.options.get(entry.selected_option).filter(|option| {
            option.available && !matches!(option.api_method.as_str(), "current" | "remote-catalog")
        }) {
            let route = ConfigModelRoute {
                model: model_entry_base_name(entry),
                api_method: option.api_method.clone(),
                provider_label: option.provider.clone(),
            };
            Config::set_agent_model_selection(
                role,
                Some(&route),
                Some(&picker_route_model_spec(entry, option)),
                entry.effort.as_deref(),
            )
        } else {
            self.set_status_notice("Choose an available authenticated model route");
            return;
        };
        match result {
            Ok(()) => {
                self.inline_interactive_state = None;
                self.pending_model_picker_load = None;
                let selection = if clear {
                    "inherit".into()
                } else {
                    entry.name.clone()
                };
                let label = agent_model_target_label(target);
                self.push_display_message(DisplayMessage::system(format!(
                    "Saved {label} model: {selection}. Applies to new {label} tasks."
                )));
                self.set_status_notice(format!("{label} model → {selection}"));
            }
            Err(error) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Failed to save {} model: {error}",
                    agent_model_target_label(target)
                )));
                self.set_status_notice("Agent model save failed");
            }
        }
    }
}
