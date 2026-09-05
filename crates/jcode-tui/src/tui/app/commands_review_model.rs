use super::App;
use crate::config::ConfigModelRoute;
use crate::session::Session;
use anyhow::{Result, bail};

/// Snapshot the complete child route before an asynchronous split can race a
/// role-config edit. These fields contain route identifiers, never credentials.
#[derive(Clone, Debug)]
pub(crate) struct ReviewModelSelection {
    override_model: bool,
    role_model_selection: Option<ConfigModelRoute>,
    model: Option<String>,
    provider_key: Option<String>,
    route_api_method: Option<String>,
    reasoning_effort: Option<String>,
}

impl ReviewModelSelection {
    pub(super) fn for_role(app: &App, label: &str) -> Result<Self> {
        let config = crate::config::config();
        let (route, model, effort) = match label.to_ascii_lowercase().as_str() {
            "review" | "autoreview" => (
                config.autoreview.route.as_ref(),
                config.autoreview.model.as_deref(),
                config.autoreview.effort.as_deref(),
            ),
            "judge" | "autojudge" => (
                config.autojudge.route.as_ref(),
                config.autojudge.model.as_deref(),
                config.autojudge.effort.as_deref(),
            ),
            _ => bail!("Unknown review role: {label}"),
        };
        let mut selection = Self::from_settings(&app.session, route, model, effort);
        if route.is_none() && model.is_none() && effort.is_none() {
            selection.reasoning_effort = if app.is_remote {
                app.remote_reasoning_effort_hint()
            } else {
                app.provider.reasoning_effort()
            }
            .or(selection.reasoning_effort);
        }
        if !app.is_remote {
            let provider =
                crate::provider::fork_for_agent_role(app.provider.as_ref(), route, model, effort)?;
            selection.model.get_or_insert_with(|| provider.model());
            selection.reasoning_effort = provider.reasoning_effort();
        }
        Ok(selection)
    }

    fn from_settings(
        parent: &Session,
        route: Option<&ConfigModelRoute>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Self {
        let mut selection = Self {
            override_model: false,
            role_model_selection: route.cloned(),
            model: parent.model.clone(),
            provider_key: parent.provider_key.clone(),
            route_api_method: parent.route_api_method.clone(),
            reasoning_effort: parent.reasoning_effort.clone(),
        };
        if let Some(route) = route {
            selection.override_model = true;
            let route = crate::provider::configured_role_route(route);
            selection.model = Some(
                if route.runtime_key == crate::provider::RuntimeKey::OpenRouter {
                    route.routed_model_spec()
                } else {
                    route.model.clone()
                },
            );
            selection.provider_key = Some(route.runtime_key.stable_id());
            selection.route_api_method = Some(route.api_method);
            // A fixed model without an effort uses that model's default.
            selection.reasoning_effort = effort.map(str::to_owned);
        } else if let Some(model) = model
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("inherit"))
        {
            selection.override_model = true;
            selection.model = Some(model.to_owned());
            selection.provider_key = None;
            selection.route_api_method = None;
            selection.reasoning_effort = effort.map(str::to_owned);
        } else if let Some(effort) = effort {
            selection.reasoning_effort = Some(effort.to_owned());
        }
        selection
    }

    pub(super) fn apply(&self, session: &mut Session) {
        session.role_model_selection = self.role_model_selection.clone();
        // Remote splits already contain the daemon's authoritative main route;
        // the remote UI's local placeholder session must not replace it.
        if self.override_model {
            session.model = self.model.clone();
            session.provider_key = self.provider_key.clone();
            session.route_api_method = self.route_api_method.clone();
            session.reasoning_effort = self.reasoning_effort.clone();
        } else if self.reasoning_effort.is_some() {
            session.reasoning_effort = self.reasoning_effort.clone();
        }
        session.provider_session_id = None;
    }
}

#[cfg(test)]
#[path = "tests/review_role_selection.rs"]
mod tests;
