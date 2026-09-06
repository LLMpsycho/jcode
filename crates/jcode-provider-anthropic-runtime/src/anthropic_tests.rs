use super::*;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        jcode_base::env::set_var(key, value);
        Self { key, previous }
    }

    fn set_if_missing(key: &'static str, value: &str) -> Option<Self> {
        if std::env::var_os(key).is_some() {
            return None;
        }
        Some(Self::set(key, value))
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            jcode_base::env::set_var(self.key, previous);
        } else {
            jcode_base::env::remove_var(self.key);
        }
    }
}

#[test]
fn direct_api_url_supports_standard_and_profile_overrides() {
    let _lock = jcode_base::storage::lock_test_env();
    let _standard = EnvVarGuard::set("ANTHROPIC_BASE_URL", "https://proxy.example/v1/");
    assert_eq!(
        direct_api_url().expect("valid direct API URL"),
        "https://proxy.example/v1/messages"
    );

    let _profile = EnvVarGuard::set(
        "JCODE_ANTHROPIC_API_BASE",
        "https://gateway.example/anthropic/v1/messages",
    );
    assert_eq!(
        direct_api_url().expect("valid direct API URL"),
        "https://gateway.example/anthropic/v1/messages"
    );
}

#[test]
fn configured_direct_headers_parse_and_reject_invalid_values() {
    let _lock = jcode_base::storage::lock_test_env();
    let _headers = EnvVarGuard::set(
        "JCODE_ANTHROPIC_HEADERS",
        r#"{"x-tenant":"alpha","x-route":"claude"}"#,
    );
    let parsed = configured_direct_headers().expect("valid custom headers");
    assert_eq!(parsed.get("x-tenant").unwrap(), "alpha");
    assert_eq!(parsed.get("x-route").unwrap(), "claude");

    let _invalid = EnvVarGuard::set("JCODE_ANTHROPIC_HEADERS", r#"{"bad header":"x"}"#);
    assert!(configured_direct_headers().is_err());
}

#[test]
fn anthropic_auth_token_selects_bearer_without_affecting_explicit_profile_auth() {
    let _lock = jcode_base::storage::lock_test_env();
    let _token = EnvVarGuard::set("ANTHROPIC_AUTH_TOKEN", "gateway-token");
    assert_eq!(
        direct_auth_mode().expect("valid direct auth mode"),
        "bearer"
    );

    let _explicit = EnvVarGuard::set("JCODE_ANTHROPIC_AUTH", "header");
    assert_eq!(
        direct_auth_mode().expect("valid direct auth mode"),
        "header"
    );
}

#[test]
fn named_profile_runtime_captures_transport_and_credential_immutably() {
    let _lock = jcode_base::storage::lock_test_env();
    let _base = EnvVarGuard::set("JCODE_ANTHROPIC_API_BASE", "https://one.example/v1");
    let _auth = EnvVarGuard::set("JCODE_ANTHROPIC_AUTH", "bearer");
    let _key_name = EnvVarGuard::set("JCODE_ANTHROPIC_API_KEY_NAME", "PROFILE_ONE_KEY");
    let _key = EnvVarGuard::set("PROFILE_ONE_KEY", "one-secret");
    let provider = AnthropicProvider::new();

    let _changed_base = EnvVarGuard::set("JCODE_ANTHROPIC_API_BASE", "https://two.example/v1");
    let _changed_key = EnvVarGuard::set("PROFILE_ONE_KEY", "two-secret");
    assert_eq!(
        provider.direct_transport.api_url,
        "https://one.example/v1/messages"
    );
    assert_eq!(provider.direct_transport.auth_mode, "bearer");
    assert_eq!(
        provider.profile_api_key.as_ref().unwrap().as_ref().unwrap(),
        "one-secret"
    );
}

#[test]
fn named_anthropic_profile_accepts_its_configured_custom_model() {
    let _lock = jcode_base::storage::lock_test_env();
    let _home = tempfile::TempDir::new().expect("temp home");
    let _home_guard = EnvVarGuard::set("JCODE_HOME", _home.path());
    std::fs::write(
        _home.path().join("config.toml"),
        r#"
        [providers.custom]
        type = "anthropic-compatible"
        base_url = "http://localhost:12345/v1"
        default_model = "claude-private"
        "#,
    )
    .expect("write config");
    jcode_base::config::Config::invalidate_cache();
    let _profile = EnvVarGuard::set("JCODE_NAMED_PROVIDER_PROFILE", "custom");
    let models = active_anthropic_profile_models().expect("active profile models");
    assert!(models.iter().any(|model| model == "claude-private"));
    assert!(!models.iter().any(|model| model == "not-configured"));
    drop(_profile);
    drop(_home_guard);
    jcode_base::config::Config::invalidate_cache();
}

async fn collect_live_smoke_stream(
    mut stream: EventStream,
    timeout: std::time::Duration,
) -> Result<(usize, usize, bool)> {
    tokio::time::timeout(timeout, async move {
        let mut text_bytes = 0usize;
        let mut thinking_bytes = 0usize;
        let mut saw_message_end = false;
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(text) => {
                    text_bytes += text.len();
                }
                StreamEvent::ThinkingDelta(text) => {
                    thinking_bytes += text.len();
                }
                StreamEvent::MessageEnd { .. } => {
                    saw_message_end = true;
                    break;
                }
                StreamEvent::Error { message, .. } => anyhow::bail!(message),
                _ => {}
            }
        }
        Ok((text_bytes, thinking_bytes, saw_message_end))
    })
    .await
    .context("live provider smoke timed out")?
}

