use super::*;
use bytes::Bytes;
use futures::StreamExt;
use jcode_provider_openrouter::stream::OpenRouterStream;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

pub(crate) struct SharedEnvLock;

pub(crate) static ENV_LOCK: SharedEnvLock = SharedEnvLock;

impl SharedEnvLock {
    /// Acquire the process-global test env lock.
    ///
    /// This recovers from a poisoned mutex (`into_inner`) instead of
    /// propagating the `PoisonError`. The env guard only protects shared
    /// process env state, so a panic in one test must not cascade into a
    /// flood of unrelated `PoisonError` failures across every other test
    /// that takes this lock.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'static, ()> {
        jcode_base::storage::lock_test_env()
    }
}

pub(crate) struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        jcode_base::env::set_var(key, value);
        Self { key, previous }
    }

    pub(crate) fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        jcode_base::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            jcode_base::env::set_var(self.key, previous);
        } else {
            jcode_base::env::remove_var(self.key);
        }
    }
}

fn test_config_dir(temp: &TempDir) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        temp.path().join("Library").join("Application Support")
    }
    #[cfg(target_os = "windows")]
    {
        temp.path().join("AppData").join("Roaming")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        temp.path().to_path_buf()
    }
}

fn write_test_api_key(temp: &TempDir, env_file: &str, env_key: &str, value: &str) {
    let config_dir = test_config_dir(temp).join("jcode");
    std::fs::create_dir_all(&config_dir).expect("create test config dir");
    std::fs::write(config_dir.join(env_file), format!("{env_key}={value}\n"))
        .expect("write test api key");
}

