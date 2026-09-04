use super::*;

async fn set_one(f: &mut Fixture, line: u64, adapter_id: i64) -> DebugBreakpointId {
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(line)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &set,
            Some(json!({"breakpoints":[{"id":adapter_id,"verified":true,"line":line}]})),
        )
        .await
        .unwrap();
    match task.await.unwrap().unwrap().mutation {
        DebugBreakpointMutation::Created { breakpoint_id } => breakpoint_id,
        other => panic!("unexpected mutation: {other:?}"),
    }
}

#[tokio::test]
async fn remove_last_breakpoint_sends_empty_full_source_replacement() {
    let mut f = fixture("owner");
    let id = set_one(&mut f, 1, 11).await;
    let task = {
        let manager = f.manager.clone();
        let session_id = f.id;
        tokio::spawn(async move {
            manager
                .remove_breakpoint("owner", session_id, DebugRemoveBreakpointRequest::new(id))
                .await
        })
    };
    let clear = request(&mut f.adapter).await;
    assert_eq!(clear.command, "setBreakpoints");
    assert_eq!(clear.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    assert!(clear.arguments.as_ref().unwrap().get("lines").is_none());
    assert!(
        clear
            .arguments
            .as_ref()
            .unwrap()
            .get("sourceModified")
            .is_none()
    );
    f.adapter
        .respond_ok(&clear, Some(json!({"breakpoints":[]})))
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(
        matches!(result.mutation, DebugBreakpointMutation::Removed { breakpoint_id } if breakpoint_id == id)
    );
    assert!(result.source.breakpoints.is_empty());
    assert!(
        f.manager
            .breakpoints("owner", f.id)
            .unwrap()
            .sources
            .is_empty()
    );
}

#[tokio::test]
async fn unknown_local_breakpoint_id_emits_zero_traffic() {
    let mut f = fixture("owner");
    assert!(matches!(
        f.manager
            .remove_breakpoint(
                "owner",
                f.id,
                DebugRemoveBreakpointRequest::new(DebugBreakpointId(999)),
            )
            .await,
        Err(DapError::BreakpointNotFound { .. })
    ));
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_adapter_rejection_preserves_synchronized_registry_and_retry_succeeds() {
    let mut f = fixture("owner");
    let first_id = set_one(&mut f, 1, 21).await;
    let before = f.manager.breakpoints("owner", f.id).unwrap();
    let rejected = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(2)),
                )
                .await
        })
    };
    let request_two = request(&mut f.adapter).await;
    assert_eq!(
        request_two.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":1},{"line":2}])
    );
    f.adapter
        .respond_error(&request_two, "rejected")
        .await
        .unwrap();
    assert!(matches!(
        rejected.await.unwrap(),
        Err(DapError::Response { .. })
    ));
    assert_eq!(f.manager.breakpoints("owner", f.id).unwrap(), before);

    let retry = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(2)),
                )
                .await
        })
    };
    let retried = request(&mut f.adapter).await;
    assert_eq!(
        retried.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":1},{"line":2}])
    );
    f.adapter
        .respond_ok(
            &retried,
            Some(json!({"breakpoints":[{"id":21,"verified":true},{"id":22,"verified":true}]})),
        )
        .await
        .unwrap();
    let result = retry.await.unwrap().unwrap();
    assert_eq!(result.source.breakpoints.len(), 2);
    assert_eq!(result.source.breakpoints[0].id, first_id);
}