#[test]
fn test_parse_sse_event() {
    let mut buffer = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n".to_string();
    let event = parse_sse_event(&mut buffer).unwrap();
    assert_eq!(event.event_type, "message_start");
    assert!(buffer.is_empty());
}

#[tokio::test]
async fn test_available_models() {
    let provider = AnthropicProvider::new();
    let models = provider.available_models();
    assert!(models.contains(&"claude-opus-4-8"));
    // Opus 4.8 is native-1M, so there is no redundant `[1m]` alias.
    assert!(!models.contains(&"claude-opus-4-8[1m]"));
    assert!(models.contains(&"claude-opus-4-6"));
    assert!(models.contains(&"claude-opus-4-6[1m]"));
    assert!(models.contains(&"claude-sonnet-4-6"));
    assert!(models.contains(&"claude-sonnet-4-6[1m]"));
    assert!(models.contains(&"claude-haiku-4-5"));
}

#[test]
fn test_effectively_1m_requires_explicit_suffix() {
    assert!(!effectively_1m("claude-opus-4-6"));
    assert!(!effectively_1m("claude-sonnet-4-6"));
    assert!(effectively_1m("claude-opus-4-6[1m]"));
    assert!(effectively_1m("claude-sonnet-4-6[1m]"));
}

#[test]
fn test_oauth_beta_headers_require_explicit_1m_suffix() {
    assert_eq!(oauth_beta_headers("claude-opus-4-6"), OAUTH_BETA_HEADERS);
    assert_eq!(
        oauth_beta_headers("claude-opus-4-6[1m]"),
        OAUTH_BETA_HEADERS_1M
    );
}

#[tokio::test]
async fn test_dangling_tool_use_repair() {
    let provider = AnthropicProvider::new();

    // Create messages with a dangling tool_use (no corresponding tool_result)
    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Let me check".to_string(),
                    cache_control: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_123".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                    thought_signature: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_456".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "/tmp/test"}),
                    thought_signature: None,
                },
            ],
            timestamp: None,
            tool_duration_ms: None,
        },
        // Missing tool_results for tool_123 and tool_456!
    ];

    let formatted = provider.format_messages(&messages, false);

    // Should have 3 messages:
    // 1. User: "Hello"
    // 2. Assistant: text + tool_uses
    // 3. User: synthetic tool_results for the dangling tool_uses
    assert_eq!(formatted.len(), 3);

    // Check the synthetic tool_result message
    let synthetic_msg = &formatted[2];
    assert_eq!(synthetic_msg.role, "user");
    assert_eq!(synthetic_msg.content.len(), 2);

    // Verify both tool_results are present
    let mut found_ids = std::collections::HashSet::new();
    for block in &synthetic_msg.content {
        if let ApiContentBlock::ToolResult {
            tool_use_id,
            is_error,
            content,
        } = block
        {
            found_ids.insert(tool_use_id.clone());
            assert!(is_error);
            match content {
                ToolResultContent::Text(t) => assert!(t.contains("interrupted")),
                ToolResultContent::Blocks(_) => panic!("Expected text content"),
            }
        } else {
            panic!("Expected ToolResult block");
        }
    }
    assert!(found_ids.contains("tool_123"));
    assert!(found_ids.contains("tool_456"));
}

