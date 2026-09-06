use super::*;
use crate::auth::codex;
use std::ffi::OsString;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        crate::env::set_var(key, value);
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        crate::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            crate::env::set_var(self.key, previous);
        } else {
            crate::env::remove_var(self.key);
        }
    }
}

#[test]
fn test_sidecar_fast_model() {
    assert_eq!(SIDECAR_FAST_MODEL, "gpt-5.6-luna");
    assert_eq!(SIDECAR_CLAUDE_MODEL, "claude-haiku-4-5-20251001");
}

#[test]
fn sidecar_http_error_classifies_permanent_client_failures() {
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
    ] {
        let error: anyhow::Error = SidecarHttpError {
            provider: "test",
            status,
            body: "failure".to_string(),
        }
        .into();
        assert_eq!(classify_error(&error), SidecarErrorKind::Permanent);
    }
}

#[test]
fn sidecar_http_error_classifies_retryable_failures() {
    for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY] {
        let error: anyhow::Error = SidecarHttpError {
            provider: "test",
            status,
            body: "failure".to_string(),
        }
        .into();
        assert_eq!(classify_error(&error), SidecarErrorKind::Transient);
    }
    assert_eq!(
        classify_error(&anyhow::anyhow!("connection reset")),
        SidecarErrorKind::Transient
    );
}

#[test]
fn test_backend_selection_prefers_openai() {
    // Make backend selection deterministic by isolating credentials.
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _openai = EnvVarGuard::unset("OPENAI_API_KEY");

    codex::upsert_account_from_tokens("openai-1", "sk-test-key-123", "", None, None)
        .expect("write OpenAI test auth");
    crate::auth::claude::upsert_account(crate::auth::claude::AnthropicAccount {
        label: "claude-1".to_string(),
        access: "claude-access".to_string(),
        refresh: "claude-refresh".to_string(),
        expires: 4_102_444_800_000,
        email: None,
        scopes: Vec::new(),
        subscription_type: None,
    })
    .expect("write Claude test auth");

    let sidecar = Sidecar::auto_selected();
    assert_eq!(sidecar.backend, SidecarBackend::OpenAI);
    assert_eq!(sidecar.model, SIDECAR_OPENAI_MODEL);
    codex::set_active_account_override(None);
    crate::auth::claude::set_active_account_override(None);
}

#[test]
fn test_chatgpt_oauth_uses_luna_with_no_reasoning_when_available() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    codex::set_active_account_override(Some("openai-1".to_string()));
    crate::provider::clear_all_model_unavailability_for_account();
    crate::provider::populate_account_models(vec![
        SIDECAR_OPENAI_MODEL.to_string(),
        SIDECAR_OPENAI_OAUTH_FALLBACK_MODEL.to_string(),
    ]);

    let (model, reasoning) = resolve_openai_request_model(SIDECAR_OPENAI_MODEL, true);
    assert_eq!(model, SIDECAR_OPENAI_MODEL);
    assert_eq!(reasoning, Some(SIDECAR_OPENAI_REASONING));

    codex::set_active_account_override(None);
}

#[test]
fn test_chatgpt_oauth_falls_back_to_gpt_5_4_low_when_luna_unavailable() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    codex::set_active_account_override(Some("openai-1".to_string()));
    crate::provider::clear_all_model_unavailability_for_account();
    crate::provider::populate_account_models(vec![SIDECAR_OPENAI_OAUTH_FALLBACK_MODEL.to_string()]);

    let (model, reasoning) = resolve_openai_request_model(SIDECAR_OPENAI_MODEL, true);
    assert_eq!(model, SIDECAR_OPENAI_OAUTH_FALLBACK_MODEL);
    assert_eq!(reasoning, Some(SIDECAR_OPENAI_OAUTH_FALLBACK_REASONING));

    codex::set_active_account_override(None);
}

