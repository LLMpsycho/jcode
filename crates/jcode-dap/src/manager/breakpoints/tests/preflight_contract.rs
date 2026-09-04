use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use super::*;

async fn assert_no_traffic(adapter: &mut FakeAdapter) {
    assert!(
        timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn missing_outside_escaping_symlink_directory_non_utf8_and_oversized_paths_fail_before_traffic()
 {
    let mut f = fixture("owner");
    let outside_root = std::env::temp_dir().join(format!(
        "jcode-dap-outside-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&outside_root).unwrap();
    let outside = outside_root.join("outside.rs");
    std::fs::write(&outside, b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, f.root.join("escape.rs")).unwrap();

    let mut paths = vec![f.root.join("missing.rs"), outside.clone(), f.root.clone()];
    #[cfg(unix)]
    {
        paths.push(f.root.join("escape.rs"));
        paths.push(PathBuf::from(OsString::from_vec(vec![
            0xff, b'.', b'r', b's',
        ])));
        paths.push(PathBuf::from(OsString::from_vec(vec![b'a', 0, b'b'])));
    }
    paths.push(PathBuf::from(
        "x".repeat(DebugOperationConfig::default().max_source_path_bytes + 1),
    ));

    for path in paths {
        assert!(
            f.manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(path, DebugSourceBreakpoint::new(1)),
                )
                .await
                .is_err()
        );
    }
    assert_no_traffic(&mut f.adapter).await;
    std::fs::remove_dir_all(outside_root).unwrap();
}

#[tokio::test]
async fn inside_workspace_symlink_resolves_to_canonical_target_path() {
    let mut f = fixture("owner");
    #[cfg(unix)]
    let link = {
        let link = f.root.join("inside-link.rs");
        std::os::unix::fs::symlink(&f.source, &link).unwrap();
        link
    };
    #[cfg(not(unix))]
    let link = f.source.clone();
    let task = {
        let manager = f.manager.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(link, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    assert_eq!(
        set.arguments.as_ref().unwrap()["source"]["path"],
        json!(f.source.canonicalize().unwrap())
    );
    f.adapter
        .respond_ok(&set, Some(json!({"breakpoints":[{"verified":true}]})))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn relative_source_resolves_under_canonical_workspace_and_emits_canonical_wire_path() {
    let mut f = fixture("owner");
    let relative = f.source.strip_prefix(&f.root).unwrap().to_path_buf();
    let task = {
        let manager = f.manager.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(relative, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    assert_eq!(
        set.arguments.as_ref().unwrap()["source"]["path"],
        json!(f.source.canonicalize().unwrap())
    );
    f.adapter
        .respond_ok(&set, Some(json!({"breakpoints":[{"verified":true}]})))
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap().source.source,
        PathBuf::from("hello world.rs")
    );
}

#[tokio::test]
async fn invalid_source_positions_and_optional_expressions_fail_before_traffic() {
    let mut f = fixture("owner");
    let limit = f.manager.core.operations.max_breakpoint_expression_bytes;
    let invalid = [
        DebugSourceBreakpoint::new(0),
        DebugSourceBreakpoint::new(MAX_DAP_INTEGER + 1),
        DebugSourceBreakpoint::new(1).with_column(0),
        DebugSourceBreakpoint::new(1).with_column(MAX_DAP_INTEGER + 1),
        DebugSourceBreakpoint::new(1).with_condition(""),
        DebugSourceBreakpoint::new(1).with_hit_condition("x".repeat(limit + 1)),
        DebugSourceBreakpoint::new(1).with_log_message("x".repeat(limit + 1)),
    ];
    for breakpoint in invalid {
        assert!(matches!(
            f.manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(&f.source, breakpoint),
                )
                .await,
            Err(DapError::InvalidBreakpoint { .. })
        ));
    }
    assert_no_traffic(&mut f.adapter).await;
}

#[tokio::test]
async fn expected_source_revision_mismatch_fails_before_traffic() {
    let mut f = fixture("owner");
    let wrong = DebugSourceRevision {
        sha256: [9; 32],
        byte_len: 999,
    };
    assert!(matches!(
        f.manager
            .set_breakpoint(
                "owner",
                f.id,
                DebugSetBreakpointRequest::new(&f.source, DebugSourceBreakpoint::new(1))
                    .with_expected_revision(wrong),
            )
            .await,
        Err(DapError::DebugSourceRevisionMismatch { .. })
    ));
    assert_no_traffic(&mut f.adapter).await;
}

#[tokio::test]
async fn conditional_hit_conditional_and_logpoint_gates_require_exact_boolean_true() {
    let _fixture = fixture("owner");
    let capabilities = Capabilities::default();
    for (breakpoint, capability) in [
        (
            DebugSourceBreakpoint::new(1).with_condition("x"),
            "supportsConditionalBreakpoints",
        ),
        (
            DebugSourceBreakpoint::new(1).with_hit_condition("2"),
            "supportsHitConditionalBreakpoints",
        ),
        (
            DebugSourceBreakpoint::new(1).with_log_message("x"),
            "supportsLogPoints",
        ),
    ] {
        for unsupported in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!("true")),
            Some(json!(1)),
            Some(json!({})),
            Some(json!([])),
        ] {
            let mut advertised = capabilities.clone();
            if let Some(value) = unsupported {
                advertised.additional.insert(capability.to_owned(), value);
            }
            assert!(matches!(
                check_capabilities(&advertised, &breakpoint),
                Err(DapError::UnsupportedDapCapability { capability: actual, .. }) if actual == capability
            ));
        }
        let mut advertised = Capabilities::default();
        advertised
            .additional
            .insert(capability.to_owned(), json!(true));
        assert!(check_capabilities(&advertised, &breakpoint).is_ok());
    }
}
