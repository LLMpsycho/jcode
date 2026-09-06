use super::*;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn rejection_fixture(
    status: &str,
    error: &str,
    requests: usize,
) -> (String, std::thread::JoinHandle<Vec<serde_json::Value>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let status = status.to_string();
    let error = error.to_string();
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut bodies = Vec::new();
        while bodies.len() < requests && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("fixture accept: {error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let body_start = loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(index) = bytes.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let length: usize = String::from_utf8_lossy(&bytes[..body_start])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|length| length.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < body_start + length {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            bodies.push(serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap());
            let (status, content_type, body) = if bodies.len() == 1 {
                (status.as_str(), "application/json", error.as_str())
            } else {
                (
                    "200 OK",
                    "text/event-stream",
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                )
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        bodies
    });
    (url, task)
}

fn fixture_provider(url: String) -> AnthropicProvider {
    let mut provider = AnthropicProvider::new();
    provider.client = Client::builder().no_proxy().build().unwrap();
    provider.direct_transport = DirectTransportConfig {
        api_url: url,
        headers: Ok(HeaderMap::new()),
        auth_mode: "none".into(),
        auth_header: "x-api-key".into(),
        oauth_fixture_url: None,
    };
    provider.profile_api_key = Some(Ok("fixture-key".into()));
    provider.credential_mode = Arc::new(RwLock::new(AnthropicCredentialMode::ApiKey));
    provider
}

#[test]
fn private_fork_credential_switch_isolates_primary_mode_cache_and_runtime_identity() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _identity = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "claude");
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "fixture-key");
    let _key_name = EnvVarGuard::set("JCODE_ANTHROPIC_API_KEY_NAME", "ANTHROPIC_API_KEY");
    // No async runtime is entered: construction cannot launch a usage fetch.
    let primary = AnthropicProvider::new();
    *primary.credentials.try_write().unwrap() = Some(CachedCredentials {
        access_token: "cached-primary-token".into(),
        refresh_token: String::new(),
        expires_at: i64::MAX,
    });
    let private = primary.fork();
    assert_eq!(private.credential_mode(), AnthropicCredentialMode::OAuth);
    private
        .set_credential_mode(AnthropicCredentialMode::ApiKey)
        .unwrap();
    assert_eq!(private.credential_mode(), AnthropicCredentialMode::ApiKey);
    assert_eq!(
        primary.credential_mode_snapshot(),
        AnthropicCredentialMode::OAuth
    );
    assert!(primary.credentials.try_read().unwrap().is_some());
    assert_eq!(std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(), "claude");

    // Primary changes cannot overwrite a private fork's authentication mode.
    primary
        .set_credential_mode(AnthropicCredentialMode::Auto)
        .unwrap();
    assert_eq!(
        primary.credential_mode_snapshot(),
        AnthropicCredentialMode::Auto
    );
    assert_eq!(private.credential_mode(), AnthropicCredentialMode::ApiKey);
    assert!(primary.credentials.try_read().unwrap().is_none());
}

#[test]
fn private_preparation_suppresses_global_identity_changes_but_primary_selection_keeps_them() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _identity = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "claude");
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "fixture-key");
    let _key_name = EnvVarGuard::set("JCODE_ANTHROPIC_API_KEY_NAME", "ANTHROPIC_API_KEY");
    let primary = AnthropicProvider::new();
    let private = AnthropicProvider::new();
    Provider::prepare_private_session(&private);
    private
        .set_credential_mode(AnthropicCredentialMode::ApiKey)
        .unwrap();
    assert_eq!(
        private.credential_mode_snapshot(),
        AnthropicCredentialMode::ApiKey
    );
    assert_eq!(
        primary.credential_mode_snapshot(),
        AnthropicCredentialMode::OAuth
    );
    assert_eq!(std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(), "claude");

    primary
        .set_credential_mode(AnthropicCredentialMode::ApiKey)
        .unwrap();
    assert_eq!(
        std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
        "claude-api"
    );
    private
        .set_credential_mode(AnthropicCredentialMode::Auto)
        .unwrap();
    assert_eq!(
        std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
        "claude-api"
    );
    assert_eq!(
        primary.credential_mode_snapshot(),
        AnthropicCredentialMode::ApiKey
    );
}

#[test]
fn private_fork_during_credential_mode_write_fails_without_guessing_authentication() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let _identity = EnvVarGuard::set("JCODE_RUNTIME_PROVIDER", "claude");
    let primary = AnthropicProvider::new();
    let held = primary.credential_mode.try_write().unwrap();
    let private = primary.fork();
    assert!(
        private
            .set_model(&primary.model())
            .unwrap_err()
            .to_string()
            .contains("authentication is changing")
    );
    let result = futures::executor::block_on(private.complete(&[], &[], "", None));
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("authentication is changing")
    );
    assert_eq!(*held, AnthropicCredentialMode::OAuth);
    assert_eq!(std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(), "claude");
}

