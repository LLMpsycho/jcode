use super::*;
use crate::tui::{InlineInteractiveState, PickerAction, PickerEntry, PickerKind, PickerOption};

impl App {
    pub(crate) fn open_login_picker_inline(&mut self) {
        self.open_auth_provider_picker_inline(false);
    }

    pub(crate) fn open_logout_picker_inline(&mut self) {
        self.open_auth_provider_picker_inline(true);
    }

    fn open_auth_provider_picker_inline(&mut self, logout: bool) {
        let status = crate::auth::AuthStatus::check_fast();
        let providers = crate::provider_catalog::tui_login_providers();
        let mut models = providers
            .into_iter()
            .filter(|provider| {
                // Logging out of the auto-import descriptor is meaningless: there is
                // no saved session for it, only detected external logins.
                !(logout
                    && matches!(
                        provider.target,
                        crate::provider_catalog::LoginProviderTarget::AutoImport
                    ))
            })
            .map(|provider| {
                let assessment = status.assessment_for_provider(provider);
                let auth_state = assessment.state;
                let state_label = match auth_state {
                    crate::auth::AuthState::Available => {
                        if matches!(
                            provider.target,
                            crate::provider_catalog::LoginProviderTarget::AutoImport
                        ) {
                            "detected"
                        } else {
                            "configured"
                        }
                    }
                    crate::auth::AuthState::Expired => "attention",
                    crate::auth::AuthState::NotConfigured => "setup",
                };
                PickerEntry {
                    name: provider.display_name.to_string(),
                    options: vec![PickerOption {
                        provider: provider.auth_kind.label().to_string(),
                        api_method: state_label.to_string(),
                        available: true,
                        detail: format!("{} · {}", assessment.method_detail, provider.menu_detail),
                        estimated_reference_cost_micros: None,
                    }],
                    action: if logout {
                        PickerAction::Logout(provider)
                    } else {
                        PickerAction::Login(provider)
                    },
                    selected_option: 0,
                    is_current: auth_state == crate::auth::AuthState::Available,
                    is_default: false,
                    is_favorite: false,
                    recommended: provider.recommended,
                    recommendation_rank: usize::MAX,
                    usage_score: 0,
                    old: false,
                    created_date: None,
                    effort: None,
                }
            })
            .collect::<Vec<_>>();

        if logout {
            // Prepend a synthetic "All providers" entry that logs out everywhere.
            models.insert(
                0,
                PickerEntry {
                    name: "All providers".to_string(),
                    options: vec![PickerOption {
                        provider: "all".to_string(),
                        api_method: "logout".to_string(),
                        available: true,
                        detail: "Log out of every provider with a saved session".to_string(),
                        estimated_reference_cost_micros: None,
                    }],
                    action: PickerAction::LogoutAll,
                    selected_option: 0,
                    is_current: false,
                    is_default: false,
                    is_favorite: false,
                    recommended: false,
                    recommendation_rank: usize::MAX,
                    usage_score: 0,
                    old: false,
                    created_date: None,
                    effort: None,
                },
            );
        }

        self.inline_view_state = None;
        self.inline_interactive_state = Some(InlineInteractiveState {
            kind: PickerKind::Login,
            filtered: (0..models.len()).collect(),
            entries: models,
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        });
        self.input.clear();
        self.cursor_pos = 0;
    }

}
