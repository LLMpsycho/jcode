use super::*;

impl AnthropicProvider {
    pub(super) fn normalized_model_key(model: &str) -> String {
        strip_1m_suffix(model).trim().to_ascii_lowercase()
    }

    pub(super) fn model_supports_output_effort(model: &str) -> bool {
        // Shared capability table (with an optimistic default for unknown 5.x+
        // generations); see `jcode_provider_core::anthropic_reasoning_caps`.
        // Fable 5 verified live 2026-07-01; Sonnet 5 verified live 2026-07-07.
        jcode_provider_core::anthropic_reasoning_caps(model).output_effort
    }

    pub(super) fn model_supports_adaptive_thinking(model: &str) -> bool {
        jcode_provider_core::anthropic_reasoning_caps(model).adaptive_thinking
    }

    pub(super) fn model_supports_manual_thinking(model: &str) -> bool {
        jcode_provider_core::anthropic_reasoning_caps(model).manual_thinking
    }

    pub(super) fn model_supports_xhigh_effort(model: &str) -> bool {
        jcode_provider_core::anthropic_reasoning_caps(model).xhigh_effort
    }

    /// `max` effort ("absolute maximum capability with no constraints on token
    /// spending") is a real API level on the `output_config` effort models,
    /// except Claude Opus 4.5 where manual thinking keeps `max` as an alias for
    /// the strongest supported level.
    pub(super) fn model_supports_max_effort(model: &str) -> bool {
        jcode_provider_core::anthropic_reasoning_caps(model).max_effort
    }

    pub(super) fn model_supports_reasoning_effort(model: &str) -> bool {
        jcode_provider_core::anthropic_reasoning_caps(model).supports_reasoning_effort()
    }

    pub(super) fn normalize_reasoning_effort(raw: &str) -> Option<String> {
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() || matches!(value.as_str(), "default" | "auto") {
            return None;
        }
        match value.as_str() {
            "off" | "disabled" => Some("none".to_string()),
            // `swarm` is a UI sentinel meaning "max effort + use the swarm tool".
            // Stored verbatim; resolved to a real effort in `actual_effort_for_model`.
            "none" | "low" | "medium" | "high" | "xhigh" | "max" | "swarm" | "swarm-deep" => {
                Some(value)
            }
            other => {
                jcode_base::logging::info(&format!(
                    "Warning: Ignoring unsupported Anthropic reasoning effort '{}'; expected none|low|medium|high|xhigh|max.",
                    other
                ));
                None
            }
        }
    }

    pub(super) fn actual_effort_for_model(model: &str, effort: &str) -> String {
        if jcode_base::prompt::is_swarm_effort(effort) {
            // Swarm rungs sit above `max` on the ladder and mean "strongest
            // reasoning the model supports", so cycling upward never lowers
            // the wire effort.
            if Self::model_supports_max_effort(model) {
                "max".to_string()
            } else if Self::model_supports_xhigh_effort(model) {
                "xhigh".to_string()
            } else {
                "high".to_string()
            }
        } else if effort == "max" && !Self::model_supports_max_effort(model) {
            if Self::model_supports_xhigh_effort(model) {
                "xhigh".to_string()
            } else {
                "high".to_string()
            }
        } else if effort == "xhigh" && !Self::model_supports_xhigh_effort(model) {
            "high".to_string()
        } else {
            effort.to_string()
        }
    }

    /// Like [`Self::actual_effort_for_model`], but preserves the swarm sentinels
    /// (light `swarm` and `swarm-deep`) so the stored/UI value keeps reflecting
    /// the chosen swarm mode. Used when persisting the user's choice; request
    /// building resolves swarm to a real effort.
    pub(super) fn store_effort_for_model(model: &str, effort: &str) -> String {
        if jcode_base::prompt::is_deep_swarm_effort(effort) {
            jcode_base::prompt::SWARM_DEEP_EFFORT.to_string()
        } else if jcode_base::prompt::is_swarm_effort(effort) {
            jcode_base::prompt::SWARM_EFFORT.to_string()
        } else {
            Self::actual_effort_for_model(model, effort)
        }
    }

    /// Default reasoning effort to apply when the user has *not* explicitly
    /// configured one. Claude Opus 5 defaults to `low`: it is strong enough
    /// at low effort for day-to-day coding/agentic work, and users can cycle
    /// up when they want deeper reasoning. Older Claude Opus models are
    /// reasoning-heavy flagships, so we default them to `xhigh` where
    /// supported (Opus 4.7/4.8), clamped to `high` on older Opus.
    /// Deliberately NOT `max`: Anthropic recommends `xhigh` as the starting
    /// point for coding/agentic work and reserves `max` for frontier problems
    /// (it costs much more and can overthink). Claude Fable 5 defaults to
    /// `high`: it benefits from deeper reasoning on coding/agentic work.
    /// Every other model keeps the model's own default (no forced effort) so
    /// cheaper models stay cheap.
    pub(super) fn default_reasoning_effort_for_model(model: &str) -> Option<String> {
        let key = Self::normalized_model_key(model);
        if key.contains("claude-opus-5") {
            Some("low".to_string())
        } else if key.contains("claude-opus") {
            Some(if Self::model_supports_xhigh_effort(model) {
                "xhigh".to_string()
            } else {
                "high".to_string()
            })
        } else if key.contains("claude-fable-5") {
            // Fable 5 defaults to `high` reasoning for stronger day-to-day
            // results. Users can still cycle down for faster/cheaper turns.
            Some("high".to_string())
        } else {
            None
        }
    }