fn isolate_openrouter_autodetect_env() -> Vec<EnvVarGuard> {
    let mut guards = vec![
        EnvVarGuard::remove("JCODE_OPENROUTER_API_BASE"),
        EnvVarGuard::remove("JCODE_OPENROUTER_API_KEY_NAME"),
        EnvVarGuard::remove("JCODE_OPENROUTER_ENV_FILE"),
        EnvVarGuard::remove("JCODE_OPENROUTER_DYNAMIC_BEARER_PROVIDER"),
        EnvVarGuard::remove("JCODE_OPENROUTER_MODEL"),
        EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE"),
        EnvVarGuard::remove("JCODE_OPENROUTER_ALLOW_NO_AUTH"),
        EnvVarGuard::remove("JCODE_OPENROUTER_TRANSPORT_STATE"),
        EnvVarGuard::remove("JCODE_OPENROUTER_PROVIDER_FEATURES"),
        EnvVarGuard::remove("JCODE_OPENROUTER_MODEL_CATALOG"),
        EnvVarGuard::remove("JCODE_OPENROUTER_AUTH_HEADER"),
        EnvVarGuard::remove("JCODE_OPENROUTER_AUTH_HEADER_NAME"),
        EnvVarGuard::remove("JCODE_OPENROUTER_STATIC_MODELS"),
        EnvVarGuard::remove("JCODE_ACTIVE_PROVIDER"),
        EnvVarGuard::remove("JCODE_RUNTIME_PROVIDER"),
        EnvVarGuard::remove("JCODE_NAMED_PROVIDER_PROFILE"),
        EnvVarGuard::remove("JCODE_PROVIDER_PROFILE_NAME"),
        EnvVarGuard::remove("JCODE_PROVIDER_PROFILE_ACTIVE"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_API_BASE"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_API_KEY_NAME"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_ENV_FILE"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_SETUP_URL"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_DEFAULT_MODEL"),
        EnvVarGuard::remove("JCODE_OPENAI_COMPAT_LOCAL_ENABLED"),
    ];
    guards.extend(
        jcode_base::provider_catalog::openai_compatible_profiles()
            .iter()
            .map(|profile| EnvVarGuard::remove(profile.api_key_env)),
    );
    guards
}

#[test]
fn test_has_credentials() {
    let _has_creds = OpenRouterProvider::has_credentials();
}

/// Extract the JSON request body from a captured raw HTTP request.
fn parse_captured_request_body(request: &str) -> serde_json::Value {
    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(request);
    serde_json::from_str(body)
        .unwrap_or_else(|err| panic!("captured request body should be JSON ({err}): {body}"))
}

fn make_endpoint(name: &str, throughput: f64, uptime: f64, cache: bool, cost: f64) -> EndpointInfo {
    EndpointInfo {
        provider_name: name.to_string(),
        tag: None,
        pricing: ModelPricing {
            prompt: Some(format!("{:.10}", cost)),
            completion: None,
            input_cache_read: if cache {
                Some("0.00000007".to_string())
            } else {
                None
            },
            input_cache_write: None,
        },
        context_length: None,
        max_completion_tokens: None,
        quantization: None,
        uptime_last_30m: Some(uptime),
        latency_last_30m: None,
        throughput_last_30m: Some(serde_json::json!({"p50": throughput})),
        supports_implicit_caching: Some(cache),
        status: Some(0),
    }
}

fn make_provider() -> OpenRouterProvider {
    OpenRouterProvider {
        client: jcode_provider_core::shared_http_client(),
        model: Arc::new(RwLock::new(DEFAULT_MODEL.to_string())),
        reasoning_effort: Arc::new(RwLock::new(None)),
        api_base: DEFAULT_API_BASE.to_string(),
        auth: ProviderAuth::AuthorizationBearer {
            token: "test".to_string(),
            label: DEFAULT_API_KEY_NAME.to_string(),
        },
        supports_provider_features: true,
        supports_model_catalog: true,
        profile_id: None,
        reasoning_effort_support: None,
        disable_reasoning_heuristics: false,
        static_reasoning_config: HashMap::new(),
        max_tokens: None,
        extra_body: None,
        static_models: Vec::new(),
        static_context_limits: HashMap::new(),
        static_image_input_support: HashMap::new(),
        send_openrouter_headers: true,
        conversation_id: new_conversation_id(),
        models_cache: Arc::new(RwLock::new(ModelsCache::default())),
        model_catalog_refresh: Arc::new(Mutex::new(ModelCatalogRefreshState::default())),
        endpoint_refresh: Arc::new(Mutex::new(EndpointRefreshTracker::default())),
        provider_routing: Arc::new(RwLock::new(ProviderRouting::default())),
        provider_pin: Arc::new(Mutex::new(None)),
        endpoints_cache: Arc::new(RwLock::new(HashMap::new())),
    }
}

fn make_custom_compatible_provider() -> OpenRouterProvider {
    OpenRouterProvider {
        client: jcode_provider_core::shared_http_client(),
        model: Arc::new(RwLock::new(DEFAULT_MODEL.to_string())),
        reasoning_effort: Arc::new(RwLock::new(None)),
        api_base: "https://compat.example.test/v1".to_string(),
        auth: ProviderAuth::AuthorizationBearer {
            token: "test".to_string(),
            label: "OPENAI_COMPAT_API_KEY".to_string(),
        },
        supports_provider_features: false,
        supports_model_catalog: true,
        profile_id: None,
        reasoning_effort_support: None,
        disable_reasoning_heuristics: false,
        static_reasoning_config: HashMap::new(),
        max_tokens: None,
        extra_body: None,
        static_models: Vec::new(),
        static_context_limits: HashMap::new(),
        static_image_input_support: HashMap::new(),
        send_openrouter_headers: false,
        conversation_id: new_conversation_id(),
        models_cache: Arc::new(RwLock::new(ModelsCache::default())),
        model_catalog_refresh: Arc::new(Mutex::new(ModelCatalogRefreshState::default())),
        endpoint_refresh: Arc::new(Mutex::new(EndpointRefreshTracker::default())),
        provider_routing: Arc::new(RwLock::new(ProviderRouting::default())),
        provider_pin: Arc::new(Mutex::new(None)),
        endpoints_cache: Arc::new(RwLock::new(HashMap::new())),
    }
}

fn spawn_single_response_models_server(body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake provider server");
    let addr = listener.local_addr().expect("fake provider addr");
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fake provider request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set read timeout");
        let mut request = vec![0u8; 8192];
        let n = stream.read(&mut request).unwrap_or(0);
        let request = String::from_utf8_lossy(&request[..n]).into_owned();
        let _ = request_tx.send(request);

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write fake provider response");
    });

    (format!("http://{addr}/v1"), request_rx)
}