#[test]
fn role_pinned_forks_isolate_policy_and_preserve_model_scoped_quota_choice() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    let provider = AnthropicProvider::new();
    provider.set_model("claude-fable-5").unwrap();
    assert!(!provider.route_pinned());
    provider.set_route_pinned(true);
    let fork = provider.fork();
    assert!(fork.route_pinned());
    fork.set_route_pinned(false);
    assert!(provider.route_pinned());
    let usage = jcode_base::usage::UsageData {
        model_scoped: vec![jcode_base::usage::ModelScopedUsageWindow {
            model_name: "Fable".into(),
            utilization: 1.0,
            resets_at: None,
        }],
        ..Default::default()
    };
    assert_eq!(
        provider.model_after_oauth_usage("claude-fable-5".into(), &usage),
        "claude-fable-5"
    );
    assert_eq!(provider.model(), "claude-fable-5");
    provider.set_route_pinned(false);
    assert!(
        provider
            .model_after_oauth_usage("claude-fable-5".into(), &usage)
            .contains("claude-opus")
    );
}

#[test]
fn role_pinned_streams_reject_model_substitution_and_effort_stripping() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
    for split in [false, true] {
        for (model, status, body, effort_error) in [
            (
                "claude-fable-5",
                "404 Not Found",
                r#"{"error":{"type":"not_found_error","message":"model not found"}}"#,
                false,
            ),
            (
                "claude-opus-5",
                "400 Bad Request",
                r#"{"error":{"type":"invalid_request_error","message":"This model does not support the effort parameter."}}"#,
                true,
            ),
        ] {
            for pinned in [false, true] {
                let (url, server) = rejection_fixture(status, body, if pinned { 1 } else { 2 });
                // Construct outside the runtime so new() cannot start a usage fetch.
                let provider = fixture_provider(url);
                provider.set_model(model).unwrap();
                provider.set_reasoning_effort("high").unwrap();
                provider.set_route_pinned(pinned);
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let failed = runtime.block_on(async {
                    let messages = [Message::user("fixture")];
                    let mut events = if split {
                        provider
                            .complete_split(&messages, &[], "static", "dynamic", None)
                            .await
                            .unwrap()
                    } else {
                        provider
                            .complete(&messages, &[], "system", None)
                            .await
                            .unwrap()
                    };
                    // Exercise pin lifetime beyond EventStream construction.
                    provider.set_route_pinned(false);
                    let mut failed = false;
                    tokio::time::timeout(Duration::from_secs(7), async {
                        while let Some(event) = events.next().await {
                            failed |= event.is_err();
                        }
                    })
                    .await
                    .unwrap();
                    failed
                });
                let requests = server.join().unwrap();
                assert_eq!(requests.len(), if pinned { 1 } else { 2 });
                assert_eq!(failed, pinned);
                assert_eq!(requests[0]["model"], model);
                if pinned {
                    assert_eq!(provider.model(), model);
                } else if effort_error {
                    assert!(requests[0]["output_config"].is_object());
                    assert!(requests[1].get("output_config").is_none());
                } else {
                    assert_ne!(requests[1]["model"], model);
                    assert_ne!(provider.model(), model);
                }
            }
        }
    }
}

#[test]
fn role_pinned_oauth_quota_error_is_terminal_without_model_retry() {
    let _lock = jcode_base::storage::lock_test_env();
    let (url, server) = rejection_fixture(
        "429 Too Many Requests",
        r#"{"error":{"type":"rate_limit_error","message":"Fable weekly usage limit reached"}}"#,
        1,
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let request = ApiRequest {
            model: "claude-fable-5".into(),
            max_tokens: 64,
            system: None,
            messages: vec![],
            tools: None,
            metadata: None,
            thinking: None,
            output_config: None,
            temperature: None,
            service_tier: None,
            stream: true,
        };
        let (tx, mut rx) = mpsc::channel(100);
        let model = Arc::new(std::sync::RwLock::new("claude-fable-5".into()));
        tokio::time::timeout(
            Duration::from_secs(2),
            run_stream_with_retries(
                Client::builder().no_proxy().build().unwrap(),
                "fixture-token".into(),
                true,
                request,
                tx,
                Arc::new(RwLock::new(None)),
                "claude-fable-5".into(),
                "fixture-session".into(),
                model.clone(),
                DirectTransportConfig {
                    api_url: API_URL.to_string(),
                    headers: Ok(HeaderMap::new()),
                    auth_mode: "none".into(),
                    auth_header: "x-api-key".into(),
                    oauth_fixture_url: Some(url),
                },
                true,
            ),
        )
        .await
        .unwrap();
        let mut failure = None;
        while let Some(event) = rx.recv().await {
            if let Err(error) = event {
                failure = Some(error.to_string());
            }
        }
        let failure = failure.expect("pinned quota rejection must be terminal");
        assert!(
            failure.contains("automatic substitution is disabled"),
            "{failure}"
        );
        assert_eq!(*model.read().unwrap(), "claude-fable-5");
    });
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "claude-fable-5");
}
