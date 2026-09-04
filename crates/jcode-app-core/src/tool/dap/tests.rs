use super::*;

fn missing_adapter_command() -> String {
    #[cfg(windows)]
    {
        r"C:\jcode-definitely-missing\adapter.exe".to_owned()
    }
    #[cfg(not(windows))]
    {
        "/jcode-definitely-missing/adapter".to_owned()
    }
}

fn adapter(kind: jcode_dap::DapAdapterKind, command: String) -> jcode_dap::DapAdapterConfig {
    jcode_dap::DapAdapterConfig { kind, command }
}

#[test]
fn omitted_adapter_selects_the_first_available_configured_profile() {
    let adapters = BTreeMap::from([
        (
            "lldb-dap".to_owned(),
            adapter(
                jcode_dap::DapAdapterKind::LldbDap,
                missing_adapter_command(),
            ),
        ),
        (
            "gdb".to_owned(),
            adapter(
                jcode_dap::DapAdapterKind::GdbDap,
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
        ),
    ]);

    let selected = select_adapter(&adapters, None).unwrap();
    assert_eq!(selected.kind(), jcode_dap::DebugAdapterKind::GdbDap);
}

#[test]
fn omitted_adapter_preserves_the_lldb_compatibility_preference() {
    let command = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let adapters = BTreeMap::from([
        (
            "a-gdb".to_owned(),
            adapter(jcode_dap::DapAdapterKind::GdbDap, command.clone()),
        ),
        (
            "lldb-dap".to_owned(),
            adapter(jcode_dap::DapAdapterKind::LldbDap, command),
        ),
    ]);

    let selected = select_adapter(&adapters, None).unwrap();
    assert_eq!(selected.kind(), jcode_dap::DebugAdapterKind::LldbDap);
}

#[test]
fn omitted_adapter_reports_guidance_when_none_are_configured() {
    let error = select_adapter(&BTreeMap::new(), None).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no configured DAP adapter is available")
    );
}

#[test]
fn explicit_unavailable_adapter_never_falls_back() {
    let adapters = BTreeMap::from([
        (
            "lldb-dap".to_owned(),
            adapter(
                jcode_dap::DapAdapterKind::LldbDap,
                missing_adapter_command(),
            ),
        ),
        (
            "gdb".to_owned(),
            adapter(
                jcode_dap::DapAdapterKind::GdbDap,
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
        ),
    ]);

    let error = select_adapter(&adapters, Some("lldb-dap")).unwrap_err();
    assert!(error.to_string().contains("'lldb-dap' is unavailable"));
}

#[test]
fn broker_tokens_are_opaque_owner_bound_and_cleaned() {
    let mut b = TokenBroker::new(8);
    let token = TokenBroker::token("ds");
    assert!(token.starts_with("ds_"));
    assert!(!token.contains("dap-"));
    b.frames.clear();
}

#[test]
fn broker_compacts_stale_order_records() {
    let mut broker = TokenBroker::new(8);
    for index in 0..32 {
        broker.record(TokenKind::Frame, format!("missing-{index}"));
    }
    assert_eq!(broker.order.len(), 32);
    broker.compact_order();
    assert!(broker.order.is_empty());
}

#[test]
fn broker_refuses_a_response_larger_than_owner_capacity() {
    let mut broker = TokenBroker::new(2);
    assert!(broker.reserve_capacity("owner", 3, true).is_err());
    assert!(broker.reserve_capacity("owner", 2, true).is_ok());
}

#[test]
fn schema_exposes_exact_action_set() {
    let service = DapService::from_config(&jcode_dap::DapConfig::default()).unwrap();
    let schema = service.tool().parameters_schema();
    let actions = schema
        .pointer("/properties/action/enum")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(actions.len(), 18);
    assert!(actions.iter().any(|value| value == "step_in_targets"));
    assert!(!actions.iter().any(|v| v == "custom" || v == "request"));
    assert!(schema.pointer("/properties/pid").is_none());
}
#[test]
fn output_is_bounded() {
    let value = json!({"x":"a".repeat(MAX_OUTPUT_CHARS+100)});
    let output = bounded_pretty(&value, MAX_OUTPUT_CHARS);
    assert!(output.len() <= MAX_OUTPUT_CHARS);
    let parsed: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["protocol"], "jcode.dap.v1");
    assert_eq!(parsed["result"]["truncated"], true);
}

#[tokio::test]
async fn explicit_frame_resolution_skips_current_frame_lookup() {
    let frame = resolve_frame_or_current(
        Some("frame-token"),
        Some(DebugThreadId::new(7)),
        |token| {
            assert_eq!(token, "frame-token");
            Ok(41)
        },
        |_| async { panic!("explicit frame tokens must not issue stackTrace") },
    )
    .await
    .unwrap();

    assert_eq!(frame, 41);
}

#[tokio::test]
async fn current_frame_resolution_is_bounded_and_thread_scoped() {
    let frame = resolve_frame_or_current(
        None,
        Some(DebugThreadId::new(7)),
        |_| panic!("omitted frame tokens must not use the token broker"),
        |request| async move {
            assert_eq!(request.thread_id, Some(DebugThreadId::new(7)));
            assert_eq!(request.start_frame, 0);
            assert_eq!(request.levels, 1);
            assert_eq!(request.expected_execution_revision, None);
            Ok(Some(41))
        },
    )
    .await
    .unwrap();

    assert_eq!(frame, 41);
}

#[tokio::test]
async fn missing_current_frame_requires_an_explicit_token() {
    let error = resolve_frame_or_current::<(), _, _, _>(
        None,
        None,
        |_| panic!("omitted frame tokens must not use the token broker"),
        |request| async move {
            assert_eq!(request.thread_id, None);
            assert_eq!(request.levels, 1);
            Ok(None)
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("supply a frame token"));
}

#[tokio::test]
async fn lifecycle_gate_serializes_cleanup_and_reconnect() {
    let service = DapService::from_config(&jcode_dap::DapConfig::default()).unwrap();
    let first = service.lock_lifecycle_transition().await;
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            service.cleanup_owner("owner"),
        )
        .await
        .is_err()
    );
    drop(first);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        service.cleanup_owner("owner"),
    )
    .await
    .unwrap();
}