fn spawn_single_response_chat_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake provider server");
    let addr = listener.local_addr().expect("fake provider addr");
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fake provider request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set read timeout");
        let mut request = vec![0u8; 16384];
        let n = stream.read(&mut request).unwrap_or(0);
        let request = String::from_utf8_lossy(&request[..n]).into_owned();
        let _ = request_tx.send(request);

        let body = "data: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write fake provider response");
    });

    (format!("http://{addr}/v1"), request_rx)
}

fn live_openrouter_models() -> Vec<String> {
    std::env::var("JCODE_LIVE_OPENROUTER_MODELS")
        .or_else(|_| std::env::var("JCODE_OPENROUTER_MODEL"))
        .unwrap_or_else(|_| "anthropic/claude-sonnet-4.6".to_string())
        .split([',', '\n'])
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
        .collect()
}

async fn collect_openrouter_live_smoke_stream(
    mut stream: EventStream,
    timeout: Duration,
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
    .context("live OpenRouter smoke timed out")?
}

#[test]
fn strict_openai_schema_endpoint_detects_mistral_profile() {
    // Mistral direct profile rejects non-standard reasoning_content/thinking
    // fields with a 422 (issue #261), so it must be flagged strict.
    assert!(OpenRouterProvider::strict_openai_schema_endpoint(
        Some("mistral"),
        "https://api.mistral.ai/v1"
    ));
    assert!(OpenRouterProvider::strict_openai_schema_endpoint(
        Some("MISTRAL"),
        "https://example.com/v1"
    ));
}

#[test]
fn strict_openai_schema_endpoint_detects_mistral_api_base() {
    assert!(OpenRouterProvider::strict_openai_schema_endpoint(
        None,
        "https://api.mistral.ai/v1"
    ));
    assert!(OpenRouterProvider::strict_openai_schema_endpoint(
        Some("custom"),
        "https://API.MISTRAL.AI/v1"
    ));
}

#[test]
fn strict_openai_schema_endpoint_allows_other_providers() {
    assert!(!OpenRouterProvider::strict_openai_schema_endpoint(
        Some("deepseek"),
        "https://api.deepseek.com"
    ));
    assert!(!OpenRouterProvider::strict_openai_schema_endpoint(
        None,
        "https://openrouter.ai/api/v1"
    ));
    assert!(!OpenRouterProvider::strict_openai_schema_endpoint(
        Some("openai"),
        "https://api.openai.com/v1"
    ));
}

#[test]
fn resolve_extra_body_returns_none_when_unset() {
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::remove("JCODE_OPENAI_EXTRA_BODY");
    assert!(OpenRouterProvider::resolve_extra_body(None, "nonexistent.env").is_none());
}

#[test]
fn resolve_extra_body_parses_env_json_object() {
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::set(
        "JCODE_OPENAI_EXTRA_BODY",
        r#"{"chat_template_kwargs":{"thinking":true,"reasoning_effort":"high"}}"#,
    );
    let extra =
        OpenRouterProvider::resolve_extra_body(None, "nonexistent.env").expect("extra body");
    let kwargs = extra
        .get("chat_template_kwargs")
        .and_then(|v| v.as_object())
        .expect("chat_template_kwargs object");
    assert_eq!(kwargs.get("thinking"), Some(&serde_json::json!(true)));
    assert_eq!(
        kwargs.get("reasoning_effort"),
        Some(&serde_json::json!("high"))
    );
}