#[tokio::test]
async fn test_no_repair_when_tool_results_present() {
    let provider = AnthropicProvider::new();

    // Create messages where tool_use has a corresponding tool_result
    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_123".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_123".to_string(),
                content: "file1.txt\nfile2.txt".to_string(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let formatted = provider.format_messages(&messages, false);

    // Should have exactly 3 messages (no synthetic ones added)
    assert_eq!(formatted.len(), 3);

    // The last message should be the actual tool_result, not synthetic
    let last_msg = &formatted[2];
    if let ApiContentBlock::ToolResult { content, .. } = &last_msg.content[0] {
        match content {
            ToolResultContent::Text(t) => assert!(t.contains("file1.txt")),
            ToolResultContent::Blocks(_) => panic!("Expected text content"),
        }
    } else {
        panic!("Expected ToolResult block");
    }
}

#[tokio::test]
async fn test_parallel_image_tool_results_stay_contiguous() {
    // Regression for Anthropic 400: "`tool_use` ids were found without `tool_result`
    // blocks immediately after". When the assistant issues several parallel `read`
    // calls that return images, each tool result is stored as its own user message in
    // the form [tool_result, image, "[Attached image ...]" text]. After merging the
    // consecutive user messages, the sibling label text blocks were wedged between the
    // tool_results, which Anthropic rejects. The label must be folded into the
    // tool_result content so every tool_result stays contiguous.
    let provider = AnthropicProvider::new();

    let make_image_result = |id: &str, label: &str| Message {
        role: Role::User,
        content: vec![
            ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: format!("Image: {label}"),
                is_error: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "AAAA".to_string(),
            },
            ContentBlock::Text {
                text: format!(
                    "[Attached image associated with the preceding tool result: {label}]"
                ),
                cache_control: None,
            },
        ],
        timestamp: None,
        tool_duration_ms: None,
    };

    let messages = vec![
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolUse {
                    id: "tool_a".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "a.png"}),
                    thought_signature: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_b".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "b.png"}),
                    thought_signature: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_c".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({"file_path": "c.png"}),
                    thought_signature: None,
                },
            ],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_image_result("tool_a", "a.png"),
        make_image_result("tool_b", "b.png"),
        make_image_result("tool_c", "c.png"),
    ];

    let formatted = provider.format_messages(&messages, false);

    // assistant message + merged user tool_result message
    assert_eq!(formatted.len(), 2);
    let user_msg = &formatted[1];
    assert_eq!(user_msg.role, "user");

    // Every block in the user message must be a tool_result (no sibling text blocks
    // wedged between them), and all three tool_use ids must be present.
    let mut seen = std::collections::HashSet::new();
    for block in &user_msg.content {
        match block {
            ApiContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                seen.insert(tool_use_id.clone());
                // Each image tool_result should carry its image and folded label text.
                match content {
                    ToolResultContent::Blocks(blocks) => {
                        assert!(
                            blocks
                                .iter()
                                .any(|b| matches!(b, ToolResultContentBlock::Image { .. })),
                            "image tool_result should contain an image block"
                        );
                        assert!(
                            blocks.iter().any(|b| matches!(
                                b,
                                ToolResultContentBlock::Text { text }
                                    if text.contains("[Attached image associated")
                            )),
                            "label text should be folded into the tool_result content"
                        );
                    }
                    ToolResultContent::Text(_) => {
                        panic!("image tool_result should use block content")
                    }
                }
            }
            _ => panic!("expected only tool_result blocks in the user message"),
        }
    }
    assert_eq!(
        seen,
        ["tool_a", "tool_b", "tool_c"]
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::HashSet<_>>()
    );
}

// --- Cross-turn cache correctness tests ---
// These tests verify the two-marker sliding-window strategy that allows each turn
// to READ from the previous turn's conversation cache.

fn count_message_cache_breakpoints(messages: &[ApiMessage]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| {
            matches!(
                b,
                ApiContentBlock::Text {
                    cache_control: Some(_),
                    ..
                } | ApiContentBlock::ToolUse {
                    cache_control: Some(_),
                    ..
                }
            )
        })
        .count()
}

