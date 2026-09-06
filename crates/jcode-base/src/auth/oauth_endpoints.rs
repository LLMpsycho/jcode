/// Claude Code OAuth configuration
pub mod claude {
    pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
    /// Claude Code uses the Claude.ai OAuth surface for tokens that can call
    /// `/v1/messages` with the `user:inference` scope. The platform/console
    /// authorize endpoint can mint tokens that refresh successfully but are not
    /// accepted by the inference API.
    pub const AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
    pub const CONSOLE_AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
    pub const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
    pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
    pub const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
    pub const LEGACY_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
    pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
    pub const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
    pub const REFRESH_SCOPES: &str =
        "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
}

pub(super) const CLAUDE_TOKEN_TIMEOUT_SECS: u64 = 15;

/// OpenAI Codex OAuth configuration
pub mod openai {
    pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
    pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
    pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
    pub const DEFAULT_PORT: u16 = 1455;
    pub const CALLBACK_PATH: &str = "/auth/callback";
    pub const SCOPES: &str =
        "openid profile email offline_access api.connectors.read api.connectors.invoke";

    pub fn redirect_uri(port: u16) -> String {
        format!("http://localhost:{}{}", port, CALLBACK_PATH)
    }

    pub fn default_redirect_uri() -> String {
        redirect_uri(DEFAULT_PORT)
    }
}
