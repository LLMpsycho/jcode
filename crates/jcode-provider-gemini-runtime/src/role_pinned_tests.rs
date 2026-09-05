use super::*;
use std::io::{Read, Write};
use std::time::Instant;
use tokio_stream::StreamExt;

fn rejection_fixture(requests: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
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
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
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
            let length: usize = String::from_utf8_lossy(&bytes[..body_start]).lines()
                .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|length| length.trim().parse().unwrap())).unwrap();
            while bytes.len() < body_start + length {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            bodies.push(String::from_utf8_lossy(&bytes[..body_start]).lines().next().unwrap().to_string());
            let (status, content_type, body) = if bodies.len() == 1 {
                ("404 Not Found", "application/json", r#"{"error":{"status":"NOT_FOUND","message":"Requested entity was not found."}}"#)
            } else {
                ("200 OK", "application/json", r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#)
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        bodies
    });
    (url, task)
}

#[test]
fn role_pinned_gemini_forks_keep_independent_policy() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let provider = GeminiProvider::new();
    provider.set_model("gemini-2.5-flash").unwrap();
    assert!(!provider.route_pinned());
    provider.set_route_pinned(true);
    let fork = provider.fork();
    assert!(fork.route_pinned());
    fork.set_route_pinned(false);
    fork.set_model("gemini-2.5-pro").unwrap();
    assert!(provider.route_pinned());
    assert_eq!(provider.model(), "gemini-2.5-flash");
}

#[test]
fn role_pinned_gemini_rejects_model_fallback_after_stream_creation() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let _key = EnvVarGuard::set_value("GEMINI_API_KEY", "fixture-key");
    let _oauth = EnvVarGuard::set_value("JCODE_GEMINI_FORCE_OAUTH", "0");
    for pinned in [false, true] {
        let (url, server) = rejection_fixture(if pinned { 1 } else { 2 });
        let _endpoint = EnvVarGuard::set_value("GEMINI_API_ENDPOINT", &url);
        let _version = EnvVarGuard::set_value("GEMINI_API_VERSION", "v1beta");
        let mut provider = GeminiProvider::new();
        provider.client = reqwest::Client::builder().no_proxy().build().unwrap();
        provider.set_model("gemini-2.5-flash").unwrap();
        provider.set_route_pinned(pinned);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let failed = runtime.block_on(async {
            let mut events = provider.complete(&[Message::user("fixture")], &[], "system", None).await.unwrap();
            // MultiProvider may release its pin once it has the stream. The
            // deferred generator must retain the original per-request policy.
            provider.set_route_pinned(false);
            let mut failed = false;
            tokio::time::timeout(Duration::from_secs(4), async {
                while let Some(event) = events.next().await {
                    failed |= event.is_err();
                }
            }).await.unwrap();
            failed
        });
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), if pinned { 1 } else { 2 });
        assert_eq!(failed, pinned);
        assert!(requests[0].contains("/models/gemini-2.5-flash:generateContent"));
        if pinned {
            assert_eq!(provider.model(), "gemini-2.5-flash");
        } else {
            assert!(!requests[1].contains("/models/gemini-2.5-flash:generateContent"));
            assert_ne!(provider.model(), "gemini-2.5-flash");
        }
    }
}