fn cached_message_indices(messages: &[ApiMessage]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.content.iter().any(|b| {
                matches!(
                    b,
                    ApiContentBlock::Text {
                        cache_control: Some(_),
                        ..
                    } | ApiContentBlock::ToolUse {
                        cache_control: Some(_),
                        ..
                    }
                )
            })
        })
        .map(|(i, _)| i)
        .collect()
}

/// Helper to build a minimal conversation with N exchanges (user→assistant pairs).
/// Returns messages suitable for add_message_cache_breakpoint (includes a trailing user msg).
fn build_conversation(exchanges: usize) -> Vec<ApiMessage> {
    let mut messages = vec![ApiMessage {
        role: "user".to_string(),
        content: vec![ApiContentBlock::Text {
            text: "identity".to_string(),
            cache_control: None,
        }],
    }];
    for i in 0..exchanges {
        messages.push(ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: format!("Question {}", i + 1),
                cache_control: None,
            }],
        });
        messages.push(ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::Text {
                text: format!("Answer {}", i + 1),
                cache_control: None,
            }],
        });
    }
    // Trailing user message (the current turn's input)
    messages.push(ApiMessage {
        role: "user".to_string(),
        content: vec![ApiContentBlock::Text {
            text: format!("Question {}", exchanges + 1),
            cache_control: None,
        }],
    });
    messages
}

#[tokio::test]
async fn test_sanitize_tool_ids_with_dots() {
    let provider = AnthropicProvider::new();

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "chatcmpl-BF2xX.tool_call.0".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "chatcmpl-BF2xX.tool_call.0".to_string(),
                content: "file1.txt".to_string(),
                is_error: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let formatted = provider.format_messages(&messages, false);

    let sanitized_id = "chatcmpl-BF2xX_tool_call_0";
    for msg in &formatted {
        for block in &msg.content {
            match block {
                ApiContentBlock::ToolUse { id, .. } => {
                    assert_eq!(id, sanitized_id);
                }
                ApiContentBlock::ToolResult { tool_use_id, .. } => {
                    assert_eq!(tool_use_id, sanitized_id);
                }
                _ => {}
            }
        }
    }
}

#[tokio::test]
async fn test_sanitize_dangling_tool_ids_with_dots() {
    let provider = AnthropicProvider::new();

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "call.with.dots".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "crash"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let formatted = provider.format_messages(&messages, false);

    let sanitized_id = "call_with_dots";
    for msg in &formatted {
        for block in &msg.content {
            match block {
                ApiContentBlock::ToolUse { id, .. } => {
                    assert_eq!(id, sanitized_id);
                }
                ApiContentBlock::ToolResult { tool_use_id, .. } => {
                    assert_eq!(tool_use_id, sanitized_id);
                }
                _ => {}
            }
        }
    }
}

/// The runtime-provider identity that `set_credential_mode` writes must decode
/// back to the exact same credential mode. This guards the model picker / header
/// widget from reporting OAuth when an API key is in use (or vice versa): the
/// env key is the single source of truth those surfaces read, so an asymmetric
/// mapping here would surface an inaccurate auth method to the user.
#[test]
fn credential_mode_runtime_provider_identity_round_trips() {
    let _guard = jcode_base::storage::lock_test_env();
    let previous = std::env::var_os("JCODE_RUNTIME_PROVIDER");

    jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", "claude");
    assert_eq!(
        AnthropicCredentialMode::from_runtime_env(jcode_provider_core::DualAuthProvider::Anthropic),
        AnthropicCredentialMode::OAuth,
        "OAuth selection must surface as the OAuth runtime identity"
    );

    jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", "claude-api");
    assert_eq!(
        AnthropicCredentialMode::from_runtime_env(jcode_provider_core::DualAuthProvider::Anthropic),
        AnthropicCredentialMode::ApiKey,
        "API-key selection must surface as the API-key runtime identity"
    );

    match previous {
        Some(value) => jcode_base::env::set_var("JCODE_RUNTIME_PROVIDER", value),
        None => jcode_base::env::remove_var("JCODE_RUNTIME_PROVIDER"),
    }
}

