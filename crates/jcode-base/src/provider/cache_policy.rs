use super::*;

pub fn cache_ttl_for_provider(provider: &str) -> Option<u64> {
    cache_ttl_for_provider_model(provider, None)
}

/// Get the prompt cache TTL in seconds for a given provider/model pair.
///
/// This is provider cache-retention policy: it depends only on provider
/// families (anthropic/openai/...) and their model capabilities, so it lives
/// in `provider` rather than the UI layer.
pub fn cache_ttl_for_provider_model(provider: &str, model: Option<&str>) -> Option<u64> {
    match provider.to_lowercase().as_str() {
        "anthropic" | "claude" => Some(if anthropic::is_cache_ttl_1h() {
            60 * 60
        } else {
            300
        }),
        "openai" => {
            if model
                .map(openai::supports_extended_prompt_cache_retention)
                .unwrap_or(false)
            {
                Some(24 * 60 * 60)
            } else {
                Some(300)
            }
        }
        "openrouter" => Some(300),
        "jcode subscription" => Some(300),
        "gemini" => Some(300),
        "copilot" => None,
        "cursor" => None,
        "antigravity" => None,
        _ => None,
    }
}
