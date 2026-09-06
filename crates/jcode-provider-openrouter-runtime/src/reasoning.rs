use super::*;

impl OpenRouterProvider {
    pub(super) fn model_reasoning_config(&self) -> Option<&(Option<bool>, Option<String>)> {
        let model = self.model_snapshot().trim().to_ascii_lowercase();
        self.static_reasoning_config.get(&model)
    }

    pub(super) fn model_reasoning_support(&self) -> Option<bool> {
        self.model_reasoning_config().and_then(|config| config.0)
    }

    pub(super) fn configured_effort_for_model(&self) -> Option<String> {
        self.model_reasoning_config()
            .and_then(|config| config.1.clone())
            .or_else(|| {
                jcode_base::config::config()
                    .provider
                    .openai_reasoning_effort
                    .clone()
            })
            .and_then(|effort| self.normalize_reasoning_effort_for_self(&effort))
    }

    pub(crate) fn supports_any_reasoning_effort(&self) -> bool {
        self.supports_deepseek_reasoning_effort()
            || self.supports_openai_reasoning_effort()
            || Self::profile_supports_unified_reasoning(
                self.profile_id.as_deref(),
                self.send_openrouter_headers,
            )
    }

    pub(crate) fn normalize_reasoning_effort_for_self(&self, effort: &str) -> Option<String> {
        if self.supports_deepseek_reasoning_effort() {
            Self::normalize_reasoning_effort(effort)
        } else if self.supports_openai_reasoning_effort() {
            Self::normalize_openai_reasoning_effort(effort)
        } else {
            Self::normalize_unified_reasoning_effort(effort)
        }
    }

    /// Initial reasoning effort at construction. Named/compat profiles that
    /// support effort honor the user's configured `openai_reasoning_effort`
    /// (issue #352: previously hardcoded to None so the config was ignored).
    pub(super) fn initial_reasoning_effort(
        reasoning_effort_support: Option<bool>,
        profile_id: Option<&str>,
    ) -> Option<String> {
        let supported = reasoning_effort_support.unwrap_or(
            Self::profile_supports_reasoning_effort(profile_id)
                || Self::profile_supports_openai_reasoning_effort(profile_id),
        );
        if !supported {
            return None;
        }
        jcode_base::config::config()
            .provider
            .openai_reasoning_effort
            .as_deref()
            .and_then(|effort| {
                if Self::profile_supports_openai_reasoning_effort(profile_id) {
                    Self::normalize_openai_reasoning_effort(effort)
                } else {
                    Self::normalize_reasoning_effort(effort)
                }
            })
    }

    pub(super) fn profile_rejects_image_input(profile_id: Option<&str>) -> bool {
        matches!(profile_id, Some(id) if id.eq_ignore_ascii_case("deepseek") || id.eq_ignore_ascii_case("zai"))
    }

    pub(super) fn profile_supports_unified_reasoning(
        profile_id: Option<&str>,
        send_openrouter_headers: bool,
    ) -> bool {
        // Real OpenRouter uses unified reasoning. The runtime may carry either
        // no profile id or the "openrouter" doctor-profile id (assigned when
        // the default api base matches the OpenRouter OpenAI-compat profile),
        // so both must qualify (issue: effort rejected on plain OpenRouter).
        (send_openrouter_headers
            && profile_id.is_none_or(|id| id.eq_ignore_ascii_case("openrouter")))
            || profile_id.is_some_and(|id| id.eq_ignore_ascii_case("conifer"))
    }

    pub(super) fn normalize_reasoning_effort(raw: &str) -> Option<String> {
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() {
            return None;
        }
        match value.as_str() {
            "none" | "low" | "medium" | "high" | "max" | "swarm" | "swarm-deep" => Some(value),
            // Match the existing OpenAI UX: accept unknown non-empty effort values
            // by snapping to the strongest setting instead of rejecting the command.
            other => {
                jcode_base::logging::info(&format!(
                    "Warning: Ignoring unsupported DeepSeek reasoning effort '{}'; expected none|low|medium|high|max.",
                    other
                ));
                None
            }
        }
    }

    pub(super) fn normalize_openai_reasoning_effort(raw: &str) -> Option<String> {
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() {
            return None;
        }
        match value.as_str() {
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "swarm"
            | "swarm-deep" => Some(value),
            other => {
                jcode_base::logging::info(&format!(
                    "Warning: Ignoring unsupported OpenAI-compatible reasoning effort '{}'.",
                    other
                ));
                None
            }
        }
    }

    pub(super) fn normalize_unified_reasoning_effort(raw: &str) -> Option<String> {
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() {
            return None;
        }
        match value.as_str() {
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "swarm" | "swarm-deep" => {
                Some(value)
            }
            "max" => Some("xhigh".to_string()),
            other => {
                jcode_base::logging::info(&format!(
                    "Warning: Ignoring unsupported OpenRouter reasoning effort '{}'; expected none|minimal|low|medium|high|xhigh|max alias.",
                    other
                ));
                None
            }
        }
    }
}
