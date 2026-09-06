use super::*;

fn cfg() -> ComposioConfig {
    ComposioConfig {
        api_key: "test-key".to_string(),
        base_url: COMPOSIO_DEFAULT_BASE.to_string(),
        connected_account_id: Some("ca_123".to_string()),
        user_id: Some("me".to_string()),
        auth_config_id: Some("ac_123".to_string()),
    }
}

#[test]
fn message_attachments_flatten_nested_parts() {
    let msg: Message = serde_json::from_value(json!({
        "id": "m1",
        "threadId": "t1",
        "payload": {
            "mimeType": "multipart/mixed",
            "filename": "",
            "parts": [
                {
                    "mimeType": "multipart/alternative",
                    "filename": "",
                    "parts": [
                        { "mimeType": "text/plain", "filename": "", "body": { "size": 10 } }
                    ]
                },
                {
                    "mimeType": "application/pdf",
                    "filename": "receipt.pdf",
                    "body": { "size": 2048, "attachmentId": "att-1" }
                },
                {
                    "mimeType": "image/png",
                    "filename": "photo.png",
                    "body": { "size": 3670016, "attachmentId": "att-2" }
                }
            ]
        }
    }))
    .unwrap();

    let attachments = msg.attachments();
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].filename, "receipt.pdf");
    assert_eq!(attachments[0].attachment_id.as_deref(), Some("att-1"));
    assert_eq!(attachments[1].filename, "photo.png");

    let lines = format_attachment_lines(&attachments);
    assert!(lines.contains("receipt.pdf (application/pdf, 2.0 KB)"));
    assert!(lines.contains("photo.png (image/png, 3.5 MB)"));

    let full = format_message_full(&msg);
    assert!(full.contains("Attachments (2):"));
}

#[test]
fn message_without_attachments_formats_clean() {
    let msg: Message = serde_json::from_value(json!({
        "id": "m2",
        "threadId": "t2",
        "payload": { "mimeType": "text/plain", "filename": "", "body": { "size": 5 } }
    }))
    .unwrap();
    assert!(msg.attachments().is_empty());
    assert!(!format_message_full(&msg).contains("Attachments"));
}

#[test]
fn composio_proxy_payload_get_has_no_body() {
    let url = format!("{}/messages?maxResults=10", GMAIL_API_BASE);
    let payload = build_composio_proxy_payload(&cfg(), "GET", &url, None);
    assert_eq!(payload["endpoint"], url);
    assert_eq!(payload["method"], "GET");
    assert!(payload.get("body").is_none());
    assert_eq!(payload["connected_account_id"], "ca_123");
    assert_eq!(payload["user_id"], "me");
}

#[test]
fn composio_proxy_payload_post_includes_body() {
    let url = format!("{}/messages/send", GMAIL_API_BASE);
    let body = json!({ "raw": "abc" });
    let payload = build_composio_proxy_payload(&cfg(), "POST", &url, Some(body.clone()));
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["body"], body);
}

#[test]
fn composio_proxy_payload_omits_optional_account_fields() {
    let bare = ComposioConfig {
        api_key: "k".to_string(),
        base_url: COMPOSIO_DEFAULT_BASE.to_string(),
        connected_account_id: None,
        user_id: None,
        auth_config_id: None,
    };
    let payload = build_composio_proxy_payload(&bare, "GET", "http://x/y", None);
    assert!(payload.get("connected_account_id").is_none());
    assert!(payload.get("user_id").is_none());
}

#[test]
fn direct_backend_label_and_default() {
    let backend = GmailBackend::Direct;
    assert_eq!(backend.label(), "direct");
    let client = GmailClient::with_backend(GmailBackend::Direct);
    assert_eq!(client.backend_label(), "direct");
}

#[test]
fn composio_backend_is_configured_and_can_send() {
    let client = GmailClient::with_backend(GmailBackend::Composio(cfg()));
    assert_eq!(client.backend_label(), "composio");
    assert!(client.is_configured());
    // Composio connections request full Gmail scopes.
    assert!(client.can_send());
    assert!(client.can_delete());
}

#[test]
fn truncate_error_caps_length() {
    let short = truncate_error("  hi  ");
    assert_eq!(short, "hi");
    let long = "x".repeat(1000);
    let capped = truncate_error(&long);
    assert!(capped.len() <= 401 + 3); // 400 chars + ellipsis byte
    assert!(capped.ends_with('…'));
}

#[test]
fn needs_connection_reflects_connected_account_presence() {
    // Composio without a connected account needs an interactive connect.
    let mut without = cfg();
    without.connected_account_id = None;
    let client = GmailClient::with_backend(GmailBackend::Composio(without));
    assert!(client.supports_connect());
    assert!(client.needs_connection());

    // With a connected account it is ready to make calls.
    let client = GmailClient::with_backend(GmailBackend::Composio(cfg()));
    assert!(!client.needs_connection());

    // Direct backend never needs a Composio connection and cannot connect.
    let direct = GmailClient::with_backend(GmailBackend::Direct);
    assert!(!direct.supports_connect());
    assert!(!direct.needs_connection());
}

#[test]
fn effective_user_id_defaults_to_default() {
    let mut c = cfg();
    c.user_id = None;
    assert_eq!(c.effective_user_id(), "default");
    c.user_id = Some("alice".to_string());
    assert_eq!(c.effective_user_id(), "alice");
}