#[tokio::test]
async fn auto_mode_falls_back_to_api_key_when_oauth_is_expired() {
    let _guard = jcode_base::storage::lock_test_env();
    let temp = tempfile::TempDir::new().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "test-anthropic-api-key");
    let _runtime = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "auto");

    jcode_base::auth::claude::upsert_account(jcode_base::auth::claude::AnthropicAccount {
        label: "claude-1".to_string(),
        access: "expired-oauth-access".to_string(),
        refresh: String::new(),
        expires: 0,
        email: None,
        subscription_type: Some("max".to_string()),
        scopes: vec!["user:inference".to_string()],
    })
    .unwrap();

    let provider = AnthropicProvider::new();
    assert_eq!(
        provider.credential_mode_snapshot(),
        AnthropicCredentialMode::Auto
    );

    let (token, is_oauth) = provider.get_access_token().await.unwrap();
    assert_eq!(token, "test-anthropic-api-key");
    assert!(
        !is_oauth,
        "automatic fallback must use API-key request semantics"
    );
}

#[tokio::test]
async fn explicit_oauth_mode_does_not_silently_fall_back_to_api_key() {
    let _guard = jcode_base::storage::lock_test_env();
    let temp = tempfile::TempDir::new().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "test-anthropic-api-key");
    let _runtime = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "claude");

    jcode_base::auth::claude::upsert_account(jcode_base::auth::claude::AnthropicAccount {
        label: "claude-1".to_string(),
        access: "expired-oauth-access".to_string(),
        refresh: String::new(),
        expires: 0,
        email: None,
        subscription_type: Some("max".to_string()),
        scopes: vec!["user:inference".to_string()],
    })
    .unwrap();

    let provider = AnthropicProvider::new();
    assert_eq!(
        provider.credential_mode_snapshot(),
        AnthropicCredentialMode::OAuth
    );

    let error = provider.get_access_token().await.unwrap_err().to_string();
    assert!(error.contains("expired"), "unexpected error: {error}");
}

#[test]
fn test_anthropic_fable_5_sends_reasoning_fields() {
    // `claude-fable-5` rejected reasoning fields during its preview, but the
    // released model accepts an adaptive `thinking` block and an
    // `output_config` effort (verified live 2026-07-01). The request builder
    // must send both when an effort is configured.
    let provider = AnthropicProvider::new();
    *provider.reasoning_effort.write().unwrap() = Some("high".to_string());

    let (thinking, output_config, temperature) =
        provider.build_reasoning_request_parts_inner("claude-fable-5", true, false);
    assert!(
        matches!(thinking, Some(ApiThinking::Adaptive { .. })),
        "Fable 5 should send an adaptive thinking block"
    );
    assert_eq!(
        output_config.as_ref().map(|c| c.effort.as_str()),
        Some("high"),
        "Fable 5 should send the configured output_config effort"
    );
    assert_eq!(temperature, None);

    // Fable 5 supports the real `max` API level, so `max` is sent verbatim.
    *provider.reasoning_effort.write().unwrap() = Some("max".to_string());
    let (_thinking, output_config, _temp) =
        provider.build_reasoning_request_parts_inner("claude-fable-5", true, false);
    assert_eq!(
        output_config.as_ref().map(|c| c.effort.as_str()),
        Some("max")
    );

    // The effort picker surfaces levels for Fable 5.
    assert!(AnthropicProvider::model_supports_reasoning_effort(
        "claude-fable-5"
    ));
}

#[test]
fn detects_anthropic_reasoning_unsupported_errors() {
    // The real 400 bodies returned when Fable 5 is sent reasoning fields.
    let thinking_400 = "anthropic api error (400 bad request): {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"adaptive thinking is not supported on this model\"}}";
    assert!(is_reasoning_unsupported_error(thinking_400));
    let effort_400 = "anthropic api error (400 bad request): {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"this model does not support the effort parameter.\"}}";
    assert!(is_reasoning_unsupported_error(effort_400));

    // Unrelated 400s must not trigger the reasoning self-heal path.
    assert!(!is_reasoning_unsupported_error(
        "anthropic api error (400 bad request): {\"type\":\"invalid_request_error\",\"message\":\"max_tokens too large\"}"
    ));
    // A thinking-mentioning error that is not a 400 must not match either.
    assert!(!is_reasoning_unsupported_error(
        "anthropic api error (429 too many requests): rate_limit on thinking budget"
    ));
    // Model-not-found is a different recovery path.
    assert!(!is_reasoning_unsupported_error(
        "anthropic api error (404 not found): {\"type\":\"not_found_error\",\"message\":\"model not found\"}"
    ));
}

