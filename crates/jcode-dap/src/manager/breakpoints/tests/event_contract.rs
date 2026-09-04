use super::*;

fn registry_with_breakpoint() -> (BreakpointRegistry, PathBuf, DebugBreakpointId) {
    let path = PathBuf::from("/workspace/source.rs");
    let id = DebugBreakpointId(1);
    let revision = DebugSourceRevision {
        sha256: [1; 32],
        byte_len: 3,
    };
    let breakpoint = DebugBreakpoint {
        id,
        source: PathBuf::from("source.rs"),
        source_revision: revision.clone(),
        requested: DebugSourceBreakpoint::new(7).with_condition("original"),
        verified: true,
        reason: None,
        message: None,
        message_truncated_prefix_bytes: 0,
        adapter_id: Some(77),
        resolved: DebugBreakpointLocation {
            line: Some(7),
            column: Some(1),
            end_line: None,
            end_column: None,
        },
    };
    let mut registry = BreakpointRegistry::default();
    registry.sources.insert(
        path.clone(),
        SourceRecord {
            original: path.clone(),
            relative: PathBuf::from("source.rs"),
            revision,
            generation: 1,
            synchronization: DebugBreakpointSynchronization::Synchronized,
            breakpoints: BTreeMap::from([(id, breakpoint)]),
        },
    );
    (registry, path, id)
}

#[test]
fn changed_event_updates_only_adapter_derived_breakpoint_fields() {
    let (mut registry, path, id) = registry_with_breakpoint();
    let before = registry.sources[&path].breakpoints[&id].clone();
    apply_breakpoint_event(
        &mut registry,
        4,
        json!({"reason":"changed","breakpoint":{"id":77,"verified":false,"reason":"pending","message":"waiting","line":8,"column":2,"endLine":9,"endColumn":3}}),
        &DebugOperationConfig::default(),
    );
    let after = &registry.sources[&path].breakpoints[&id];
    assert_eq!(after.id, before.id);
    assert_eq!(after.source, before.source);
    assert_eq!(after.source_revision, before.source_revision);
    assert_eq!(after.requested, before.requested);
    assert_eq!(after.adapter_id, before.adapter_id);
    assert!(!after.verified);
    assert_eq!(after.reason, Some(DebugBreakpointReason::Pending));
    assert_eq!(after.message.as_deref(), Some("waiting"));
    assert_eq!(
        after.resolved,
        DebugBreakpointLocation {
            line: Some(8),
            column: Some(2),
            end_line: Some(9),
            end_column: Some(3)
        }
    );
}

#[test]
fn new_missing_id_unknown_id_invalid_source_and_unknown_reason_events_increment_unmatched_without_state_loss()
 {
    let (mut registry, path, id) = registry_with_breakpoint();
    let before = registry.sources[&path].breakpoints[&id].clone();
    let events = [
        json!({"reason":"new","breakpoint":{"id":77,"verified":true}}),
        json!({"reason":"changed","breakpoint":{"verified":true}}),
        json!({"reason":"changed","breakpoint":{"id":999,"verified":true}}),
        json!({"reason":"changed","breakpoint":{"id":77,"verified":true,"line":0}}),
        json!({"reason":"adapter-specific","breakpoint":{"id":77,"verified":true}}),
    ];
    for (index, event) in events.into_iter().enumerate() {
        apply_breakpoint_event(
            &mut registry,
            i64::try_from(index).unwrap(),
            event,
            &DebugOperationConfig::default(),
        );
    }
    assert_eq!(registry.unmatched_events, 5);
    assert_eq!(registry.sources[&path].breakpoints[&id], before);
}