#[test]
fn resolve_extra_body_ignores_invalid_env_json() {
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::set("JCODE_OPENAI_EXTRA_BODY", "not-json");
    assert!(OpenRouterProvider::resolve_extra_body(None, "nonexistent.env").is_none());
}

#[test]
fn resolve_extra_body_ignores_non_object_env_json() {
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::set("JCODE_OPENAI_EXTRA_BODY", "[1,2,3]");
    assert!(OpenRouterProvider::resolve_extra_body(None, "nonexistent.env").is_none());
}

#[test]
fn resolve_extra_body_merges_config_and_env_with_env_override() {
    let _lock = ENV_LOCK.lock();
    let config = serde_json::json!({
        "chat_template_kwargs": {"thinking": false},
        "config_only": 1,
    });
    let _guard = EnvVarGuard::set(
        "JCODE_OPENAI_EXTRA_BODY",
        r#"{"chat_template_kwargs":{"thinking":true},"env_only":2}"#,
    );
    let extra = OpenRouterProvider::resolve_extra_body(Some(&config), "nonexistent.env")
        .expect("merged extra body");
    // Env overrides the colliding key.
    assert_eq!(
        extra
            .get("chat_template_kwargs")
            .and_then(|v| v.get("thinking")),
        Some(&serde_json::json!(true))
    );
    // Non-colliding keys from both sources survive.
    assert_eq!(extra.get("config_only"), Some(&serde_json::json!(1)));
    assert_eq!(extra.get("env_only"), Some(&serde_json::json!(2)));
}

#[test]
fn resolve_extra_body_ignores_non_object_config() {
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::remove("JCODE_OPENAI_EXTRA_BODY");
    let config = serde_json::json!("not an object");
    assert!(OpenRouterProvider::resolve_extra_body(Some(&config), "nonexistent.env").is_none());
}

#[test]
fn named_profile_extra_body_threads_into_provider() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let _extra_guard = EnvVarGuard::remove("JCODE_OPENAI_EXTRA_BODY");

    let mut profile = jcode_base::config::NamedProviderConfig {
        base_url: "https://integrate.api.nvidia.com/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        requires_api_key: Some(false),
        ..Default::default()
    };
    profile.extra_body = Some(serde_json::json!({
        "chat_template_kwargs": {"thinking": true, "reasoning_effort": "high"}
    }));

    let provider = OpenRouterProvider::new_named_openai_compatible("my-nim", &profile)
        .expect("build named provider");
    let extra = provider.extra_body.as_ref().expect("extra body present");
    assert_eq!(
        extra
            .get("chat_template_kwargs")
            .and_then(|v| v.get("reasoning_effort")),
        Some(&serde_json::json!("high"))
    );
}

#[test]
fn named_provider_config_deserializes_nested_extra_body_toml() {
    // Verifies the exact `config.toml` shape documented in the README:
    // a nested `[providers.<name>.extra_body.chat_template_kwargs]` table
    // round-trips into the `serde_json::Value` field correctly.
    let toml_str = r#"
type = "openai-compatible"
base_url = "https://integrate.api.nvidia.com/v1"
api_key_env = "NVIDIA_API_KEY"
default_model = "deepseek-ai/deepseek-v4-flash"

[extra_body.chat_template_kwargs]
thinking = true
reasoning_effort = "high"
"#;
    let profile: jcode_base::config::NamedProviderConfig =
        toml::from_str(toml_str).expect("parse named provider toml");
    let extra = profile.extra_body.as_ref().expect("extra_body present");
    let kwargs = extra
        .get("chat_template_kwargs")
        .and_then(|v| v.as_object())
        .expect("chat_template_kwargs object");
    assert_eq!(kwargs.get("thinking"), Some(&serde_json::json!(true)));
    assert_eq!(
        kwargs.get("reasoning_effort"),
        Some(&serde_json::json!("high"))
    );

    // And the resolver hands it back unchanged when no env override is set.
    let _lock = ENV_LOCK.lock();
    let _guard = EnvVarGuard::remove("JCODE_OPENAI_EXTRA_BODY");
    let resolved =
        OpenRouterProvider::resolve_extra_body(profile.extra_body.as_ref(), "nonexistent.env")
            .expect("resolved extra body");
    assert_eq!(
        resolved
            .get("chat_template_kwargs")
            .and_then(|v| v.get("reasoning_effort")),
        Some(&serde_json::json!("high"))
    );
}

