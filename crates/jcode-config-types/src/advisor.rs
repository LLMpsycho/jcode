use crate::ConfigModelRoute;
use serde::{Deserialize, Serialize};

/// Internal second-model advisor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvisorConfig {
    /// Enable advisor turn capture and review scheduling (default: true).
    pub enabled: bool,
    /// Advisor operating mode.
    pub mode: AdvisorMode,
    /// Optional model-selection request applied to the forked provider.
    pub model: Option<String>,
    /// Exact authenticated catalog route, taking precedence over model strings.
    pub route: Option<ConfigModelRoute>,
    /// Reasoning effort validated against the selected advisor model.
    pub effort: Option<String>,
    /// Shared advisor specialization instructions.
    pub instructions: Option<String>,
    /// Named independent advisors. Empty retains the single default advisor.
    pub roster: Vec<AdvisorRosterEntry>,
    /// Optional reviewer role used by interactive and selfdev-guardian modes.
    pub reviewer_model: Option<String>,
    /// Optional verification role used by final-review mode.
    pub verification_model: Option<String>,
    /// Exact runtime keys allowed to receive advisor evidence. None inherits
    /// authenticated provider availability; an empty list denies every route.
    pub allowed_runtime_keys: Option<Vec<String>>,
    /// Maximum notes the advisor may publish for one primary turn.
    pub max_notes_per_turn: usize,
    /// Review one out of every N completed primary turns.
    pub review_every_n_turns: usize,
    /// Maximum provider reviews started during one advisor runtime.
    pub max_reviews_per_session: usize,
    /// Completed turns to suppress the same handled concern; observation continues.
    pub handled_note_immunity_turns: usize,
    /// Completed-turn interruption cooldown; concerns become asides, blockers bypass.
    pub interrupt_immunity_turns: usize,
    /// Minimum severity that may gate a future risky operation.
    pub block_on_severity: AdvisorSeverity,
    /// Redact recognized secrets before advisor context is retained or sent.
    pub redact: bool,
}

impl Default for AdvisorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: AdvisorMode::Interactive,
            model: None,
            route: None,
            effort: None,
            instructions: None,
            roster: Vec::new(),
            reviewer_model: None,
            verification_model: None,
            allowed_runtime_keys: None,
            max_notes_per_turn: 1,
            review_every_n_turns: 1,
            max_reviews_per_session: 100,
            handled_note_immunity_turns: 2,
            interrupt_immunity_turns: 3,
            block_on_severity: AdvisorSeverity::Blocker,
            redact: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisorMode {
    #[default]
    Interactive,
    SelfdevGuardian,
    FinalReview,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum AdvisorSeverity {
    Nit,
    Concern,
    #[default]
    Blocker,
}

/// One independently configured advisor. Permission and budget limits are inherited
/// from the enclosing advisor configuration and cannot be widened by an entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdvisorRosterEntry {
    pub name: String,
    pub enabled: bool,
    pub model: Option<String>,
    pub route: Option<ConfigModelRoute>,
    pub effort: Option<String>,
    pub instructions: Option<String>,
}

impl Default for AdvisorRosterEntry {
    fn default() -> Self {
        Self {
            name: "default".into(),
            enabled: true,
            model: None,
            route: None,
            effort: None,
            instructions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_config_keeps_single_default_and_named_routes_round_trip() {
        let legacy: AdvisorConfig =
            serde_json::from_str(r#"{"enabled":true,"model":"reviewer"}"#).unwrap();
        assert!(legacy.roster.is_empty());
        assert!(legacy.route.is_none());
        let config: AdvisorConfig = serde_json::from_str(r#"{"enabled":true,"roster":[{"name":"security","route":{"model":"reviewer","api_method":"openai-oauth","provider_label":"OpenAI"},"effort":"high","instructions":"Inspect permissions"},{"name":"tests","enabled":false}]}"#).unwrap();
        assert!(config.roster[0].enabled);
        assert!(!config.roster[1].enabled);
        let restored: AdvisorConfig =
            serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap();
        assert_eq!(
            restored.roster[0].route.as_ref().unwrap().api_method,
            "openai-oauth"
        );
        assert_eq!(restored.roster[0].effort.as_deref(), Some("high"));
        assert!(
            serde_json::from_str::<AdvisorRosterEntry>(
                r#"{"name":"security","allowed_runtime_keys":["openai-api-key"]}"#
            )
            .is_err()
        );
    }
}