#[test]
fn detects_anthropic_model_not_found_errors() {
    // The real 404 body returned when a model id was retired (e.g. Fable 5).
    let real = "anthropic api error (404 not found): {\"type\":\"error\",\"error\":{\"type\":\"not_found_error\",\"message\":\"claude fable 5 is not available. please use opus 4.8.\"}}";
    assert!(is_model_not_found_error(real));

    // Structural marker alone (lowercased error chain).
    assert!(is_model_not_found_error(
        "model claude-foo not found (not_found_error)"
    ));

    // Unrelated failures must not trigger the model fallback path.
    assert!(!is_model_not_found_error(
        "anthropic api error (401 unauthorized): invalid authentication credentials"
    ));
    assert!(!is_model_not_found_error(
        "anthropic api error (429 too many requests): rate_limit"
    ));
    assert!(!is_model_not_found_error(
        "anthropic api error (404 not found): resource missing"
    ));
}

#[test]
fn anthropic_fallback_prefers_best_available_and_skips_tried_and_retired() {
    // The fallback logic reads the process-global model catalog; lock and
    // reset it so fixture models hydrated by other tests cannot leak in.
    let _guard = jcode_base::storage::lock_test_env();
    jcode_base::provider::models::reset_model_catalog_services_for_tests();
    let known = jcode_base::provider::known_anthropic_model_ids();
    assert!(
        !known.is_empty(),
        "expected a non-empty Anthropic model catalog"
    );

    // With nothing tried, the fallback offers the highest-quality (flagship)
    // model, NOT merely the first catalog entry. The curated order ranks Opus
    // ahead of Haiku, so the chosen model must not be a Haiku/retired tier when
    // a stronger one exists.
    let first = anthropic_fallback_model(&[], "").expect("a fallback should exist");
    let first_key = AnthropicProvider::normalized_model_key(&first);
    assert!(
        !first_key.contains("haiku"),
        "fallback must not downgrade to Haiku when a flagship is available, got {first}"
    );
    assert!(
        !anthropic_model_is_retired(&first),
        "fallback must never pick a retired model, got {first}"
    );

    // A retired model in `tried` must never be re-offered, and the result must
    // skip retired families entirely.
    let next = anthropic_fallback_model(&["claude-mythos-1".to_string()], "")
        .expect("another fallback should exist");
    assert!(!anthropic_model_is_retired(&next));

    // Exhausting every viable known model yields None.
    let exhausted = anthropic_fallback_model(&known, "");
    assert!(
        exhausted.is_none(),
        "no fallback should remain once all known models are tried, got {exhausted:?}"
    );
}

#[test]
fn anthropic_fallback_honors_server_recommendation() {
    // The recommendation matcher scores hints against the process-global model
    // catalog; lock and reset it so fixture models hydrated by other tests
    // (e.g. claude-opus-5-preview) cannot outrank the real catalog entries.
    let _guard = jcode_base::storage::lock_test_env();
    jcode_base::provider::models::reset_model_catalog_services_for_tests();
    // The real 404 body recommends a specific replacement model. We must honor
    // it over the generic quality ranking.
    let body = "anthropic api error (404 not found): {\"type\":\"error\",\"error\":{\"type\":\"not_found_error\",\"message\":\"claude fable 5 is not available. please use opus 4.8. learn more: https://anthropic.com\"}}";
    let recommended =
        anthropic_recommended_model_from_error(body).expect("should parse a recommendation");
    assert_eq!(
        AnthropicProvider::normalized_model_key(&recommended),
        "claude-opus-4-8",
        "server recommendation 'Opus 4.8' should map to claude-opus-4-8"
    );

    // The full fallback also returns the recommended model.
    let fallback = anthropic_fallback_model(&["claude-mythos-1".to_string()], body)
        .expect("a fallback should exist");
    assert_eq!(
        AnthropicProvider::normalized_model_key(&fallback),
        "claude-opus-4-8"
    );

    // A recommendation pointing at a retired model is ignored (falls through to
    // quality ranking).
    let retired_rec = "model x not available. please use mythos 1.";
    assert!(
        anthropic_recommended_model_from_error(retired_rec).is_none()
            || !anthropic_model_is_retired(
                &anthropic_recommended_model_from_error(retired_rec).unwrap()
            )
    );

    // No recommendation phrase -> None.
    assert!(anthropic_recommended_model_from_error("429 too many requests").is_none());
}