// ============================================================================
// Mid-stream retry rollback (issue #338 gap #3)
// ============================================================================

/// Fake SSE server: the first connection streams partial output then drops the
/// socket mid-stream (transport fault); the second connection streams a clean,
/// complete response.
fn spawn_midstream_fault_then_complete_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake provider server");
    let addr = listener.local_addr().expect("fake provider addr");

    std::thread::spawn(move || {
        // Connection 1: partial output, then abrupt close (no [DONE]).
        {
            let (mut stream, _) = listener.accept().expect("accept first request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let mut request = vec![0u8; 65536];
            let _ = stream.read(&mut request);
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial answer that must not duplicate\"}}]}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write partial response");
            stream.flush().expect("flush partial response");
            // Drop without terminating the chunked encoding: the client sees
            // an unexpected EOF mid-stream (transient transport fault).
            drop(stream);
        }

        // Connection 2 (the retry): clean complete response.
        {
            let (mut stream, _) = listener.accept().expect("accept retry request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let mut request = vec![0u8; 65536];
            let _ = stream.read(&mut request);
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"final answer\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write retry response");
        }
    });

    format!("http://{addr}/v1")
}

/// Regression for issue #338 gap #3: a transient transport fault that hits
/// mid-stream, after partial output has already been emitted, must surface a
/// `RetryRollback` before the replayed response so consumers can discard the
/// partial attempt instead of rendering duplicated output.
#[test]
fn midstream_transport_fault_emits_retry_rollback_before_replay() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    rt.block_on(async {
        let api_base = spawn_midstream_fault_then_complete_server();
        let client = reqwest::Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<StreamEvent>>(64);

        let request = serde_json::json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        });

        super::openrouter_sse_stream::run_stream_with_retries(
            client,
            api_base,
            ProviderAuth::None {
                label: "test".to_string(),
            },
            false,
            new_conversation_id(),
            request,
            tx,
            Arc::new(Mutex::new(None)),
            "test-model".to_string(),
        )
        .await;

        let mut events = Vec::new();
        while let Some(item) = rx.recv().await {
            events.push(item);
        }

        let mut saw_partial = false;
        let mut rollback_after_partial = false;
        let mut final_after_rollback = false;
        let mut duplicate_partial_without_rollback = false;
        for item in &events {
            let Ok(event) = item else {
                panic!("stream surfaced an error instead of retrying: {item:?}");
            };
            match event {
                StreamEvent::TextDelta(text) => {
                    if text.contains("partial answer") {
                        if saw_partial && !rollback_after_partial {
                            duplicate_partial_without_rollback = true;
                        }
                        saw_partial = true;
                    }
                    if text.contains("final answer") {
                        assert!(
                            rollback_after_partial,
                            "replayed response arrived without a RetryRollback after partial output"
                        );
                        final_after_rollback = true;
                    }
                }
                StreamEvent::RetryRollback { .. } => {
                    assert!(
                        saw_partial,
                        "RetryRollback must only be emitted after partial output was streamed"
                    );
                    rollback_after_partial = true;
                }
                _ => {}
            }
        }

        assert!(saw_partial, "first attempt's partial output never arrived");
        assert!(
            rollback_after_partial,
            "no RetryRollback emitted for the mid-stream fault"
        );
        assert!(
            final_after_rollback,
            "retry never delivered the complete response"
        );
        assert!(
            !duplicate_partial_without_rollback,
            "partial output duplicated without an interleaved rollback"
        );
    });
}