#[test]
fn breakpoint_snapshot_ordering_and_totals_are_deterministic() {
    let (mut registry, path, _) = registry_with_breakpoint();
    let mut second = registry.sources.remove(&path).unwrap();
    second.original = PathBuf::from("/workspace/z.rs");
    second.relative = PathBuf::from("z.rs");
    registry
        .sources
        .insert(PathBuf::from("/workspace/z.rs"), second);
    let mut first = SourceRecord {
        original: PathBuf::from("/workspace/a.rs"),
        relative: PathBuf::from("a.rs"),
        revision: DebugSourceRevision {
            sha256: [2; 32],
            byte_len: 1,
        },
        generation: 3,
        synchronization: DebugBreakpointSynchronization::Synchronized,
        breakpoints: BTreeMap::new(),
    };
    for local in [DebugBreakpointId(4), DebugBreakpointId(2)] {
        first.breakpoints.insert(
            local,
            DebugBreakpoint {
                id: local,
                source: PathBuf::from("a.rs"),
                source_revision: first.revision.clone(),
                requested: DebugSourceBreakpoint::new(local.get()),
                verified: true,
                reason: None,
                message: None,
                message_truncated_prefix_bytes: 0,
                adapter_id: None,
                resolved: DebugBreakpointLocation::default(),
            },
        );
    }
    registry
        .sources
        .insert(PathBuf::from("/workspace/a.rs"), first);
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.total_breakpoints, 3);
    assert_eq!(
        snapshot
            .sources
            .iter()
            .map(|source| source.source.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from("a.rs"), PathBuf::from("z.rs")]
    );
    assert_eq!(
        snapshot.sources[0]
            .breakpoints
            .iter()
            .map(|breakpoint| breakpoint.id)
            .collect::<Vec<_>>(),
        vec![DebugBreakpointId(2), DebugBreakpointId(4)]
    );
}

#[test]
fn unmatched_adapter_event_diagnostic_saturates_without_state_loss() {
    let (mut registry, _, _) = registry_with_breakpoint();
    registry.unmatched_events = u64::MAX;
    let before = registry.snapshot();
    apply_breakpoint_event(
        &mut registry,
        99,
        json!({"reason":"changed","breakpoint":{"id":999,"verified":false}}),
        &DebugOperationConfig::default(),
    );
    let after = registry.snapshot();
    assert_eq!(after.unmatched_adapter_events, u64::MAX);
    assert_eq!(after.sources, before.sources);
}

#[test]
fn adapter_message_limit_accepts_boundary_and_truncates_boundary_plus_one() {
    let (mut registry, _, id) = registry_with_breakpoint();
    let adapter_id = registry.sources.values().next().unwrap().breakpoints[&id]
        .adapter_id
        .unwrap();
    let config = DebugOperationConfig {
        max_breakpoint_message_bytes: 4,
        ..DebugOperationConfig::default()
    };
    apply_breakpoint_event(
        &mut registry,
        1,
        json!({"reason":"changed","breakpoint":{"id":adapter_id,"message":"1234"}}),
        &config,
    );
    let boundary = registry.snapshot();
    assert_eq!(
        boundary.sources[0].breakpoints[0].message.as_deref(),
        Some("1234")
    );
    assert_eq!(
        boundary.sources[0].breakpoints[0].message_truncated_prefix_bytes,
        0
    );
    apply_breakpoint_event(
        &mut registry,
        2,
        json!({"reason":"changed","breakpoint":{"id":adapter_id,"message":"12345"}}),
        &config,
    );
    let plus_one = registry.snapshot();
    assert_eq!(
        plus_one.sources[0].breakpoints[0].message.as_deref(),
        Some("2345")
    );
    assert_eq!(
        plus_one.sources[0].breakpoints[0].message_truncated_prefix_bytes,
        1
    );
}

#[tokio::test]
async fn public_breakpoint_snapshot_bounds_adapter_message_at_utf8_byte_boundary() {
    let mut f = fixture_with_operations(
        "owner",
        DebugOperationConfig {
            max_breakpoint_message_bytes: 4,
            ..DebugOperationConfig::default()
        },
    );
    let manager = f.manager.clone();
    let id = f.id;
    let source = f.source.clone();
    let task = tokio::spawn(async move {
        manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
            )
            .await
    });
    let request = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &request,
            Some(json!({"breakpoints":[{"id":7,"verified":true}]})),
        )
        .await
        .unwrap();
    task.await.unwrap().unwrap();

    for (message, retained, truncated) in [("éé", "éé", 0), ("aéé", "éé", 1)] {
        f.adapter
            .event(
                "breakpoint",
                Some(json!({
                    "reason":"changed",
                    "breakpoint":{"id":7,"verified":true,"message":message}
                })),
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                let breakpoint = f
                    .manager
                    .breakpoints("owner", f.id)
                    .unwrap()
                    .sources
                    .into_iter()
                    .flat_map(|source| source.breakpoints)
                    .next()
                    .unwrap();
                if breakpoint.message.as_deref() == Some(retained)
                    && breakpoint.message_truncated_prefix_bytes == truncated
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
