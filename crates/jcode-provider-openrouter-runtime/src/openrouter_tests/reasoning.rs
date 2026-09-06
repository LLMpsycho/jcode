/// Regression for issue #321: when an assistant turn is interrupted mid-thinking
/// on a direct OpenAI-compatible provider that does not support reasoning replay
/// (e.g. DeepSeek), the persisted assistant message contains only a `Reasoning`
/// block. The request builder must not emit an assistant message that has
/// neither `content` nor `tool_calls`, otherwise the provider rejects the whole
/// request with 400 "Invalid assistant message: content or tool_calls must be
/// set" and the session can never recover.
#[test]
fn interrupted_reasoning_only_assistant_message_is_not_sent_empty() {
    let _lock = ENV_LOCK.lock();
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        profile_id: Some("deepseek".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "do a thing".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        // Assistant turn that was interrupted while only reasoning had streamed,
        // so it carries a Reasoning block but no text or tool calls.
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Reasoning {
                text: "thinking about the request".to_string(),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "actually do this instead".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    let body = parse_captured_request_body(&request);
    let api_messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("request should contain messages array");

    for msg in api_messages {
        if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let has_content = msg
            .get("content")
            .map(|v| !v.is_null() && v.as_str().map(|s| !s.is_empty()).unwrap_or(true))
            .unwrap_or(false);
        let has_tool_calls = msg
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|calls| !calls.is_empty())
            .unwrap_or(false);
        assert!(
            has_content || has_tool_calls,
            "assistant message must carry content or tool_calls (issue #321); got: {msg}"
        );
    }
}
/// Companion to issue #321: when the provider *does* support reasoning replay
/// (e.g. a generic OpenRouter-style endpoint with provider features enabled and
/// thinking on), an interrupted reasoning-only assistant turn should be sent
/// with both a `reasoning_content` field and a valid (empty) `content`, so the
/// turn is preserved without violating the "content or tool_calls" requirement.
#[test]
fn interrupted_reasoning_only_assistant_message_keeps_reasoning_with_content() {
    let _lock = ENV_LOCK.lock();
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        profile_id: None,
        supports_provider_features: true,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "do a thing".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Reasoning {
                text: "thinking about the request".to_string(),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "actually do this instead".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    let body = parse_captured_request_body(&request);
    let api_messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("request should contain messages array");

    let assistant = api_messages
        .iter()
        .find(|msg| msg.get("role").and_then(|v| v.as_str()) == Some("assistant"))
        .expect("request should retain the interrupted assistant turn");

    assert!(
        assistant.get("reasoning_content").is_some(),
        "reasoning-capable provider should keep reasoning_content; got: {assistant}"
    );
    assert!(
        assistant.get("content").is_some(),
        "interrupted reasoning-only assistant turn must still carry content (issue #321); got: {assistant}"
    );
}
/// Regression for issue #322: the dedicated Kimi coding endpoint
/// (`https://api.kimi.com/coding/v1`, model `kimi-for-coding`) enables thinking
/// server-side and rejects any assistant tool-call message that lacks
/// `reasoning_content` with 400 "thinking is enabled but reasoning_content is
/// missing in assistant tool call message". When an assistant turn produced a
/// tool call without an accompanying reasoning block (the common case once the
/// thinking stream is not persisted), the request builder must still attach a
/// `reasoning_content` field to that assistant message so the endpoint accepts
/// the request.
#[test]
fn kimi_for_coding_tool_call_message_includes_reasoning_content() {
    let _lock = ENV_LOCK.lock();
    let _thinking = EnvVarGuard::remove("JCODE_OPENROUTER_THINKING");
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        // The dedicated Kimi coding endpoint is a direct OpenAI-compatible
        // profile (no OpenRouter provider routing features).
        profile_id: Some("kimi".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        model: Arc::new(RwLock::new("kimi-for-coding".to_string())),
        ..make_custom_compatible_provider()
    };

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "list the files".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        // Assistant turn that emitted a tool call but whose hidden reasoning was
        // not persisted (so there is no Reasoning block to replay).
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
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
                tool_use_id: "call_1".to_string(),
                content: "a.txt\nb.txt".to_string(),
                is_error: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    let body = parse_captured_request_body(&request);
    let api_messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("request should contain messages array");

    let assistant = api_messages
        .iter()
        .find(|msg| {
            msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
                && msg.get("tool_calls").is_some()
        })
        .expect("request should retain the assistant tool-call turn");

    let reasoning = assistant.get("reasoning_content");
    assert!(
        reasoning.is_some_and(|value| value.as_str().is_some_and(|s| !s.is_empty())),
        "Kimi coding endpoint requires reasoning_content on assistant tool-call messages (issue #322); got: {assistant}"
    );
}
/// Regression for issue #815: DeepSeek-family models on direct
/// OpenAI-compatible profiles require the reasoning returned alongside an
/// assistant tool call to be replayed on the next request. These routes do not
/// enable OpenRouter provider features, so model-family detection must unlock
/// the stored `reasoning_content` without adding a top-level thinking config.
#[test]
fn direct_compatible_deepseek_tool_call_replays_reasoning_content() {
    let _lock = ENV_LOCK.lock();
    let _thinking = EnvVarGuard::remove("JCODE_OPENROUTER_THINKING");
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        profile_id: Some("opencode-zen".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        model: Arc::new(RwLock::new("deepseek-v4-flash-free".to_string())),
        ..make_custom_compatible_provider()
    };

    let messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "list the files".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Reasoning {
                    text: "I should inspect the workspace first.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                    thought_signature: None,
                },
            ],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "a.txt\nb.txt".to_string(),
                is_error: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    let body = parse_captured_request_body(&request);
    let assistant = body["messages"]
        .as_array()
        .expect("request should contain messages array")
        .iter()
        .find(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("assistant")
                && message.get("tool_calls").is_some()
        })
        .expect("request should retain the assistant tool-call turn");

    assert_eq!(
        assistant
            .get("reasoning_content")
            .and_then(|value| value.as_str()),
        Some("I should inspect the workspace first."),
        "direct DeepSeek request must replay stored reasoning_content (issue #815): {assistant}"
    );
    assert!(
        body.get("thinking").is_none(),
        "server-managed thinking must not add OpenRouter's top-level thinking field: {body}"
    );
}
#[test]
fn direct_deepseek_profile_exposes_max_reasoning_effort() {
    let provider = OpenRouterProvider {
        profile_id: Some("deepseek".to_string()),
        supports_provider_features: false,
        ..make_custom_compatible_provider()
    };

    assert_eq!(
        provider.available_efforts(),
        vec![
            "none",
            "low",
            "medium",
            "high",
            "max",
            "swarm",
            "swarm-deep"
        ]
    );
    provider
        .set_reasoning_effort("max")
        .expect("DeepSeek direct profile should accept max effort");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("max"));
}
#[test]
fn direct_zai_profile_exposes_openai_reasoning_effort_ladder() {
    let provider = OpenRouterProvider {
        profile_id: Some("zai".to_string()),
        supports_provider_features: false,
        ..make_custom_compatible_provider()
    };

    assert_eq!(
        provider.available_efforts(),
        jcode_provider_core::OPENAI_SELECTABLE_EFFORTS
    );
    provider
        .set_reasoning_effort("xhigh")
        .expect("Z.AI Coding Plan should accept xhigh effort");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("xhigh"));
}
#[test]
fn direct_zai_profile_applies_configured_effort_on_construction_and_model_switch() {
    let configured = jcode_base::config::config()
        .provider
        .openai_reasoning_effort
        .as_deref()
        .and_then(OpenRouterProvider::normalize_openai_reasoning_effort);
    assert_eq!(
        OpenRouterProvider::initial_reasoning_effort(None, Some("zai")),
        configured
    );

    let provider = OpenRouterProvider {
        profile_id: Some("zai".to_string()),
        supports_provider_features: false,
        reasoning_effort: Arc::new(RwLock::new(None)),
        ..make_custom_compatible_provider()
    };
    provider.set_model("glm-5.3-flash").unwrap();
    assert_eq!(provider.reasoning_effort(), configured);
}
#[test]
fn openrouter_profile_exposes_unified_reasoning_effort() {
    let provider = make_provider();

    assert_eq!(
        provider.available_efforts(),
        vec![
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "swarm",
            "swarm-deep"
        ]
    );
    provider
        .set_reasoning_effort("minimal")
        .expect("OpenRouter minimal effort should be accepted");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("minimal"));
    provider
        .set_reasoning_effort("max")
        .expect("OpenRouter max alias should be accepted");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("xhigh"));
}
#[test]
fn openrouter_with_openrouter_profile_id_exposes_unified_reasoning_effort() {
    // The default OpenRouter api base matches the "openrouter" OpenAI-compat
    // doctor profile, so `new()` can assign profile_id = Some("openrouter").
    // That runtime is still real OpenRouter and must keep unified reasoning
    // (regression: /effort failed with "Reasoning effort is not supported").
    let provider = OpenRouterProvider {
        profile_id: Some("openrouter".to_string()),
        ..make_provider()
    };

    assert_eq!(
        provider.available_efforts(),
        vec![
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "swarm",
            "swarm-deep"
        ]
    );
    provider
        .set_reasoning_effort("high")
        .expect("OpenRouter with doctor profile id should accept effort");
    assert_eq!(provider.reasoning_effort().as_deref(), Some("high"));
}
#[test]
fn non_deepseek_compatible_profile_does_not_expose_reasoning_effort() {
    let provider = make_custom_compatible_provider();

    assert!(provider.available_efforts().is_empty());
    let error = provider
        .set_reasoning_effort("max")
        .expect_err("generic compatible profile should not expose DeepSeek effort UX");
    assert!(
        error.to_string().contains("not supported"),
        "unexpected error: {error:?}"
    );
}
#[test]
fn openrouter_chat_request_sends_unified_reasoning_effort() {
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        model: Arc::new(RwLock::new("anthropic/claude-sonnet-4.6".to_string())),
        supports_model_catalog: false,
        ..make_provider()
    };
    provider
        .set_reasoning_effort("high")
        .expect("OpenRouter unified reasoning should accept high effort");

    let messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            event.expect("stream event should parse");
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.contains(r#""reasoning":{"effort":"high"}"#),
        "OpenRouter request should include unified reasoning effort: {request}"
    );
    assert!(
        !request.contains(r#""thinking":{"type":"enabled"}"#),
        "unified reasoning should supersede legacy thinking override: {request}"
    );
}
#[tokio::test]
#[ignore = "live smoke: requires OPENROUTER_API_KEY or configured OpenRouter credentials"]
async fn live_openrouter_unified_reasoning_smoke() -> Result<()> {
    let _env_lock = ENV_LOCK.lock();
    let Some(token) = OpenRouterProvider::get_api_key() else {
        eprintln!(
            "skipping live OpenRouter smoke: OPENROUTER_API_KEY or configured OpenRouter credentials not found"
        );
        return Ok(());
    };

    let models = live_openrouter_models();
    let effort = std::env::var("JCODE_LIVE_OPENROUTER_REASONING_EFFORT")
        .unwrap_or_else(|_| "low".to_string());
    let max_tokens = std::env::var("JCODE_LIVE_OPENROUTER_MAX_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(1024);

    for model in models {
        let provider = OpenRouterProvider {
            auth: ProviderAuth::AuthorizationBearer {
                token: token.clone(),
                label: configured_api_key_name(),
            },
            model: Arc::new(RwLock::new(model.clone())),
            max_tokens: Some(max_tokens),
            ..make_provider()
        };
        provider.set_reasoning_effort(&effort)?;

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "Live smoke test: answer exactly OK.".to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        }];

        let stream = provider
            .complete(
                &messages,
                &[],
                "You are a live provider smoke test. Keep the answer tiny.",
                None,
            )
            .await
            .with_context(|| format!("starting live OpenRouter stream for {model}"))?;
        let (text_bytes, thinking_bytes, saw_message_end) =
            collect_openrouter_live_smoke_stream(stream, Duration::from_secs(90))
                .await
                .with_context(|| format!("collecting live OpenRouter stream for {model}"))?;

        eprintln!(
            "live OpenRouter reasoning smoke passed: model={model}, effort={effort}, text_bytes={text_bytes}, thinking_bytes={thinking_bytes}, message_end={saw_message_end}"
        );
        assert!(
            text_bytes > 0 || thinking_bytes > 0,
            "live OpenRouter response for {model} contained neither text nor thinking deltas"
        );
    }

    Ok(())
}
#[test]
fn direct_deepseek_chat_request_sends_reasoning_effort() {
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        model: Arc::new(RwLock::new("deepseek-v4-pro".to_string())),
        profile_id: Some("deepseek".to_string()),
        supports_provider_features: false,
        supports_model_catalog: false,
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };
    provider
        .set_reasoning_effort("max")
        .expect("DeepSeek direct profile should accept max effort");

    let messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            event.expect("stream event should parse");
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.starts_with("POST /v1/chat/completions "),
        "unexpected chat request: {request}"
    );
    assert!(
        request.contains(r#""model":"deepseek-v4-pro""#),
        "request should contain model: {request}"
    );
    assert!(
        request.contains(r#""reasoning_effort":"max""#),
        "DeepSeek request should include max reasoning effort: {request}"
    );
}
#[test]
fn direct_openai_compatible_chat_request_preserves_max_reasoning_effort() {
    let (api_base, request_rx) = spawn_single_response_chat_server();
    let provider = OpenRouterProvider {
        api_base,
        model: Arc::new(RwLock::new("gpt-5.5".to_string())),
        supports_provider_features: false,
        supports_model_catalog: false,
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        ..make_custom_compatible_provider()
    };
    provider
        .set_reasoning_effort("max")
        .expect("direct OpenAI-compatible profile should accept max effort");

    let messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut stream = provider
            .complete(&messages, &[], "", None)
            .await
            .expect("fake chat request should start");
        while let Some(event) = stream.next().await {
            event.expect("stream event should parse");
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    assert!(
        request.contains(r#""reasoning_effort":"max""#),
        "direct compatible request must preserve OpenAI max: {request}"
    );
}
/// Issue #352: reasoning effort must follow the *model family*, not just the
/// dedicated `deepseek` profile id. A custom compat endpoint (named profile or
/// generic openai-compatible) serving a DeepSeek model supports `/effort`.
#[test]
fn compat_profile_serving_deepseek_model_supports_reasoning_effort() {
    let provider = make_custom_compatible_provider();

    // Non-DeepSeek model on a custom endpoint: no effort support.
    provider.set_model("some-random-model").unwrap();
    assert!(provider.available_efforts().is_empty());
    assert!(provider.set_reasoning_effort("high").is_err());
    assert_eq!(provider.reasoning_effort(), None);

    // DeepSeek-family model: DeepSeek-style efforts become available.
    provider.set_model("deepseek-v4-flash").unwrap();
    assert_eq!(
        provider.available_efforts(),
        vec![
            "none",
            "low",
            "medium",
            "high",
            "max",
            "swarm",
            "swarm-deep"
        ]
    );
    provider
        .set_reasoning_effort("high")
        .expect("deepseek model on compat endpoint accepts effort");
    assert_eq!(provider.reasoning_effort(), Some("high".to_string()));
}
/// GPT-family reasoning models served by a direct OpenAI-compatible gateway
/// (e.g. OpenCode Zen's `gpt-5.3-codex-spark`) accept the standard OpenAI
/// `reasoning_effort` field, so the effort command must work for them.
#[test]
fn compat_profile_serving_gpt_family_model_supports_reasoning_effort() {
    let provider = make_custom_compatible_provider();

    for model in [
        "gpt-5.3-codex-spark",
        "gpt-5.5",
        "gpt-5.1-codex-mini",
        "o1",
        "o5-mini",
    ] {
        provider.set_model(model).unwrap();
        assert_eq!(
            provider.available_efforts(),
            vec![
                "none",
                "minimal",
                "low",
                "medium",
                "high",
                "xhigh",
                "max",
                "swarm",
                "swarm-deep"
            ],
            "{model} should expose OpenAI effort vocabulary"
        );
        provider
            .set_reasoning_effort("high")
            .unwrap_or_else(|e| panic!("{model} on compat endpoint accepts effort: {e}"));
        assert_eq!(provider.reasoning_effort(), Some("high".to_string()));
        // A direct compatible endpoint receives OpenAI's real max value.
        provider.set_reasoning_effort("max").unwrap();
        assert_eq!(provider.reasoning_effort(), Some("max".to_string()));
    }

    // Explicit config override still wins in the off direction.
    let force_off = OpenRouterProvider {
        reasoning_effort_support: Some(false),
        ..make_custom_compatible_provider()
    };
    force_off.set_model("gpt-5.3-codex-spark").unwrap();
    assert!(force_off.available_efforts().is_empty());
    assert!(force_off.set_reasoning_effort("high").is_err());
}
#[test]
fn compatible_model_switch_clears_an_effort_invalid_for_the_new_vocabulary() {
    let provider = make_custom_compatible_provider();
    provider.set_model("gpt-5.5").unwrap();
    provider.set_reasoning_effort("minimal").unwrap();
    provider.set_model("deepseek-v4").unwrap();
    assert_eq!(provider.reasoning_effort(), None);
    assert!(
        provider.set_reasoning_effort("minimal").is_err(),
        "DeepSeek must reject rather than silently promote minimal to max"
    );
}
/// Issue #352: named-profile config can override effort support explicitly in
/// both directions.
#[test]
fn named_profile_supports_reasoning_effort_config_override() {
    let force_on = OpenRouterProvider {
        reasoning_effort_support: Some(true),
        ..make_custom_compatible_provider()
    };
    force_on.set_model("not-a-deepseek-model").unwrap();
    assert_eq!(
        force_on.available_efforts(),
        vec![
            "none",
            "low",
            "medium",
            "high",
            "max",
            "swarm",
            "swarm-deep"
        ]
    );
    force_on
        .set_reasoning_effort("medium")
        .expect("explicit supports_reasoning_effort=true enables effort");
    assert_eq!(force_on.reasoning_effort(), Some("medium".to_string()));

    let force_off = OpenRouterProvider {
        reasoning_effort_support: Some(false),
        ..make_custom_compatible_provider()
    };
    force_off.set_model("deepseek-v4-flash").unwrap();
    assert!(force_off.available_efforts().is_empty());
    assert!(
        force_off.set_reasoning_effort("high").is_err(),
        "explicit supports_reasoning_effort=false suppresses model auto-detection"
    );
}
/// Issue #352: named profiles construct with the user's configured
/// `openai_reasoning_effort` when the profile supports effort, instead of
/// silently ignoring the config.
#[test]
fn named_profile_construction_reads_openai_reasoning_effort_config() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let config = jcode_base::config::NamedProviderConfig {
        base_url: "https://compat.example.test/v1".to_string(),
        api_key: Some("test".to_string()),
        default_model: Some("deepseek-v4".to_string()),
        supports_reasoning_effort: Some(true),
        ..Default::default()
    };

    let provider =
        OpenRouterProvider::new_named_openai_compatible("custom", &config).expect("provider");
    // The config default is only applied when openai_reasoning_effort is set;
    // with no config value the provider starts with no effort but still
    // supports setting one.
    let initial = provider.reasoning_effort();
    let configured = jcode_base::config::config()
        .provider
        .openai_reasoning_effort
        .clone();
    match configured {
        Some(_) => assert!(initial.is_some(), "configured effort must be honored"),
        None => assert_eq!(initial, None),
    }
    provider
        .set_reasoning_effort("max")
        .expect("explicitly-enabled profile accepts effort");
}
#[test]
fn named_profile_can_disable_reasoning_model_name_heuristics() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let config = jcode_base::config::NamedProviderConfig {
        base_url: "https://compat.example.test/v1".to_string(),
        api_key: Some("test".to_string()),
        default_model: Some("gpt-5.5-enterprise".to_string()),
        disable_reasoning_heuristics: true,
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("enterprise", &config).unwrap();
    assert!(provider.available_efforts().is_empty());
    assert!(provider.set_reasoning_effort("high").is_err());
}
#[test]
fn named_profile_model_reasoning_overrides_capability_and_default_effort() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");
    let config = jcode_base::config::NamedProviderConfig {
        base_url: "https://compat.example.test/v1".to_string(),
        api_key: Some("test".to_string()),
        default_model: Some("reasoning-custom".to_string()),
        disable_reasoning_heuristics: true,
        models: vec![
            jcode_base::config::NamedProviderModelConfig {
                id: "reasoning-custom".to_string(),
                reasoning: Some(true),
                reasoning_effort: Some("high".to_string()),
                ..Default::default()
            },
            jcode_base::config::NamedProviderModelConfig {
                id: "gpt-5-disabled".to_string(),
                reasoning: Some(false),
                ..Default::default()
            },
            jcode_base::config::NamedProviderModelConfig {
                id: "reasoning-mini".to_string(),
                reasoning: Some(true),
                reasoning_effort: Some("low".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("custom", &config).unwrap();
    assert_eq!(provider.reasoning_effort(), Some("high".to_string()));
    assert!(provider.available_efforts().contains(&"xhigh"));

    provider.set_model("gpt-5-disabled").unwrap();
    assert!(provider.available_efforts().is_empty());
    assert_eq!(provider.reasoning_effort(), None);

    provider.set_model("reasoning-mini").unwrap();
    assert_eq!(provider.reasoning_effort(), Some("low".to_string()));
}