/// Regression: when the shared interactive server boots an `OpenRouterProvider`
/// without binding `profile_id` (the deferred-auth bootstrap path used by the
/// TUI server), a session-routing `<name>:` prefix for a *user-defined* named
/// provider profile (`[providers.<name>]` in config.toml) must still be
/// stripped before the model id reaches the upstream API. Without this, a
/// resumed/new TUI session sends e.g. `cline:cline-pass/qwen3.7-max` verbatim
/// and the gateway rejects it with 404 model_not_found, even though headless
/// `jcode run` (which binds profile_id in-process) works fine.
#[test]
fn user_named_profile_prefix_is_stripped_even_without_profile_id() {
    let _lock = ENV_LOCK.lock();
    let temp = TempDir::new().expect("create temp home");
    let jcode_home = temp.path().join("jcode-home");
    let _jcode_home = EnvVarGuard::set("JCODE_HOME", &jcode_home);
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _appdata = EnvVarGuard::set("APPDATA", temp.path().join("AppData").join("Roaming"));
    let _env = isolate_openrouter_autodetect_env();
    let (api_base, request_rx) = spawn_single_response_chat_server();

    std::fs::create_dir_all(&jcode_home).expect("create test config dir");
    std::fs::write(
        jcode_home.join("config.toml"),
        r#"
[provider]
default_provider = "cline"

[providers.cline]
type = "openai-compatible"
base_url = "https://api.cline.bot/api/v1"
api_key_env = "TEST_CLINE_KEY"
default_model = "cline-pass/qwen3.7-max"
model_catalog = false
"#,
    )
    .expect("write test config");
    jcode_base::config::invalidate_config_cache();

    // Simulate the shared-server provider slot: a generic OpenAI-compatible
    // provider with NO profile_id bound (deferred-auth bootstrap path).
    let provider = OpenRouterProvider {
        api_base,
        profile_id: None,
        supports_provider_features: false,
        supports_model_catalog: false,
        ..make_custom_compatible_provider()
    };

    // Session restore / default-model routing hands the provider a
    // `<name>:<model>` spec for the user profile.
    provider
        .set_model("cline:cline-pass/qwen3.7-max")
        .expect("set prefixed model");

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
            if event.is_err() {
                break;
            }
        }
    });

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("capture fake provider request");
    let body = parse_captured_request_body(&request);
    assert_eq!(
        body.get("model").and_then(|v| v.as_str()),
        Some("cline-pass/qwen3.7-max"),
        "user-defined named profile prefix must be stripped from the outbound model id; got: {request}"
    );

    jcode_base::config::invalidate_config_cache();
}
include!("openrouter_stream_options_tests.rs");

/// A named OpenAI-compatible profile keeps the stable machine-facing
/// `Provider::name()` and surfaces its identity through `display_name()`.
///
/// Issue #691 proposed returning `profile_id` from `name()`. That would regress
/// the contract documented on the trait and settled in #329: billing, routing,
/// and provider-class matching key off `name()`, so it must stay constant for a
/// provider class, while user-visible labels come from `display_name()`. This
/// pins both halves so the split cannot be undone by accident.
#[test]
fn named_openai_compatible_provider_keeps_stable_name_and_profile_display_name() {
    let _lock = ENV_LOCK.lock();
    let _namespace = EnvVarGuard::remove("JCODE_OPENROUTER_CACHE_NAMESPACE");

    let profile = jcode_base::config::NamedProviderConfig {
        base_url: "https://llm.example.com/v1".to_string(),
        auth: jcode_base::config::NamedProviderAuth::None,
        default_model: Some("example-model".to_string()),
        ..Default::default()
    };

    let provider = OpenRouterProvider::new_named_openai_compatible("example-compat", &profile)
        .expect("named profile should initialize");

    // Machine-facing identity: stable per provider class.
    assert_eq!(
        Provider::name(&provider),
        "openrouter",
        "billing/routing key off name(); it must not become the profile id"
    );
    // User-facing identity: the profile the user configured.
    assert_eq!(provider.runtime_display_name(), "example-compat");
    assert_eq!(Provider::display_name(&provider), "example-compat");
}

