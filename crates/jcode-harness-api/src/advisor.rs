//! Advisor controls use the session's existing authenticated provider routes.
//! These wire DTOs deliberately have no dependency on provider runtimes.

use crate::ModelRouteInfo;
use serde::{Deserialize, Serialize};

/// Opaque authenticated runtime identity returned by the advisor catalog.
/// Preserve `kind` exactly; it is not a provider name or a permission key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvisorRuntimeKey {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvisorRouteSelection {
    pub model: String,
    pub runtime_key: AdvisorRuntimeKey,
    pub api_method: String,
    pub provider_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// A control request, independent of the primary model and its effort.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AdvisorRequest {
    Status,
    Inspect,
    Enable,
    Disable,
    Acknowledge { note_id: String },
    Dismiss { note_id: String },
    ModelOptions {
        #[serde(default)]
        selection: Option<AdvisorRouteSelection>,
    },
    SelectModel {
        selection: AdvisorRouteSelection,
        #[serde(default)]
        reasoning_effort: Option<String>,
    },
    UsePrimary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvisorModelSettings {
    pub enabled: bool,
    pub selection: Option<AdvisorRouteSelection>,
    pub reasoning_effort: Option<String>,
    pub follows_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdvisorModelOptions {
    pub selection: Option<AdvisorRouteSelection>,
    pub reasoning_effort: Option<String>,
    pub available_routes: Vec<ModelRouteInfo>,
    /// Canonical selections to forward unchanged; older daemons may omit them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_selections: Vec<AdvisorRouteSelection>,
    pub available_efforts: Vec<String>,
}

/// A correlated control reply. `error` reports a rejected or non-durable
/// control; callers must inspect it before claiming the change succeeded.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AdvisorControlResult {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<AdvisorModelSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_options: Option<AdvisorModelOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