#[tokio::test]
async fn two_source_adapter_id_collision_rejects_and_id_only_events_mutate_neither_source() {
    let mut f = fixture("owner");
    set_one(&mut f, 1, 21).await;
    let second = f.root.join("second.rs");
    std::fs::write(&second, b"fn second() {}\n").unwrap();
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(second, DebugSourceBreakpoint::new(1)),
            )
            .await
    });
    let request = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &request,
            Some(json!({"breakpoints":[{"id":21,"verified":true}]})),
        )
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::InvalidSetBreakpointsResponse { .. })
    ));
    let before = f.manager.breakpoints("owner", f.id).unwrap();
    f.adapter
        .event(
            "breakpoint",
            Some(json!({"reason":"changed","breakpoint":{"id":21,"verified":false}})),
        )
        .await
        .unwrap();
    f.adapter
        .event(
            "breakpoint",
            Some(json!({"reason":"removed","breakpoint":{"id":21}})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if f.manager
                .breakpoints("owner", f.id)
                .unwrap()
                .unmatched_adapter_events
                == before.unmatched_adapter_events + 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let after = f.manager.breakpoints("owner", f.id).unwrap();
    assert_eq!(after.sources, before.sources);
    assert_eq!(after.total_breakpoints, before.total_breakpoints);
}

#[tokio::test]
async fn malformed_success_marks_indeterminate_and_next_mutation_resets_without_source_modified() {
    let mut f = fixture("owner");
    let malformed = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let first = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(&first, Some(json!({"breakpoints":[{}]})))
        .await
        .unwrap();
    assert!(matches!(
        malformed.await.unwrap(),
        Err(DapError::InvalidSetBreakpointsResponse { .. })
    ));
    assert_eq!(
        f.manager.breakpoints("owner", f.id).unwrap().sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );

    let retry = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(2)),
                )
                .await
        })
    };
    let reset = request(&mut f.adapter).await;
    assert_eq!(reset.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    assert!(
        reset
            .arguments
            .as_ref()
            .unwrap()
            .get("sourceModified")
            .is_none()
    );
    f.adapter
        .respond_ok(&reset, Some(json!({"breakpoints":[]})))
        .await
        .unwrap();
    let desired = request(&mut f.adapter).await;
    assert_eq!(
        desired.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":1},{"line":2}])
    );
    f.adapter
        .respond_ok(
            &desired,
            Some(json!({"breakpoints":[{"id":31,"verified":true},{"id":32,"verified":true}]})),
        )
        .await
        .unwrap();
    assert_eq!(
        retry.await.unwrap().unwrap().source.synchronization,
        DebugBreakpointSynchronization::Synchronized
    );
}

#[tokio::test]
async fn failed_compensating_clear_leaves_public_source_indeterminate() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    std::fs::write(&f.source, b"changed while pending").unwrap();
    f.adapter
        .respond_ok(
            &set,
            Some(json!({"breakpoints":[{"id":41,"verified":true}]})),
        )
        .await
        .unwrap();
    let compensation = request(&mut f.adapter).await;
    assert_eq!(
        compensation.arguments.as_ref().unwrap()["breakpoints"],
        json!([])
    );
    assert_eq!(
        compensation.arguments.as_ref().unwrap()["sourceModified"],
        json!(true)
    );
    f.adapter
        .respond_error(&compensation, "cannot clear")
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::BreakpointReconciliationIndeterminate { .. })
    ));
    let snapshot = f.manager.breakpoints("owner", f.id).unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    assert_eq!(
        snapshot.sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );
}

#[tokio::test]
async fn queued_event_at_or_before_response_sequence_is_discarded() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    f.adapter.event("breakpoint", Some(json!({"reason":"changed","breakpoint":{"id":51,"verified":false,"reason":"pending"}}))).await.unwrap();
    f.adapter
        .respond_ok(
            &set,
            Some(json!({"breakpoints":[{"id":51,"verified":true}]})),
        )
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(result.source.breakpoints[0].verified);
    assert_eq!(result.source.breakpoints[0].reason, None);
}

#[tokio::test]
async fn source_revision_change_before_dispatch_resets_stale_source_and_reports_discarded_ids() {
    let mut f = fixture("owner");
    let stale_id = set_one(&mut f, 1, 61).await;
    std::fs::write(&f.source, b"new revision\n").unwrap();
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        let id = f.id;
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(2)),
                )
                .await
        })
    };
    let reset = request(&mut f.adapter).await;
    assert_eq!(reset.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    assert_eq!(
        reset.arguments.as_ref().unwrap()["sourceModified"],
        json!(true)
    );
    f.adapter
        .respond_ok(&reset, Some(json!({"breakpoints":[]})))
        .await
        .unwrap();
    let desired = request(&mut f.adapter).await;
    assert_eq!(
        desired.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":2}])
    );
    assert!(
        desired
            .arguments
            .as_ref()
            .unwrap()
            .get("sourceModified")
            .is_none()
    );
    f.adapter
        .respond_ok(
            &desired,
            Some(json!({"breakpoints":[{"id":62,"verified":true}]})),
        )
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.discarded_stale_breakpoints, vec![stale_id]);
    assert_eq!(result.source.breakpoints.len(), 1);
    assert_eq!(result.source.breakpoints[0].requested.line, 2);
}