#[test]
fn anthropic_quality_rank_orders_opus_before_haiku_and_retired_last() {
    let opus = anthropic_model_quality_rank("claude-opus-4-8");
    let sonnet = anthropic_model_quality_rank("claude-sonnet-4-6");
    let haiku = anthropic_model_quality_rank("claude-haiku-4-5");
    let retired = anthropic_model_quality_rank("claude-mythos-1");
    // Fable 5 is live again and curated as the flagship, so it ranks first.
    let fable = anthropic_model_quality_rank("claude-fable-5");
    assert!(
        fable <= opus,
        "Fable 5 should rank at or ahead of Opus ({fable} vs {opus})"
    );
    assert!(
        opus < sonnet,
        "Opus should outrank Sonnet ({opus} vs {sonnet})"
    );
    assert!(
        sonnet < haiku,
        "Sonnet should outrank Haiku ({sonnet} vs {haiku})"
    );
    assert!(
        haiku < retired,
        "retired models must sort last ({haiku} vs {retired})"
    );
    assert_eq!(retired, usize::MAX);
    // Dated live ids must rank like their canonical base.
    assert_eq!(
        anthropic_model_quality_rank("claude-haiku-4-5-20251001"),
        haiku
    );
}

#[test]
fn fable_quota_fallback_selects_the_best_available_opus() {
    let fallback = AnthropicProvider::best_available_opus_model("claude-fable-5")
        .expect("the curated Anthropic catalog should contain an Opus fallback");
    assert!(
        fallback.contains("claude-opus"),
        "unexpected fallback: {fallback}"
    );

    let candidates = jcode_base::provider::cached_anthropic_model_ids()
        .unwrap_or_else(jcode_base::provider::known_anthropic_model_ids);
    let best_rank = candidates
        .iter()
        .filter(|model| model.to_ascii_lowercase().contains("claude-opus"))
        .filter(|model| !anthropic_model_is_retired(model))
        .map(|model| anthropic_model_quality_rank(model))
        .min()
        .expect("available Opus model");
    assert_eq!(anthropic_model_quality_rank(&fallback), best_rank);
}

#[test]
fn model_scoped_usage_routes_only_exhausted_fable_to_opus() {
    let usage = jcode_base::usage::UsageData {
        model_scoped: vec![jcode_base::usage::ModelScopedUsageWindow {
            model_name: "Fable".to_string(),
            utilization: 1.0,
            resets_at: Some("2026-08-11T00:00:00Z".to_string()),
        }],
        ..Default::default()
    };
    let fallback = AnthropicProvider::fallback_for_model_scoped_usage("claude-fable-5", &usage)
        .expect("exhausted Fable should route to Opus");
    assert!(
        fallback.contains("claude-opus"),
        "unexpected fallback: {fallback}"
    );
    assert!(
        AnthropicProvider::fallback_for_model_scoped_usage("claude-opus-5", &usage).is_none(),
        "an exhausted Fable scope must not reroute an explicitly selected Opus"
    );

    let available = jcode_base::usage::UsageData {
        model_scoped: vec![jcode_base::usage::ModelScopedUsageWindow {
            model_name: "Fable".to_string(),
            utilization: 0.98,
            resets_at: None,
        }],
        ..Default::default()
    };
    assert!(
        AnthropicProvider::fallback_for_model_scoped_usage("claude-fable-5", &available).is_none(),
        "Fable must remain selected while its scoped quota is available"
    );
}

#[test]
fn detects_live_fable_scoped_limit_errors_without_misrouting_other_limits() {
    assert!(is_fable_scoped_limit_error(
        "claude-fable-5",
        r#"429 {"type":"rate_limit_error","message":"You have reached your weekly Fable limit"}"#,
    ));
    assert!(is_fable_scoped_limit_error(
        "claude-fable-5",
        "usage limit reached for the 7-day model window",
    ));
    assert!(!is_fable_scoped_limit_error(
        "claude-opus-5",
        "weekly Fable rate limit reached",
    ));
    assert!(!is_fable_scoped_limit_error(
        "claude-fable-5",
        "429 overloaded_error: service temporarily overloaded",
    ));
    assert!(!is_fable_scoped_limit_error(
        "claude-fable-5",
        "global 5-hour rate limit reached",
    ));
}