#[test]
fn test_build_openai_request_uses_configured_default_and_fallback_reasoning() {
    let request = build_openai_request(
        SIDECAR_OPENAI_OAUTH_FALLBACK_MODEL,
        "system",
        "hello",
        true,
        Some(SIDECAR_OPENAI_OAUTH_FALLBACK_REASONING),
    );
    assert_eq!(request["model"], SIDECAR_OPENAI_OAUTH_FALLBACK_MODEL);
    assert_eq!(
        request["reasoning"],
        serde_json::json!({"effort": SIDECAR_OPENAI_OAUTH_FALLBACK_REASONING})
    );

    let luna_request = build_openai_request(
        SIDECAR_OPENAI_MODEL,
        "system",
        "hello",
        true,
        Some(SIDECAR_OPENAI_REASONING),
    );
    assert_eq!(luna_request["model"], SIDECAR_OPENAI_MODEL);
    assert_eq!(
        luna_request["reasoning"],
        serde_json::json!({"effort": SIDECAR_OPENAI_REASONING})
    );
}

#[test]
fn test_openai_api_key_mode_uses_luna_with_no_reasoning() {
    let (model, reasoning) = resolve_openai_request_model(SIDECAR_OPENAI_MODEL, false);
    assert_eq!(model, SIDECAR_OPENAI_MODEL);
    assert_eq!(reasoning, Some(SIDECAR_OPENAI_REASONING));
}

// ---- Provider-backed sidecar (works on ALL providers) -------------------

/// Minimal provider stub that echoes a fixed reply for `complete`, so the
/// default `complete_simple` path the sidecar uses can be exercised without
/// network access. Stands in for any of the 8 real providers.
struct StubProvider {
    name: &'static str,
    reply: String,
}

#[async_trait::async_trait]
impl crate::provider::Provider for StubProvider {
    async fn complete(
        &self,
        _messages: &[crate::message::Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        let reply = self.reply.clone();
        let stream =
            futures::stream::once(
                async move { Ok(jcode_message_types::StreamEvent::TextDelta(reply)) },
            );
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        self.name
    }

    fn model(&self) -> String {
        format!("{}-model", self.name)
    }

    fn fork(&self) -> std::sync::Arc<dyn crate::provider::Provider> {
        std::sync::Arc::new(StubProvider {
            name: self.name,
            reply: self.reply.clone(),
        })
    }
}

/// With NO OpenAI/Claude credentials, the sidecar must select the live
/// agent provider (the universal path) instead of failing. This is the core
/// guarantee that memory features work on every provider, not just two.
#[test]
fn sidecar_uses_active_provider_when_no_oauth_creds() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _openai = EnvVarGuard::unset("OPENAI_API_KEY");

    // Simulate running on a non-OpenAI/Claude provider (e.g. Gemini).
    crate::provider::set_active_provider(std::sync::Arc::new(StubProvider {
        name: "gemini",
        reply: "[2,1]".to_string(),
    }));

    let sidecar = Sidecar::auto_selected();
    assert_eq!(
        sidecar.backend_name(),
        "provider",
        "with no OAuth creds, the sidecar must route through the active provider"
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(sidecar.complete("rank these", "1. a\n2. b"))
        .expect("provider-backed completion should succeed");
    assert_eq!(out, "[2,1]", "sidecar must return the provider's text");
}

#[test]
fn provider_sidecar_keeps_the_route_selected_at_construction() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _openai = EnvVarGuard::unset("OPENAI_API_KEY");

    crate::provider::set_active_provider(std::sync::Arc::new(StubProvider {
        name: "configured-profile",
        reply: "configured-profile-response".to_string(),
    }));
    let sidecar = Sidecar::auto_selected();

    // Model switches and catalog refreshes can replace the process-global
    // provider while a background memory request is queued. Dispatch must
    // still use the exact profile fork selected above.
    crate::provider::set_active_provider(std::sync::Arc::new(StubProvider {
        name: "fallback-route",
        reply: "wrong-fallback-response".to_string(),
    }));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(sidecar.complete("extract memories", "conversation"))
        .expect("provider-backed completion should retain its route");

