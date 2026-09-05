use super::*;
use crate::config::ConfigModelRoute;
use crate::provider::{RouteSelection, RuntimeKey, configured_role_route, fork_for_agent_role};

impl Agent {
    /// Restore a role's explicit route before constructing a runnable agent.
    /// Old sessions retain their normal constructor behavior.
    pub(crate) fn new_with_role_session(
        provider: Arc<dyn Provider>,
        registry: Registry,
        mut session: Session,
        allowed_tools: Option<HashSet<String>>,
    ) -> Result<Self> {
        let Some(route) = session.role_model_selection.clone() else {
            return Ok(Self::new_with_session(provider, registry, session, allowed_tools));
        };
        let provider = fork_for_agent_role(
            provider.as_ref(), Some(&route), None, session.reasoning_effort.as_deref(),
        )?;
        // The selected fork already holds exact auth/model/effort. Do not
        // repeat the legacy best-effort string restoration in the constructor.
        session.model = None;
        session.reasoning_effort = None;
        let mut agent = Self::new_with_session(provider, registry, session, allowed_tools);
        agent.set_role_model_selection(&configured_role_route(&route))?;
        Ok(agent)
    }

    pub(crate) fn set_role_model_selection(&mut self, selection: &RouteSelection) -> Result<()> {
        self.session.role_model_selection = Some(ConfigModelRoute {
            model: selection.model.clone(),
            api_method: selection.api_method.clone(),
            provider_label: selection.provider_label.clone(),
        });
        self.session.model = Some(if selection.runtime_key == RuntimeKey::OpenRouter {
            selection.routed_model_spec()
        } else {
            selection.model.clone()
        });
        self.session.provider_key = Some(selection.runtime_key.stable_id());
        self.session.route_api_method = Some(selection.api_method.clone());
        self.session.reasoning_effort = self.provider.reasoning_effort();
        self.provider.set_route_pinned(true);
        self.session.save()
    }
}

#[cfg(test)]
#[path = "role_model_tests.rs"]
mod tests;