#[test]
fn ping_keepalive_emits_streaming_phase_event() {
    // Issue #451: during silent reasoning phases, `ping` events can be the
    // only upstream traffic. They must surface as a StreamEvent so the client
    // stall guard sees activity instead of cancelling a healthy stream.
    let mut state = SseStreamState::default();
    let event = SseEvent {
        event_type: "ping".to_string(),
        data: r#"{"type": "ping"}"#.to_string(),
    };
    let events = process_sse_event(&event, &mut state, true);
    assert!(
        events.iter().any(|e| matches!(
            e,
            StreamEvent::ConnectionPhase {
                phase: jcode_message_types::ConnectionPhase::Streaming
            }
        )),
        "expected ping to emit a Streaming ConnectionPhase event, got {events:?}"
    );
}

#[test]
fn test_anthropic_opus_5_low_effort_reaches_the_wire() {
    // Benchmark campaigns pin `claude-opus-5` at `low` effort. Opus 5 also
    // *defaults* to `low` (jcode's default model/effort pairing), and an
    // explicit `low` must survive normalization, must NOT be silently
    // promoted, and must land in `output_config.effort` on the request.
    assert!(AnthropicProvider::model_supports_output_effort(
        "claude-opus-5"
    ));
    assert_eq!(
        AnthropicProvider::default_reasoning_effort_for_model("claude-opus-5").as_deref(),
        Some("low"),
    );
    assert_eq!(
        AnthropicProvider::normalize_reasoning_effort("low").as_deref(),
        Some("low"),
    );
    // Downward selection is never clamped upward toward the model default.
    assert_eq!(
        AnthropicProvider::actual_effort_for_model("claude-opus-5", "low"),
        "low",
    );
    assert_eq!(
        AnthropicProvider::store_effort_for_model("claude-opus-5", "low"),
        "low",
    );

    let provider = AnthropicProvider::new();
    *provider
        .model
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = "claude-opus-5".to_string();
    provider.set_reasoning_effort("low").unwrap();
    assert_eq!(provider.reasoning_effort().as_deref(), Some("low"));

    let (thinking, output_config, _temp) =
        provider.build_reasoning_request_parts_inner("claude-opus-5", true, false);
    assert_eq!(
        output_config
            .expect("explicit low effort should set output_config")
            .effort,
        "low",
    );
    // Opus 5 rejects `thinking.type.enabled`; it requires adaptive thinking.
    assert!(matches!(thinking, Some(ApiThinking::Adaptive { .. })));
}

/// A `content_block_start` carrying an unrecognized block type must still
/// deserialize. Before the `Unknown` catch-all the whole event failed to parse
/// and was dropped, so an unknown *tool* block produced a turn that reported
/// `stop_reason: tool_use` with no tool call for the agent to run.
#[test]
fn test_anthropic_unknown_content_block_start_does_not_drop_event() {
    for block_type in [
        "server_tool_use",
        "web_search_tool_result",
        "some_future_block",
    ] {
        let mut state = SseStreamState::default();
        let event = SseEvent {
            event_type: "content_block_start".to_string(),
            data: serde_json::json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": block_type, "id": "srvtoolu_1", "name": "web_search"}
            })
            .to_string(),
        };
        let events = process_sse_event(&event, &mut state, false);
        assert!(
            events.is_empty(),
            "{block_type}: unknown block must not synthesize stream events"
        );
        assert!(
            state.current_tool_use.is_none(),
            "{block_type}: unknown block must not start tool accumulation"
        );
        assert!(
            !state.current_thinking_block,
            "{block_type}: unknown block must not start a thinking block"
        );
    }
}

#[path = "role_pinned_tests.rs"]
mod role_pinned_tests;

include!("anthropic_tests/reasoning.rs");
include!("anthropic_tests/prompt_cache.rs");

#[cfg(unix)]
#[test]
fn invalid_unicode_direct_configuration_is_retained_as_redacted_error() {
    use std::os::unix::ffi::OsStringExt;

    let _lock = jcode_base::storage::lock_test_env();
    for name in [
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
    ] {
        let mut invalid = b"private-config-sentinel".to_vec();
        invalid.push(0xff);
        let _value = EnvVarGuard::set(name, std::ffi::OsString::from_vec(invalid));
        let config = DirectTransportConfig::from_env();
        let error = config
            .headers
            .expect_err("invalid explicit config must block direct requests");
        assert!(error.contains(name));
        assert!(!error.contains("private-config-sentinel"));
    }
}
