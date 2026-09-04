use super::*;

async fn complete_set(
    f: &mut Fixture,
    source: PathBuf,
    breakpoint: DebugSourceBreakpoint,
    adapter_id: i64,
) -> Result<DebugBreakpointMutationResult> {
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, breakpoint),
            )
            .await
    });
    let request = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &request,
            Some(json!({"breakpoints":request.arguments.as_ref().unwrap()["breakpoints"].as_array().unwrap().iter().enumerate().map(|(index, _)| json!({"id":adapter_id + index as i64,"verified":true})).collect::<Vec<_>>()})),
        )
        .await
        .unwrap();
    task.await.unwrap()
}

#[tokio::test]
async fn source_per_source_and_total_limits_accept_boundary_and_reject_plus_one_without_traffic() {
    for (sources, per_source, total, second_same_source, expected_scope) in [
        (1, 2, 2, false, "sources"),
        (2, 1, 2, true, "per-source"),
        (2, 1, 1, false, "total"),
    ] {
        let mut f = fixture_with_operations(
            "owner",
            DebugOperationConfig {
                operation_timeout: Duration::from_secs(2),
                max_breakpoint_sources: sources,
                max_breakpoints_per_source: per_source,
                max_total_breakpoints: total,
                ..Default::default()
            },
        );
        let source = f.source.clone();
        complete_set(&mut f, source, DebugSourceBreakpoint::new(1), 10)
            .await
            .unwrap();
        let other = if second_same_source {
            f.source.clone()
        } else {
            let path = f.root.join("other.rs");
            std::fs::write(&path, b"fn other() {}\n").unwrap();
            path
        };
        let before = f.manager.breakpoints("owner", f.id).unwrap();
        assert!(matches!(
            f.manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(other, DebugSourceBreakpoint::new(2)),
                )
                .await,
            Err(DapError::BreakpointLimitExceeded { scope, .. }) if scope == expected_scope
        ));
        assert_eq!(f.manager.breakpoints("owner", f.id).unwrap(), before);
        assert!(
            timeout(Duration::from_millis(20), f.adapter.recv())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn expression_and_canonical_source_path_limits_accept_boundary_and_reject_plus_one() {
    for (capability, boundary, plus_one) in [
        (
            "supportsConditionalBreakpoints",
            DebugSourceBreakpoint::new(1).with_condition("éé"),
            DebugSourceBreakpoint::new(2).with_condition("aéé"),
        ),
        (
            "supportsHitConditionalBreakpoints",
            DebugSourceBreakpoint::new(1).with_hit_condition("éé"),
            DebugSourceBreakpoint::new(2).with_hit_condition("aéé"),
        ),
        (
            "supportsLogPoints",
            DebugSourceBreakpoint::new(1).with_log_message("éé"),
            DebugSourceBreakpoint::new(2).with_log_message("aéé"),
        ),
    ] {
        let mut f = fixture_with_operations(
            "owner",
            DebugOperationConfig {
                operation_timeout: Duration::from_secs(2),
                max_breakpoint_expression_bytes: 4,
                ..Default::default()
            },
        );
        {
            let entry = f.manager.core.entry(f.id).unwrap();
            lock(&entry.data)
                .capabilities
                .additional
                .insert(capability.into(), json!(true));
        }
        let source = f.source.clone();
        complete_set(&mut f, source, boundary, 20).await.unwrap();
        let before = f.manager.breakpoints("owner", f.id).unwrap();
        assert!(matches!(
            f.manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(&f.source, plus_one),
                )
                .await,
            Err(DapError::InvalidBreakpoint { message })
                if message == "breakpoint expression is empty or exceeds configured byte limit"
        ));
        assert_eq!(f.manager.breakpoints("owner", f.id).unwrap(), before);
        assert!(
            timeout(Duration::from_millis(20), f.adapter.recv())
                .await
                .is_err()
        );
    }

    for accepted in [true, false] {
        let root = std::env::temp_dir().join(format!(
            "jcode-dap-path-limit-{}-{}",
            std::process::id(),
            crate::session::next_manager_id().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = root.join("source.rs");
        std::fs::write(&source, b"x").unwrap();
        let canonical = source.canonicalize().unwrap();
        let canonical_len = canonical.as_os_str().as_encoded_bytes().len();
        let manager = DebugSessionManager::new_with_operation_config(
            DebugSessionManagerConfig::default(),
            DebugOperationConfig {
                operation_timeout: Duration::from_secs(2),
                max_source_path_bytes: canonical_len - usize::from(!accepted),
                ..Default::default()
            },
        )
        .unwrap();
        let (client, mut adapter) = FakeAdapter::pair(1024 * 1024);
        let mut reservation = manager
            .reserve(NewDebugSession {
                owner_session_id: "owner".into(),
                workspace: DebugWorkspaceKey::new(&root, "path-limit").unwrap(),
                adapter_id: "fake".into(),
                start: Some(DebugSessionStart::Launch {
                    program: source.clone(),
                    cwd: root.clone(),
                }),
            })
            .unwrap();
        reservation.attach_client(client).unwrap();
        reservation.mark_configuring().unwrap();
        reservation.mark_running().unwrap();
        let id = reservation.commit().unwrap();
        if accepted {
            let task = tokio::spawn({
                let manager = manager.clone();
                let source = source.clone();
                async move {
                    manager
                        .set_breakpoint(
                            "owner",
                            id,
                            DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                        )
                        .await
                }
            });
            let request = request(&mut adapter).await;
            assert_eq!(
                request.arguments.as_ref().unwrap()["source"]["path"],
                json!(canonical)
            );
            adapter
                .respond_ok(&request, Some(json!({"breakpoints":[{"verified":true}]})))
                .await
                .unwrap();
            task.await.unwrap().unwrap();
        } else {
            assert!(matches!(
                manager
                    .set_breakpoint(
                        "owner",
                        id,
                        DebugSetBreakpointRequest::new(&source, DebugSourceBreakpoint::new(1)),
                    )
                    .await,
                Err(DapError::InvalidDebugSource { path, message })
                    if path == source && message == "source path exceeds configured byte limit"
            ));
            assert!(manager.breakpoints("owner", id).unwrap().sources.is_empty());
            assert!(
                timeout(Duration::from_millis(20), adapter.recv())
                    .await
                    .is_err()
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
