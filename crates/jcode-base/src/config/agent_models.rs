use super::{AgentModelRole, Config, ConfigModelRoute};
use std::io::Write;

impl Config {
    /// Save one agent default as a single model/route/effort update.
    ///
    /// The legacy model string remains readable by older binaries. Exact routes
    /// take precedence in current binaries. Clearing the model and route also
    /// clears effort, so inheriting cannot retain a stale model-specific effort.
    /// Read errors and invalid TOML leave the original file untouched, and
    /// process environment overrides are never baked into the saved config.
    pub fn set_agent_model_selection(
        role: AgentModelRole,
        route: Option<&ConfigModelRoute>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> anyhow::Result<()> {
        let route = route.cloned();
        if let Some(route) = &route {
            anyhow::ensure!(
                !route.model.trim().is_empty()
                    && !route.api_method.trim().is_empty()
                    && !route.provider_label.trim().is_empty(),
                "An agent model route must include its model, API method, and provider"
            );
        }
        let model =
            normalized_value(model).or_else(|| route.as_ref().map(|route| route.model.clone()));
        let effort = if model.is_some() || route.is_some() {
            normalized_value(effort)
        } else {
            None
        };
        let mut cfg = Self::load_for_update()?;
        match role {
            AgentModelRole::Swarm => {
                cfg.agents.swarm_model = model;
                cfg.agents.swarm_route = route;
                cfg.agents.swarm_effort = effort;
            }
            AgentModelRole::Review => {
                cfg.autoreview.model = model;
                cfg.autoreview.route = route;
                cfg.autoreview.effort = effort;
            }
            AgentModelRole::Judge => {
                cfg.autojudge.model = model;
                cfg.autojudge.route = route;
                cfg.autojudge.effort = effort;
            }
            AgentModelRole::Memory => {
                cfg.agents.memory_model = model;
                cfg.agents.memory_route = route;
                cfg.agents.memory_effort = effort;
            }
            AgentModelRole::Ambient => {
                cfg.ambient.model = model;
                cfg.ambient.route = route;
                cfg.ambient.effort = effort;
            }
        }

        let path = Self::path().ok_or_else(|| anyhow::anyhow!("No config path"))?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("No config directory"))?;
        std::fs::create_dir_all(parent)?;
        let content = toml::to_string_pretty(&cfg)?;
        // Temp files start owner-only and persist replaces the destination
        // atomically, so a crash cannot leave half of a route selection saved.
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(content.as_bytes())?;
        staged.as_file().sync_all()?;
        staged.persist(&path).map_err(|error| error.error)?;
        Self::invalidate_cache();
        Ok(())
    }
}

fn normalized_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