    assert_eq!(sidecar.model_name(), "configured-profile-model");
    assert_eq!(out, "configured-profile-response");
}

/// Provider adapters that enforce empty tool sets use the same text-only
/// completion path regardless of their provider label.
#[test]
fn sidecar_provider_path_works_for_toolless_provider_adapters() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::TempDir::new().expect("create temp jcode home");
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _openai = EnvVarGuard::unset("OPENAI_API_KEY");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    for provider in [
        "claude",
        "openai",
        "copilot",
        "antigravity",
        "gemini",
        "cursor",
        "bedrock",
        "openrouter",
    ] {
        crate::provider::set_active_provider(std::sync::Arc::new(StubProvider {
            name: provider,
            reply: "[1]".to_string(),
        }));
        let sidecar = Sidecar::auto_selected();
        assert_eq!(
            sidecar.backend_name(),
            "provider",
            "{provider}: sidecar should use the provider path with no OAuth creds"
        );
        let out = rt
            .block_on(sidecar.complete("sys", "user"))
            .unwrap_or_else(|e| panic!("{provider}: provider-backed completion failed: {e}"));
        assert_eq!(out, "[1]", "{provider}: sidecar must echo provider output");
    }
}

#[test]
fn test_is_anthropic_oauth_forbidden() {
    // The exact error string the sidecar surfaces from a forbidden OAuth org.
    let forbidden = anyhow::anyhow!(
        "Claude API error (403 Forbidden): {{\"type\":\"error\",\"error\":{{\"type\":\"permission_error\",\"message\":\"OAuth authentication is currently not allowed for this organization.\"}}}}"
    );
    assert!(is_anthropic_oauth_forbidden(&forbidden));

    // Unrelated failures must NOT trigger the API-key fallback.
    assert!(!is_anthropic_oauth_forbidden(&anyhow::anyhow!(
        "Claude API error (401 Unauthorized): bad token"
    )));
    assert!(!is_anthropic_oauth_forbidden(&anyhow::anyhow!(
        "Failed to send request to Claude API"
    )));
    // A 403 from a permission_error (the organization gate) still counts even
    // if the human-readable message phrasing changes slightly.
    assert!(is_anthropic_oauth_forbidden(&anyhow::anyhow!(
        "Claude API error (403 Forbidden): {{\"error\":{{\"type\":\"permission_error\"}}}}"
    )));
}

#[test]
fn test_build_claude_api_key_system_param_omits_identity_spoof() {
    // API-key path must NOT impersonate the official Claude Code CLI.
    let none = build_claude_api_key_system_param("");
    assert!(none.is_none(), "empty system => no system param");

    let ClaudeApiSystem::Blocks(blocks) =
        build_claude_api_key_system_param("be terse").expect("system present");
    assert_eq!(blocks.len(), 1, "only the caller's system prompt is sent");
    assert_eq!(blocks[0].text, "be terse");

    // The OAuth builder, by contrast, injects the Claude Code identity spoof.
    let ClaudeApiSystem::Blocks(oauth_blocks) =
        build_claude_system_param("be terse").expect("oauth system present");
    assert!(
        oauth_blocks.iter().any(|b| b.text == CLAUDE_CODE_IDENTITY),
        "oauth path keeps the identity block"
    );
}

#[test]
fn test_anthropic_sidecar_prefers_api_key_respects_pinned_mode() {
    // Pinning the runtime to API-key mode must make the sidecar prefer the key.
    let _g = EnvVarGuard::set_path("JCODE_RUNTIME_PROVIDER", std::path::Path::new("claude-api"));
    assert!(
        anthropic_sidecar_prefers_api_key(),
        "claude-api runtime => prefer API key"
    );

    // Pinning to OAuth mode must NOT prefer the key.
    let _g2 = EnvVarGuard::set_path("JCODE_RUNTIME_PROVIDER", std::path::Path::new("claude"));
    assert!(
        !anthropic_sidecar_prefers_api_key(),
        "claude (oauth) runtime => do not force API key"
    );
}