    /// The raw, user-configured reasoning effort for this provider, if any.
    /// `None` means "use the model default" (see
    /// [`Self::default_reasoning_effort_for_model`]).
    pub(super) fn stored_reasoning_effort(&self) -> Option<String> {
        self.reasoning_effort
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Effective reasoning effort for `model`, resolving the model default when
    /// the user has not configured an explicit effort.
    pub(super) fn effort_for_model(&self, model: &str) -> Option<String> {
        if !Self::model_supports_reasoning_effort(model) {
            return None;
        }
        Some(
            self.stored_reasoning_effort()
                .or_else(|| Self::default_reasoning_effort_for_model(model))
                .unwrap_or_else(|| "none".to_string()),
        )
    }

    pub(super) fn model_supports_priority_service_tier(model: &str) -> bool {
        Self::normalized_model_key(model).contains("claude-opus-4-8")
    }

    pub(super) fn normalize_service_tier(raw: &str) -> Result<Option<String>> {
        let value = raw.trim().to_ascii_lowercase();
        match value.as_str() {
            "" | "default" => Ok(None),
            "off" | "standard" | "standard_only" => Ok(Some("standard_only".to_string())),
            // The Anthropic API uses `auto` for the latency-optimized tier. Keep
            // accepting `priority` because `/fast on` is shared with OpenAI.
            "priority" | "auto" => Ok(Some("auto".to_string())),
            other => anyhow::bail!(
                "Unsupported Anthropic service tier '{}'; expected priority/auto or off/standard_only",
                other
            ),
        }
    }

    pub(super) fn current_service_tier_for_model(&self, model: &str) -> Option<String> {
        let tier = self
            .service_tier
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        tier.filter(|_| Self::model_supports_priority_service_tier(model))
    }

    /// Output-token budget for `model`: an explicit env override when set,
    /// otherwise the model's published maximum. A flat default would clamp
    /// 128K-output models to 32K and truncate long agentic turns mid-tool-call.
    pub(super) fn max_tokens_for(&self, model: &str) -> u32 {
        self.max_tokens_override
            .unwrap_or_else(|| jcode_provider_core::anthropic::anthropic_max_output_tokens(model))
    }

    pub(super) fn manual_thinking_budget(effort: &str, max_tokens: u32) -> Option<u32> {
        let desired = match effort {
            "low" => 1_024,
            "medium" => 4_096,
            "high" => 8_192,
            "xhigh" | "max" => 16_384,
            e if jcode_base::prompt::is_swarm_effort(e) => 16_384,
            _ => return None,
        };
        let budget = desired.min(max_tokens.saturating_sub(1));
        (budget >= 1_024).then_some(budget)
    }

    pub(super) fn build_reasoning_request_parts(
        &self,
        model: &str,
        is_oauth: bool,
    ) -> (Option<ApiThinking>, Option<ApiOutputConfig>, Option<f32>) {
        // `display.show_thinking` is a request to *see* the model's reasoning.
        // Anthropic only streams thinking summaries when a thinking request is
        // present, so opting into the display must also opt into generating it.
        let show_thinking = jcode_base::config::config().display.show_thinking;
        self.build_reasoning_request_parts_inner(model, is_oauth, show_thinking)
    }

    pub(super) fn build_reasoning_request_parts_inner(
        &self,
        model: &str,
        is_oauth: bool,
        show_thinking: bool,
    ) -> (Option<ApiThinking>, Option<ApiOutputConfig>, Option<f32>) {
        let effort = self.effort_for_model(model);
        // An explicit "none" (user-configured or a model default) means
        // reasoning was deliberately disabled, so it must also win over the
        // `display.show_thinking` fallback below. `effort_for_model` returns
        // Some("none") even when nothing is configured, so check the
        // stored/default effort instead.
        let effort_is_explicit_none = self
            .stored_reasoning_effort()
            .or_else(|| Self::default_reasoning_effort_for_model(model))
            .as_deref()
            == Some("none");
        let effort = effort.as_deref().filter(|effort| *effort != "none");
        let show_thinking = show_thinking && !effort_is_explicit_none;

        let output_config = effort
            .filter(|_| Self::model_supports_output_effort(model))
            .map(|effort| ApiOutputConfig {
                effort: Self::actual_effort_for_model(model, effort),
            });

        // When only the display toggle is on (no explicit effort), request
        // thinking without forcing `output_config`, so the model keeps its
        // default reasoning strength and only the thinking *display* is enabled.
        let thinking = if Self::model_supports_adaptive_thinking(model) {
            (effort.is_some() || show_thinking).then_some(ApiThinking::Adaptive {
                display: Some("summarized"),
            })
        } else if Self::model_supports_manual_thinking(model) {
            // Manual-thinking models need a concrete budget. Use the configured
            // effort, or fall back to a minimal budget when only the display
            // toggle is on.
            effort
                .or(show_thinking.then_some("low"))
                .and_then(|effort| Self::manual_thinking_budget(effort, self.max_tokens_for(model)))
                .map(|budget_tokens| ApiThinking::Enabled { budget_tokens })
        } else {
            None
        };

        // Extended/adaptive thinking is incompatible with temperature. OAuth path
        // normally mirrors Claude Code's temperature=1.0, so omit it when thinking is active.
        let temperature = if is_oauth && thinking.is_none() {
            Some(1.0)
        } else {
            None
        };

        (thinking, output_config, temperature)
    }
}
