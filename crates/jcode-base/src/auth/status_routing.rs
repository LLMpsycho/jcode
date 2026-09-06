use super::{
    AuthState, AuthStatus, LoginProviderAuthStateKey, LoginProviderDescriptor, api_key_available,
};

impl AuthStatus {
    pub fn state_for_key(&self, key: LoginProviderAuthStateKey) -> AuthState {
        match key {
            LoginProviderAuthStateKey::ExternalImport => {
                if Self::has_any_untrusted_external_auth() {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            LoginProviderAuthStateKey::Jcode => self.jcode,
            LoginProviderAuthStateKey::Anthropic => self.anthropic.state,
            LoginProviderAuthStateKey::OpenAi => self.openai,
            LoginProviderAuthStateKey::Azure => self.azure,
            LoginProviderAuthStateKey::Bedrock => self.bedrock,
            LoginProviderAuthStateKey::OpenRouterLike => self.openrouter,
            LoginProviderAuthStateKey::Copilot => self.copilot,
            LoginProviderAuthStateKey::Antigravity => self.antigravity,
            LoginProviderAuthStateKey::Gemini => self.gemini,
            LoginProviderAuthStateKey::Cursor => self.cursor,
            LoginProviderAuthStateKey::GrokBuild => self.grok_build,
            LoginProviderAuthStateKey::Google => self.google,
        }
    }

    pub fn state_for_provider(&self, provider: LoginProviderDescriptor) -> AuthState {
        match provider.target {
            crate::provider_catalog::LoginProviderTarget::AutoImport => {
                if Self::has_any_untrusted_external_auth() {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            crate::provider_catalog::LoginProviderTarget::Jcode => {
                if crate::subscription_catalog::has_credentials() {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            crate::provider_catalog::LoginProviderTarget::OpenRouter => {
                if api_key_available("OPENROUTER_API_KEY", "openrouter.env") {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            crate::provider_catalog::LoginProviderTarget::OpenAiApiKey => {
                if api_key_available("OPENAI_API_KEY", "openai.env") {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            // The `anthropic-api` login provider is the *API-key* path. It must
            // report on the presence of an Anthropic API key alone, never borrow
            // the OAuth/subscription credential's availability (that is the
            // separate `claude` provider). Sharing `auth_state_key::Anthropic`
            // previously made this provider claim "available / OAuth + API key"
            // even with zero API key configured, which then failed at request
            // time because API-key mode never falls back to OAuth.
            crate::provider_catalog::LoginProviderTarget::ClaudeApiKey => {
                if api_key_available("ANTHROPIC_API_KEY", "anthropic.env") {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            // The `claude` login provider is the *OAuth/subscription* path: the
            // mirror image of the `anthropic-api` rule above. It must report on
            // the OAuth credential alone, never borrow the API key's
            // availability, so the two rows never blur into one ambiguous
            // "OAuth + API key" answer.
            crate::provider_catalog::LoginProviderTarget::Claude => self.anthropic.oauth_state,
            // Same split for OpenAI: `openai` is the ChatGPT/Codex OAuth login,
            // `openai-api` (handled above) is the API-key login.
            crate::provider_catalog::LoginProviderTarget::OpenAi => self.openai_oauth_state,
            crate::provider_catalog::LoginProviderTarget::Bedrock => {
                if crate::provider::bedrock::BedrockProvider::has_credentials() {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            crate::provider_catalog::LoginProviderTarget::GrokBuild => self.grok_build,
            crate::provider_catalog::LoginProviderTarget::OpenAiCompatible(profile) => {
                if crate::provider_catalog::openai_compatible_profile_is_configured(profile) {
                    AuthState::Available
                } else {
                    AuthState::NotConfigured
                }
            }
            _ => self.state_for_key(provider.auth_state_key),
        }
    }
}