/// Issue #1167: OpenCode Go/Zen require a stable per-conversation
/// `x-opencode-session` header; other OpenAI-compatible hosts must not get it.
#[test]
fn opencode_session_header_only_for_opencode_hosts() {
    assert!(is_opencode_api_base("https://opencode.ai/zen/go/v1"));
    assert!(is_opencode_api_base("https://opencode.ai/zen/v1"));
    assert!(is_opencode_api_base("https://api.opencode.ai/v1"));
    assert!(!is_opencode_api_base("https://openrouter.ai/api/v1"));
    assert!(!is_opencode_api_base("https://api.deepseek.com/v1"));
    assert!(!is_opencode_api_base("not a url"));

    let client = reqwest::Client::new();
    let req = apply_opencode_session_header(
        client.post("https://opencode.ai/zen/go/v1/chat/completions"),
        "https://opencode.ai/zen/go/v1",
        "conv-123",
    )
    .build()
    .unwrap();
    assert_eq!(
        req.headers()
            .get(OPENCODE_SESSION_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some("conv-123")
    );

    let req = apply_opencode_session_header(
        client.post("https://openrouter.ai/api/v1/chat/completions"),
        "https://openrouter.ai/api/v1",
        "conv-123",
    )
    .build()
    .unwrap();
    assert!(req.headers().get(OPENCODE_SESSION_HEADER).is_none());
}

#[test]
fn opencode_session_ids_are_uuids_and_unique() {
    let a = new_conversation_id();
    let b = new_conversation_id();
    assert_ne!(a, b);
    assert!(uuid::Uuid::parse_str(&a).is_ok());
}

/// Wire-level check for issue #1167: a real `chat/completions` request whose
/// api_base host is `opencode.ai` carries `x-opencode-session`, and a
/// request to another host does not. The DNS override points the hostname at
/// a local listener, so the full stream path (including retries) is exercised.
fn spawn_header_capturing_server() -> (std::net::SocketAddr, std::sync::mpsc::Receiver<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut buf = vec![0u8; 65536];
        let n = stream.read(&mut buf).unwrap_or(0);
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (addr, rx)
}

fn captured_request_for_host(host: &str, conversation_id: &str) -> String {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let (addr, rx) = spawn_header_capturing_server();
        let client = reqwest::Client::builder()
            .resolve(host, addr)
            .build()
            .expect("client");
        let api_base = format!("http://{host}:{}/zen/go/v1", addr.port());
        let (tx, mut events) = tokio::sync::mpsc::channel::<anyhow::Result<StreamEvent>>(64);
        super::openrouter_sse_stream::run_stream_with_retries(
            client,
            api_base,
            ProviderAuth::None {
                label: "test".to_string(),
            },
            false,
            conversation_id.to_string(),
            serde_json::json!({"model": "m", "messages": [], "stream": true}),
            tx,
            Arc::new(Mutex::new(None)),
            "m".to_string(),
        )
        .await;
        while events.recv().await.is_some() {}
        rx.recv_timeout(Duration::from_secs(5))
            .expect("server captured request")
    })
}

#[test]
fn opencode_session_header_is_sent_on_the_wire_only_to_opencode_hosts() {
    let raw = captured_request_for_host("opencode.ai", "conv-wire-1167").to_ascii_lowercase();
    assert!(
        raw.contains("x-opencode-session: conv-wire-1167"),
        "opencode.ai request lacked the header:\n{raw}"
    );

    let raw = captured_request_for_host("example.test", "conv-wire-1167").to_ascii_lowercase();
    assert!(
        !raw.contains("x-opencode-session"),
        "non-opencode host received the header:\n{raw}"
    );
}

include!("openrouter_tests/catalog_profiles.rs");
include!("openrouter_tests/reasoning.rs");
include!("openrouter_tests/routing.rs");
