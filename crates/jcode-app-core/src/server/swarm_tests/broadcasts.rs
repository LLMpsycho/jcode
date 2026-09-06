#[tokio::test]
async fn broadcast_swarm_plan_with_previous_includes_newly_ready_ids() {
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![
                PlanItem {
                    content: "setup".to_string(),
                    status: "completed".to_string(),
                    priority: "high".to_string(),
                    id: "setup".to_string(),
                    subsystem: None,
                    file_scope: Vec::new(),
                    blocked_by: Vec::new(),
                    assigned_to: None,
                },
                PlanItem {
                    content: "follow-up".to_string(),
                    status: "queued".to_string(),
                    priority: "high".to_string(),
                    id: "follow-up".to_string(),
                    subsystem: None,
                    file_scope: Vec::new(),
                    blocked_by: vec!["setup".to_string()],
                    assigned_to: None,
                },
            ],
            version: 2,
            participants: HashSet::from(["worker".to_string()]),
            task_progress: HashMap::new(),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])));
    let (worker, mut worker_rx) = swarm_member("worker", "agent", false);
    let swarm_members = Arc::new(RwLock::new(HashMap::from([("worker".to_string(), worker)])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));
    let previous_items = vec![
        PlanItem {
            content: "setup".to_string(),
            status: "running".to_string(),
            priority: "high".to_string(),
            id: "setup".to_string(),
            subsystem: None,
            file_scope: Vec::new(),
            blocked_by: Vec::new(),
            assigned_to: Some("worker".to_string()),
        },
        PlanItem {
            content: "follow-up".to_string(),
            status: "queued".to_string(),
            priority: "high".to_string(),
            id: "follow-up".to_string(),
            subsystem: None,
            file_scope: Vec::new(),
            blocked_by: vec!["setup".to_string()],
            assigned_to: None,
        },
    ];

    broadcast_swarm_plan_with_previous(
        "swarm-1",
        Some("task_completed".to_string()),
        Some(&previous_items),
        &swarm_plans,
        &swarm_members,
        &swarms_by_id,
    )
    .await;

    match worker_rx.recv().await.expect("swarm plan event") {
        ServerEvent::SwarmPlan {
            reason,
            summary: Some(summary),
            ..
        } => {
            assert_eq!(reason.as_deref(), Some("task_completed"));
            assert_eq!(summary.newly_ready_ids, vec!["follow-up".to_string()]);
            assert_eq!(summary.next_ready_ids, vec!["follow-up".to_string()]);
        }
        other => panic!("expected SwarmPlan event, got {other:?}"),
    }
}
/// Deterministic demonstration of the mutate->broadcast version-inversion
/// race (wiring-audit.plan-broadcast-ordering).
///
/// `broadcast_swarm_plan_with_previous` snapshots `(version, items)` under
/// `swarm_plans.read()`, releases the lock, and only later (after further
/// await points on `swarms_by_id.read()` / `swarm_members.read()`) sends
/// on `member.event_tx`. A second mutator can bump the version AND
/// complete its own broadcast inside that window, so a single ordered
/// mpsc channel can deliver v6 before v5.
///
/// This test parks broadcast A (snapshot v5, empty participants, so it
/// must await `swarms_by_id.read()`) behind a held `swarms_by_id.write()`
/// guard, lets mutator B bump to v6 and broadcast it, then releases A.
/// The worker receives [6, 5]: inverted versions on one channel.
///
/// If this test starts failing with versions == [6, 6] or [5, 6], the
/// race has been fixed (e.g. by holding the plan lock through send or by
/// stamping a send-order sequence); update the wiring audit and consider
/// whether the TUI-side monotonicity guard (server_events.rs SwarmPlan
/// handler currently overwrites `swarm_plan_version` unconditionally) is
/// still needed.
#[tokio::test]
async fn swarm_plan_broadcast_versions_can_invert_on_one_member_channel() {
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![plan_item("t1", "task one")],
            version: 5,
            // Empty participants: broadcast A takes the swarms_by_id
            // fallback path, which is where we deterministically park it.
            participants: HashSet::new(),
            task_progress: HashMap::new(),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])));
    let (worker, mut worker_rx) = swarm_member("worker", "agent", false);
    let swarm_members = Arc::new(RwLock::new(HashMap::from([("worker".to_string(), worker)])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));

    // Hold a write guard on swarms_by_id so broadcast A parks after it
    // has already snapshotted version 5 from swarm_plans.
    let gate = swarms_by_id.write().await;

    let a = tokio::spawn({
        let swarm_plans = Arc::clone(&swarm_plans);
        let swarm_members = Arc::clone(&swarm_members);
        let swarms_by_id = Arc::clone(&swarms_by_id);
        async move {
            broadcast_swarm_plan(
                "swarm-1",
                Some("mutator_1".to_string()),
                &swarm_plans,
                &swarm_members,
                &swarms_by_id,
            )
            .await;
        }
    });
    // Current-thread test runtime: yielding runs A until it parks on the
    // contended swarms_by_id.read().await, past its v5 snapshot.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    // Mutator B: bump to v6 and register an explicit participant so B's
    // broadcast skips the swarms_by_id fallback and is not blocked by
    // the gate. This mirrors real mutators (write, release, broadcast).
    {
        let mut plans = swarm_plans.write().await;
        let vp = plans.get_mut("swarm-1").expect("plan");
        vp.version = 6;
        vp.participants.insert("worker".to_string());
    }
    broadcast_swarm_plan(
        "swarm-1",
        Some("mutator_2".to_string()),
        &swarm_plans,
        &swarm_members,
        &swarms_by_id,
    )
    .await;

    // Release A: it resumes with its stale v5 snapshot and sends it
    // after v6 on the same ordered channel.
    drop(gate);
    a.await.expect("broadcast task");

    let mut versions = Vec::new();
    while let Ok(event) = worker_rx.try_recv() {
        if let ServerEvent::SwarmPlan { version, .. } = event {
            versions.push(version);
        }
    }
    assert_eq!(
        versions,
        vec![6, 5],
        "expected version inversion on one member channel; if this fails \
         the mutate->broadcast race may have been fixed (update the \
         wiring audit)"
    );
}
/// Deterministic demonstration of the SwarmStatus immediate-path
/// snapshot-vs-send inversion (wiring-audit.status-proposal-ordering).
///
/// `broadcast_swarm_status_now` snapshots member statuses under
/// `swarm_members.read()`, drops the guard, then awaits
/// `fanout_session_event` (a `swarm_members.write()` acquisition) before
/// sending. Swarms below `JCODE_SWARM_STATUS_DEBOUNCE_MEMBER_THRESHOLD`
/// (default 2) take this immediate, non-debounced path on every status
/// change, so two concurrent broadcasts can deliver an old snapshot after
/// a newer one on the same ordered mpsc channel. A last-write-wins
/// consumer (the TUI SwarmStatus handler) is then left showing the stale
/// status until the next unrelated broadcast.
///
/// Unlike the SwarmPlan inversion test above, there is no second lock we
/// can gate on: the status path snapshots from the same `swarm_members`
/// lock it later writes, so holding any guard also blocks the mutator.
/// Instead this test uses tokio's cooperative budget (128 units per task
/// poll on a current-thread runtime; every RwLock acquisition consumes
/// exactly one). Draining 126 units leaves broadcast A exactly enough for
/// `swarms_by_id.read()` and the `swarm_members.read()` snapshot, forcing
/// a yield at the (uncontended) `swarm_members.write()` inside
/// `fanout_session_event`, i.e. precisely inside the race window between
/// snapshot and send.
///
/// If this test starts failing with `["running", "running"]` or
/// `["ready", "running"]`, the race has been fixed (e.g. by holding the
/// read lock through the send, or by stamping a monotonic sequence on
/// SwarmStatus and dropping stale ones consumer-side); update the wiring
/// audit. If it fails because broadcast A parks somewhere else, the tokio
/// coop budget constants changed: re-derive the `128 - 2` drain count.
#[tokio::test]
async fn swarm_status_immediate_broadcasts_can_invert_on_one_member_channel() {
    let (worker, mut worker_rx) = swarm_member("worker", "agent", false);
    let swarm_members = Arc::new(RwLock::new(HashMap::from([("worker".to_string(), worker)])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));

    // Broadcast A: snapshots status "ready", then is forced to yield at
    // the fanout write acquisition, before sending.
    let a = tokio::spawn({
        let swarm_members = Arc::clone(&swarm_members);
        let swarms_by_id = Arc::clone(&swarms_by_id);
        async move {
            // Initial task budget is 128. Leave exactly 2 units so the two
            // read acquisitions (session-id list + status snapshot)
            // succeed and the fanout write acquisition forces a yield.
            for _ in 0..126 {
                tokio::task::coop::consume_budget().await;
            }
            broadcast_swarm_status("swarm-1", &swarm_members, &swarms_by_id).await;
        }
    });
    // Single yield on the current-thread runtime: A runs its entire first
    // poll (budget drain + both reads) and parks after snapshotting
    // "ready". Its coop yield happens *before* joining the lock queue, so
    // every acquisition below is uncontended and the mutator finishes
    // within one poll, before A is re-polled.
    tokio::task::yield_now().await;

    // Concurrent mutator: flips the status and completes its own
    // immediate broadcast while A is parked between snapshot and send.
    {
        let mut members = swarm_members.write().await;
        members.get_mut("worker").expect("worker member").status = "running".to_string();
    }
    broadcast_swarm_status("swarm-1", &swarm_members, &swarms_by_id).await;

    // Release A: it resumes with a fresh budget and sends its stale
    // "ready" snapshot after "running" on the same ordered channel.
    a.await.expect("broadcast task");

    let mut statuses = Vec::new();
    while let Ok(event) = worker_rx.try_recv() {
        if let ServerEvent::SwarmStatus { members } = event {
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].session_id, "worker");
            statuses.push(members[0].status.clone());
        }
    }
    assert_eq!(
        statuses,
        vec!["running".to_string(), "ready".to_string()],
        "expected status inversion (new-then-old) on one member channel; \
         if this fails with the correct order, the snapshot-vs-send race \
         may have been fixed (update the wiring audit)"
    );
}
/// Restored (persisted) plan participants with dead channels starve live
/// swarm members of plan broadcasts: the fallback to swarms_by_id only
/// triggers when `participants` is EMPTY, so a participant set that only
/// contains stale sessions (e.g. restored after a server restart, where
/// `from_persisted_member` gives every member a closed event_tx) means
/// nobody receives the snapshot, not even live members of the swarm.
#[tokio::test]
async fn stale_participants_starve_live_members_of_plan_broadcasts() {
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![plan_item("t1", "task one")],
            version: 7,
            // "ghost" is a participant restored from disk whose session
            // no longer exists in this server process.
            participants: HashSet::from(["ghost".to_string()]),
            task_progress: HashMap::new(),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])));
    // Ghost member as produced by swarm_persistence restore: present in
    // the member map but with a closed event channel.
    let (ghost, ghost_rx) = swarm_member("ghost", "agent", true);
    drop(ghost_rx);
    let (live, mut live_rx) = swarm_member("live", "agent", false);
    let swarm_members = Arc::new(RwLock::new(HashMap::from([
        ("ghost".to_string(), ghost),
        ("live".to_string(), live),
    ])));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["ghost".to_string(), "live".to_string()]),
    )])));

    broadcast_swarm_plan(
        "swarm-1",
        Some("test".to_string()),
        &swarm_plans,
        &swarm_members,
        &swarms_by_id,
    )
    .await;

    assert!(
        live_rx.try_recv().is_err(),
        "live member unexpectedly received the plan broadcast; stale \
         participant starvation may have been fixed (update the wiring \
         audit)"
    );
}
