use super::*;

fn response(body: Value) -> Response {
    Response::success(9, 3, "setBreakpoints", Some(body))
}

#[test]
fn breakpoint_request_omits_absent_fields_and_deprecated_lines() {
    let value = source_breakpoint_json(&DebugSourceBreakpoint::new(7));
    assert_eq!(value, json!({"line": 7}));
    assert!(value.get("lines").is_none());
    assert!(value.get("column").is_none());
    assert!(value.get("condition").is_none());
    assert!(value.get("hitCondition").is_none());
    assert!(value.get("logMessage").is_none());
}

#[test]
fn breakpoint_request_serializes_all_optional_fields_in_camel_case() {
    let value = source_breakpoint_json(
        &DebugSourceBreakpoint::new(7)
            .with_column(3)
            .with_condition("ready")
            .with_hit_condition("5")
            .with_log_message("value={value}"),
    );
    assert_eq!(
        value,
        json!({
            "line": 7,
            "column": 3,
            "condition": "ready",
            "hitCondition": "5",
            "logMessage": "value={value}"
        })
    );
}

#[test]
fn set_breakpoints_response_decodes_required_and_optional_fields_exactly() {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-wire-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("wire.rs");
    std::fs::write(&source, b"x").unwrap();
    let canonical = source.canonicalize().unwrap();
    let workspace = DebugWorkspaceKey::new(&root, "owner").unwrap();
    let parsed = parse_breakpoints_response(
        &response(json!({"breakpoints":[{
            "id": i32::MAX,
            "verified": false,
            "reason": "pending",
            "message": "not bound",
            "line": 9,
            "column": 2,
            "endLine": 10,
            "endColumn": 4,
            "source": {"path": canonical}
        }]})),
        1,
        &DebugOperationConfig::default(),
        &workspace,
        &source.canonicalize().unwrap(),
    )
    .unwrap();
    assert_eq!(parsed.len(), 1);
    let mut breakpoint = pending_breakpoint(
        DebugBreakpointId(1),
        &ResolvedSource {
            original: source.clone(),
            canonical: source.canonicalize().unwrap(),
            wire_path: source.canonicalize().unwrap().to_str().unwrap().to_owned(),
            relative: PathBuf::from("wire.rs"),
            revision: DebugSourceRevision {
                sha256: [1; 32],
                byte_len: 1,
            },
        },
        DebugSourceBreakpoint::new(9),
    );
    apply_wire_breakpoint(
        &mut breakpoint,
        parsed.into_iter().next().unwrap(),
        &DebugOperationConfig::default(),
    );
    assert_eq!(breakpoint.adapter_id, Some(i64::from(i32::MAX)));
    assert!(!breakpoint.verified);
    assert_eq!(breakpoint.reason, Some(DebugBreakpointReason::Pending));
    assert_eq!(breakpoint.message.as_deref(), Some("not bound"));
    assert_eq!(
        breakpoint.resolved,
        DebugBreakpointLocation {
            line: Some(9),
            column: Some(2),
            end_line: Some(10),
            end_column: Some(4)
        }
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn set_breakpoints_response_requires_body_array_verified_and_exact_cardinality() {
    let f = fixture("owner");
    let workspace = DebugWorkspaceKey::new(&f.root, "owner").unwrap();
    let canonical = f.source.canonicalize().unwrap();
    for (candidate, count) in [
        (Response::success(2, 1, "setBreakpoints", None), 0),
        (response(json!({})), 0),
        (response(json!({"breakpoints":[{}]})), 1),
        (response(json!({"breakpoints":[]})), 1),
    ] {
        assert!(matches!(
            parse_breakpoints_response(
                &candidate,
                count,
                &DebugOperationConfig::default(),
                &workspace,
                &canonical
            ),
            Err(DapError::InvalidSetBreakpointsResponse { .. })
        ));
    }
}

#[tokio::test]
async fn breakpoint_wire_integer_boundaries_accept_limits_and_reject_overflow_or_zero() {
    let f = fixture("owner");
    let workspace = DebugWorkspaceKey::new(&f.root, "owner").unwrap();
    let canonical = f.source.canonicalize().unwrap();
    for body in [
        json!({"breakpoints":[{"id": i32::MIN, "verified": true, "line": MAX_DAP_INTEGER}]}),
        json!({"breakpoints":[{"id": i32::MAX, "verified": true, "column": MAX_DAP_INTEGER}]}),
    ] {
        assert!(
            parse_breakpoints_response(
                &response(body),
                1,
                &DebugOperationConfig::default(),
                &workspace,
                &canonical
            )
            .is_ok()
        );
    }
    for body in [
        json!({"breakpoints":[{"id": i64::from(i32::MAX) + 1, "verified": true}]}),
        json!({"breakpoints":[{"verified": true, "line": 0}]}),
        json!({"breakpoints":[{"verified": true, "line": MAX_DAP_INTEGER + 1}]}),
    ] {
        assert!(matches!(
            parse_breakpoints_response(
                &response(body),
                1,
                &DebugOperationConfig::default(),
                &workspace,
                &canonical
            ),
            Err(DapError::InvalidSetBreakpointsResponse { .. })
        ));
    }
}
