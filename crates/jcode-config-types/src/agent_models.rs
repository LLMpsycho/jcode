use serde::{Deserialize, Serialize};

/// Persisted catalog route identity, without runtime dependencies or credentials.
/// The provider layer reconstructs the runtime key from the catalog API method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigModelRoute {
    pub model: String,
    pub api_method: String,
    pub provider_label: String,
}

/// Agent defaults that are shared by sessions through the config file.
/// Advisor selection is session-scoped and uses its existing control protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentModelRole {
    Swarm,
    Review,
    Judge,
    Memory,
    Ambient,
}
