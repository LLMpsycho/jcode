use super::{
    broadcast_swarm_plan, broadcast_swarm_plan_with_previous, broadcast_swarm_status,
    member_in_status_broadcast, member_status_is_dead, now_unix_ms, parse_swarm_tasks,
    refresh_swarm_task_staleness, remove_session_from_swarm, salvage_assignments_of_dead_member,
    swarm_ancestors, swarm_is_self_or_ancestor, swarm_spawn_depth, touch_swarm_task_progress,
    update_member_status, update_member_status_with_report,
};
use crate::plan::PlanItem;
use crate::protocol::{NotificationType, ServerEvent};
use crate::server::{SwarmMember, VersionedPlan};
use jcode_swarm_core::{
    append_swarm_completion_report_instructions, summarize_plan_items, truncate_detail,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};

fn plan_item(id: &str, content: &str) -> PlanItem {
    PlanItem {
        content: content.to_string(),
        status: "pending".to_string(),
        priority: "medium".to_string(),
        id: id.to_string(),
        subsystem: None,
        file_scope: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }
}

#[test]
fn truncate_detail_collapses_whitespace_and_ellipsizes() {
    assert_eq!(truncate_detail("hello   there\nworld", 11), "hello th...");
}

#[test]
fn summarize_plan_items_limits_output() {
    let items = vec![
        plan_item("1", "inspect"),
        plan_item("2", "refactor"),
        plan_item("3", "test"),
    ];

    assert_eq!(
        summarize_plan_items(&items, 2),
        "inspect; refactor (+1 more)"
    );
}

#[test]
fn parse_swarm_tasks_accepts_wrapped_json() {
    let text = "Plan:\n[{\"description\":\"A\",\"prompt\":\"B\",\"subagent_type\":\"general\"}]";
    let tasks = parse_swarm_tasks(text);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].description, "A");
    assert_eq!(tasks[0].prompt, "B");
    assert_eq!(tasks[0].subagent_type.as_deref(), Some("general"));
}

#[test]
fn append_swarm_completion_report_instructions_is_idempotent() {
    let prompt = "Implement the task.";
    let with_instructions = append_swarm_completion_report_instructions(prompt);

    assert!(with_instructions.starts_with(prompt));
    assert!(with_instructions.contains("SWARM COMPLETION REPORT REQUIRED"));
    assert!(with_instructions.contains("swarm tool with action=\"report\""));
    assert_eq!(
        append_swarm_completion_report_instructions(&with_instructions),
        with_instructions
    );
}

fn swarm_member(
    session_id: &str,
    role: &str,
    is_headless: bool,
) -> (SwarmMember, mpsc::UnboundedReceiver<ServerEvent>) {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    (
        SwarmMember {
            session_id: session_id.to_string(),
            event_tx,
            event_txs: HashMap::new(),
            working_dir: None,
            swarm_id: Some("swarm-1".to_string()),
            swarm_enabled: true,
            status: "ready".to_string(),
            detail: None,
            task_label: None,
            friendly_name: Some(session_id.to_string()),
            report_back_to_session_id: None,
            latest_completion_report: None,
            role: role.to_string(),
            joined_at: Instant::now(),
            last_status_change: Instant::now(),
            is_headless,
            output_tail: None,
            todo_progress: None,
            todo_items: Vec::new(),
            runtime: crate::protocol::SwarmMemberRuntime::default(),
        },
        event_rx,
    )
}

fn member_with_parent(session_id: &str, parent: Option<&str>) -> SwarmMember {
    let (mut member, _rx) = swarm_member(session_id, "agent", false);
    member.report_back_to_session_id = parent.map(str::to_string);
    member
}

#[test]
fn idle_spawned_worker_reap_selects_only_finished_idle_spawned_agents() {
    use super::idle_spawned_worker_reap_candidates;

    let idle_after = Duration::from_secs(60);
    let old = Instant::now() - Duration::from_secs(120);

    // Finished spawned worker, idle past the window: reapable.
    let mut reapable = member_with_parent("reapable", Some("coord"));
    reapable.status = "ready".to_string();
    reapable.last_status_change = old;

    // Terminal-status spawned worker: reapable.
    let mut stopped = member_with_parent("stopped", Some("coord"));
    stopped.status = "completed".to_string();
    stopped.last_status_change = old;

    // Same shape but user-created (no spawner): never reaped.
    let mut user_owned = member_with_parent("user-owned", None);
    user_owned.status = "ready".to_string();
    user_owned.last_status_change = old;

    // Spawned but still running: not reaped.
    let mut running = member_with_parent("running", Some("coord"));
    running.status = "running".to_string();
    running.last_status_change = old;

    // Spawned and finished, but recently: not reaped yet.
    let mut fresh = member_with_parent("fresh", Some("coord"));
    fresh.status = "ready".to_string();

    // Spawned coordinator (sub-swarm manager): never reaped by role.
    let mut sub_coordinator = member_with_parent("sub-coord", Some("coord"));
    sub_coordinator.role = "coordinator".to_string();
    sub_coordinator.status = "ready".to_string();
    sub_coordinator.last_status_change = old;

    let members: HashMap<String, SwarmMember> = [
        reapable,
        stopped,
        user_owned,
        running,
        fresh,
        sub_coordinator,
    ]
    .into_iter()
    .map(|member| (member.session_id.clone(), member))
    .collect();

    let mut candidates = idle_spawned_worker_reap_candidates(&members, idle_after);
    candidates.sort();
    assert_eq!(
        candidates,
        vec!["reapable".to_string(), "stopped".to_string()]
    );
}

#[test]
fn idle_worker_reap_window_env_zero_disables() {
    // Note: mutating the process env in tests is racy in general, but this
    // env var is read on every call (not cached), and no other test touches
    // it.
    unsafe {
        std::env::set_var("JCODE_SWARM_IDLE_WORKER_REAP_SECS", "0");
    }
    assert_eq!(super::swarm_idle_worker_reap_after(), None);
    unsafe {
        std::env::set_var("JCODE_SWARM_IDLE_WORKER_REAP_SECS", "90");
    }
    assert_eq!(
        super::swarm_idle_worker_reap_after(),
        Some(Duration::from_secs(90))
    );
    unsafe {
        std::env::remove_var("JCODE_SWARM_IDLE_WORKER_REAP_SECS");
    }
    assert!(super::swarm_idle_worker_reap_after().is_some());
}

#[test]
fn status_broadcast_keeps_live_and_recently_terminal_members_only() {
    let retention = Duration::from_secs(900);

    let (live, _rx) = swarm_member("live", "agent", false);
    assert!(member_in_status_broadcast(&live, retention));

    let (mut fresh_terminal, _rx) = swarm_member("fresh", "agent", false);
    fresh_terminal.status = "completed".to_string();
    assert!(member_in_status_broadcast(&fresh_terminal, retention));

    let (mut stale_terminal, _rx) = swarm_member("stale", "agent", false);
    stale_terminal.status = "stopped".to_string();
    stale_terminal.last_status_change = Instant::now() - Duration::from_secs(901);
    assert!(!member_in_status_broadcast(&stale_terminal, retention));

    // A stale *live* status is never filtered, no matter how old.
    let (mut old_live, _rx) = swarm_member("old-live", "agent", false);
    old_live.last_status_change = Instant::now() - Duration::from_secs(100_000);
    assert!(member_in_status_broadcast(&old_live, retention));
}

#[test]
fn swarm_depth_and_ancestry_follow_report_back_chain() {
    let mut members: HashMap<String, SwarmMember> = HashMap::new();
    for (id, parent) in [
        ("root", None),
        ("a", Some("root")),
        ("b", Some("a")),
        ("c", Some("b")),
    ] {
        members.insert(id.to_string(), member_with_parent(id, parent));
    }

    assert_eq!(swarm_spawn_depth(&members, "root"), 0);
    assert_eq!(swarm_spawn_depth(&members, "a"), 1);
    assert_eq!(swarm_spawn_depth(&members, "c"), 3);
    assert_eq!(swarm_ancestors(&members, "c"), vec!["b", "a", "root"]);

    // Ownership: an ancestor (or self) owns the subtree.
    assert!(swarm_is_self_or_ancestor(&members, "a", "c"));
    assert!(swarm_is_self_or_ancestor(&members, "root", "c"));
    assert!(swarm_is_self_or_ancestor(&members, "c", "c"));
    // A sibling/descendant is not an ancestor.
    assert!(!swarm_is_self_or_ancestor(&members, "c", "a"));
    assert!(!swarm_is_self_or_ancestor(&members, "b", "a"));
}

#[test]
fn swarm_ancestry_guards_against_cycles() {
    let mut members: HashMap<String, SwarmMember> = HashMap::new();
    // x -> y -> x is a (pathological) cycle; depth must terminate.
    members.insert("x".to_string(), member_with_parent("x", Some("y")));
    members.insert("y".to_string(), member_with_parent("y", Some("x")));
    assert_eq!(swarm_spawn_depth(&members, "x"), 1);
    assert_eq!(swarm_ancestors(&members, "x"), vec!["y"]);
}

#[tokio::test]
async fn remove_session_from_swarm_reassigns_to_non_headless_member() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from([
            "coord".to_string(),
            "headless".to_string(),
            "worker".to_string(),
        ]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "coord".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![PlanItem {
                content: "task".to_string(),
                status: "pending".to_string(),
                priority: "medium".to_string(),
                id: "1".to_string(),
                subsystem: None,
                file_scope: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: Some("coord".to_string()),
            }],
            version: 1,
            participants: HashSet::from(["coord".to_string()]),
            task_progress: HashMap::new(),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])));

    let (coord, _coord_rx) = swarm_member("coord", "coordinator", false);
    let (headless, mut headless_rx) = swarm_member("headless", "agent", true);
    let (worker, mut worker_rx) = swarm_member("worker", "agent", false);
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("headless".to_string(), headless);
        members.insert("worker".to_string(), worker);
        members.remove("coord");
    }

    remove_session_from_swarm(
        "coord",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
    )
    .await;

    assert_eq!(
        swarm_coordinators
            .read()
            .await
            .get("swarm-1")
            .map(String::as_str),
        Some("worker")
    );
    assert!(
        swarm_plans
            .read()
            .await
            .get("swarm-1")
            .is_some_and(|plan| plan.participants.contains("worker"))
    );
    assert_eq!(
        swarm_members
            .read()
            .await
            .get("worker")
            .map(|member| member.role.as_str()),
        Some("coordinator")
    );
    assert_eq!(
        swarm_members
            .read()
            .await
            .get("headless")
            .map(|member| member.role.as_str()),
        Some("agent")
    );

    let headless_events: Vec<_> = std::iter::from_fn(|| headless_rx.try_recv().ok()).collect();
    assert!(headless_events.iter().all(|event| {
        !matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message == "You are now the coordinator for this swarm."
        )
    }));

    let worker_events: Vec<_> = std::iter::from_fn(|| worker_rx.try_recv().ok()).collect();
    assert!(worker_events.iter().any(|event| {
        matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message == "You are now the coordinator for this swarm."
        )
    }));
}

#[tokio::test]
async fn remove_session_reparents_children_to_live_grandparent() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["root".to_string(), "mid".to_string(), "leaf".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "root".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::new()));

    let (root, _root_rx) = swarm_member("root", "coordinator", false);
    let (mut mid, _mid_rx) = swarm_member("mid", "agent", true);
    mid.report_back_to_session_id = Some("root".to_string());
    let (mut leaf, _leaf_rx) = swarm_member("leaf", "agent", true);
    leaf.report_back_to_session_id = Some("mid".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("root".to_string(), root);
        members.insert("mid".to_string(), mid);
        members.insert("leaf".to_string(), leaf);
    }

    remove_session_from_swarm(
        "mid",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
    )
    .await;

    // Leaf follows the report-back chain up to its grandparent instead of
    // dangling on the removed session.
    let members = swarm_members.read().await;
    assert_eq!(
        members
            .get("leaf")
            .and_then(|member| member.report_back_to_session_id.as_deref()),
        Some("root")
    );
    assert!(swarm_is_self_or_ancestor(&members, "root", "leaf"));
}

#[tokio::test]
async fn remove_session_reparents_children_to_coordinator_when_no_grandparent() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from([
            "coord".to_string(),
            "peer_root".to_string(),
            "child".to_string(),
        ]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "coord".to_string(),
    )])));
    let swarm_plans = Arc::new(RwLock::new(HashMap::new()));

    // peer_root is itself a root (no parent), so its children have no
    // grandparent to inherit; they should fall back to the coordinator.
    let (coord, _coord_rx) = swarm_member("coord", "coordinator", false);
    let (peer_root, _peer_rx) = swarm_member("peer_root", "agent", false);
    let (mut child, _child_rx) = swarm_member("child", "agent", true);
    child.report_back_to_session_id = Some("peer_root".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("peer_root".to_string(), peer_root);
        members.insert("child".to_string(), child);
    }

    remove_session_from_swarm(
        "peer_root",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
    )
    .await;

    let members = swarm_members.read().await;
    assert_eq!(
        members
            .get("child")
            .and_then(|member| member.report_back_to_session_id.as_deref()),
        Some("coord")
    );
}

#[tokio::test]
async fn update_member_status_notifies_coordinator_when_headless_worker_returns_ready() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["coord".to_string(), "worker".to_string()]),
    )])));

    let (coord, mut coord_rx) = swarm_member("coord", "coordinator", false);
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.status = "running".to_string();
    worker.detail = Some("doing task".to_string());
    worker.report_back_to_session_id = Some("coord".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("worker".to_string(), worker);
    }

    update_member_status(
        "worker",
        "ready",
        None,
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    let events: Vec<_> = std::iter::from_fn(|| coord_rx.try_recv().ok()).collect();
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message.contains("finished their work and is ready for more")
        )
    }));
}

#[tokio::test]
async fn member_elapsed_time_runs_only_while_active_and_freezes_afterward() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.runtime.elapsed_secs = Some(12);
    swarm_members
        .write()
        .await
        .insert("worker".to_string(), worker);

    update_member_status(
        "worker",
        "running",
        None,
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        swarm_members
            .read()
            .await
            .get("worker")
            .and_then(|member| member.runtime.elapsed_secs),
        None,
        "active members should derive elapsed time from joined_at"
    );

    {
        let mut members = swarm_members.write().await;
        members.get_mut("worker").unwrap().joined_at = Instant::now() - Duration::from_secs(37);
    }
    update_member_status(
        "worker",
        "completed",
        None,
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    let frozen = swarm_members
        .read()
        .await
        .get("worker")
        .and_then(|member| member.runtime.elapsed_secs)
        .expect("terminal member should retain frozen elapsed time");
    assert!((37..=38).contains(&frozen), "frozen={frozen}");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        swarm_members
            .read()
            .await
            .get("worker")
            .and_then(|member| member.runtime.elapsed_secs),
        Some(frozen)
    );
}

#[tokio::test]
async fn update_member_status_prefers_explicit_report_back_owner_over_coordinator() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from([
            "coord".to_string(),
            "owner".to_string(),
            "worker".to_string(),
        ]),
    )])));

    let (coord, mut coord_rx) = swarm_member("coord", "coordinator", false);
    let (owner, mut owner_rx) = swarm_member("owner", "agent", false);
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.status = "running".to_string();
    worker.detail = Some("doing task".to_string());
    worker.report_back_to_session_id = Some("owner".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("owner".to_string(), owner);
        members.insert("worker".to_string(), worker);
    }

    update_member_status(
        "worker",
        "ready",
        None,
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    let owner_events: Vec<_> = std::iter::from_fn(|| owner_rx.try_recv().ok()).collect();
    assert!(owner_events.iter().any(|event| {
        matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message.contains("finished their work and is ready for more")
        )
    }));
    let coord_events: Vec<_> = std::iter::from_fn(|| coord_rx.try_recv().ok()).collect();
    assert!(coord_events.iter().all(|event| {
        !matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message.contains("finished their work and is ready for more")
        )
    }));
}

#[tokio::test]
async fn update_member_status_includes_completion_report_in_owner_notification() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["coord".to_string(), "worker".to_string()]),
    )])));

    let (coord, mut coord_rx) = swarm_member("coord", "coordinator", false);
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.status = "running".to_string();
    worker.report_back_to_session_id = Some("coord".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("worker".to_string(), worker);
    }

    update_member_status_with_report(
        "worker",
        "ready",
        None,
        Some("Validated the parser and all tests passed.".to_string()),
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    let events: Vec<_> = std::iter::from_fn(|| coord_rx.try_recv().ok()).collect();
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ServerEvent::Notification {
                notification_type: NotificationType::Message { .. },
                message,
                ..
            } if message.contains("Report:\nValidated the parser")
                && !message.contains("No final textual report")
        )
    }));
}

#[tokio::test]
async fn update_member_status_skips_noop_broadcasts() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));

    let (worker, mut worker_rx) = swarm_member("worker", "agent", false);
    swarm_members
        .write()
        .await
        .insert("worker".to_string(), worker);

    update_member_status(
        "worker",
        "ready",
        None,
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    assert!(worker_rx.try_recv().is_err());

    update_member_status(
        "worker",
        "busy",
        Some("working".to_string()),
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    assert!(matches!(
        worker_rx.try_recv(),
        Ok(ServerEvent::SwarmStatus { members }) if members.len() == 1
            && members[0].session_id == "worker"
            && members[0].status == "busy"
            && members[0].detail.as_deref() == Some("working")
    ));
}

#[tokio::test]
async fn refresh_swarm_task_staleness_marks_running_tasks_stale_and_heartbeat_revives() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::new()));
    let now_ms = now_unix_ms();
    let stale_age_ms = super::swarm_task_stale_after().as_millis() as u64 + 5_000;
    let swarm_plans = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![PlanItem {
                content: "task".to_string(),
                status: "running".to_string(),
                priority: "medium".to_string(),
                id: "task-1".to_string(),
                subsystem: None,
                file_scope: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: Some("worker".to_string()),
            }],
            version: 1,
            participants: HashSet::from(["worker".to_string()]),
            task_progress: HashMap::from([(
                "task-1".to_string(),
                crate::server::SwarmTaskProgress {
                    assigned_session_id: Some("worker".to_string()),
                    started_at_unix_ms: Some(now_ms.saturating_sub(stale_age_ms)),
                    last_heartbeat_unix_ms: Some(now_ms.saturating_sub(stale_age_ms)),
                    ..Default::default()
                },
            )]),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])));
    let (worker, _worker_rx) = swarm_member("worker", "agent", true);
    swarm_members
        .write()
        .await
        .insert("worker".to_string(), worker);

    refresh_swarm_task_staleness(
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;

    {
        let plans = swarm_plans.read().await;
        let plan = plans.get("swarm-1").expect("plan");
        assert_eq!(plan.items[0].status, "running_stale");
        assert!(
            plan.task_progress
                .get("task-1")
                .and_then(|progress| progress.stale_since_unix_ms)
                .is_some()
        );
    }

    let revived = touch_swarm_task_progress(
        "swarm-1",
        "task-1",
        Some("worker"),
        Some("still working".to_string()),
        Some("checkpoint saved".to_string()),
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;
    assert!(revived);

    let plans = swarm_plans.read().await;
    let plan = plans.get("swarm-1").expect("plan");
    assert_eq!(plan.items[0].status, "running");
    let progress = plan.task_progress.get("task-1").expect("progress");
    assert_eq!(
        progress.checkpoint_summary.as_deref(),
        Some("checkpoint saved")
    );
    assert!(progress.stale_since_unix_ms.is_none());
}

#[test]
fn member_status_is_dead_matches_terminal_non_success_states() {
    for status in ["failed", "stopped", "crashed"] {
        assert!(member_status_is_dead(status), "{status} should be dead");
    }
    for status in ["ready", "running", "running_stale", "queued", "completed"] {
        assert!(!member_status_is_dead(status), "{status} should be alive");
    }
}

fn running_plan_assigned_to(
    assignee: &str,
    reclaims: Option<u32>,
) -> Arc<RwLock<HashMap<String, VersionedPlan>>> {
    Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        VersionedPlan {
            items: vec![PlanItem {
                content: "task".to_string(),
                status: "running".to_string(),
                priority: "medium".to_string(),
                id: "task-1".to_string(),
                subsystem: None,
                file_scope: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: Some(assignee.to_string()),
            }],
            version: 1,
            participants: HashSet::from([assignee.to_string()]),
            task_progress: HashMap::from([(
                "task-1".to_string(),
                crate::server::SwarmTaskProgress {
                    assigned_session_id: Some(assignee.to_string()),
                    dead_assignee_reclaims: reclaims,
                    ..Default::default()
                },
            )]),
            mode: "light".to_string(),
            node_meta: HashMap::new(),
        },
    )])))
}

#[tokio::test]
async fn salvage_requeues_dead_members_tasks_and_notifies_coordinator() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["coord".to_string(), "worker".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "coord".to_string(),
    )])));
    let swarm_plans = running_plan_assigned_to("worker", None);
    let (coord, mut coord_rx) = swarm_member("coord", "coordinator", false);
    let (worker, _worker_rx) = swarm_member("worker", "agent", true);
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("worker".to_string(), worker);
    }

    let outcome = salvage_assignments_of_dead_member(
        "worker",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;

    assert_eq!(outcome.requeued_task_ids, vec!["task-1".to_string()]);
    assert!(outcome.failed_task_ids.is_empty());
    {
        let plans = swarm_plans.read().await;
        let plan = plans.get("swarm-1").expect("plan");
        assert_eq!(plan.items[0].status, "queued");
        assert_eq!(plan.items[0].assigned_to, None);
        let progress = plan.task_progress.get("task-1").expect("progress");
        assert_eq!(progress.assigned_session_id, None);
        assert_eq!(progress.dead_assignee_reclaims, Some(1));
    }

    let coord_events: Vec<_> = std::iter::from_fn(|| coord_rx.try_recv().ok()).collect();
    assert!(
        coord_events.iter().any(|event| matches!(
            event,
            ServerEvent::Notification { message, .. }
                if message.contains("died") && message.contains("task-1")
        )),
        "coordinator should be told about the salvage, got {coord_events:?}"
    );
}

#[tokio::test]
async fn salvage_fails_task_once_reclaim_cap_is_reached() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::new()));
    let swarm_plans =
        running_plan_assigned_to("worker", Some(crate::plan::MAX_DEAD_ASSIGNEE_RECLAIMS));
    let (worker, _worker_rx) = swarm_member("worker", "agent", true);
    swarm_members
        .write()
        .await
        .insert("worker".to_string(), worker);

    let outcome = salvage_assignments_of_dead_member(
        "worker",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;

    assert!(outcome.requeued_task_ids.is_empty());
    assert_eq!(outcome.failed_task_ids, vec!["task-1".to_string()]);
    let plans = swarm_plans.read().await;
    let plan = plans.get("swarm-1").expect("plan");
    assert_eq!(plan.items[0].status, "failed");
    assert_eq!(plan.items[0].assigned_to, None);
}

#[tokio::test]
async fn remove_session_from_swarm_salvages_running_assignments() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["coord".to_string(), "worker".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "coord".to_string(),
    )])));
    let swarm_plans = running_plan_assigned_to("worker", None);
    let (coord, _coord_rx) = swarm_member("coord", "coordinator", false);
    let (worker, _worker_rx) = swarm_member("worker", "agent", true);
    {
        let mut members = swarm_members.write().await;
        members.insert("coord".to_string(), coord);
        members.insert("worker".to_string(), worker);
    }

    remove_session_from_swarm(
        "worker",
        "swarm-1",
        &swarm_members,
        &swarms_by_id,
        &swarm_coordinators,
        &swarm_plans,
    )
    .await;

    let plans = swarm_plans.read().await;
    let plan = plans.get("swarm-1").expect("plan");
    assert_eq!(plan.items[0].status, "queued");
    assert_eq!(plan.items[0].assigned_to, None);
}

#[tokio::test]
async fn staleness_sweep_salvages_tasks_of_vanished_assignee() {
    // The assignee is not a swarm member at all (zombie left over from a
    // previous process): no grace period applies and the sweep must
    // requeue its running task.
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["coord".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        "coord".to_string(),
    )])));
    let swarm_plans = running_plan_assigned_to("ghost", None);
    // Give the task a fresh heartbeat so the first sweep phase does not
    // interfere; the salvage phase must still fire on the dead assignee.
    {
        let mut plans = swarm_plans.write().await;
        let plan = plans.get_mut("swarm-1").expect("plan");
        let progress = plan.task_progress.get_mut("task-1").expect("progress");
        progress.last_heartbeat_unix_ms = Some(now_unix_ms());
    }
    let (coord, _coord_rx) = swarm_member("coord", "coordinator", false);
    swarm_members
        .write()
        .await
        .insert("coord".to_string(), coord);

    refresh_swarm_task_staleness(
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;

    let plans = swarm_plans.read().await;
    let plan = plans.get("swarm-1").expect("plan");
    assert_eq!(plan.items[0].status, "queued");
    assert_eq!(plan.items[0].assigned_to, None);
}

#[tokio::test]
async fn staleness_sweep_grants_grace_to_recently_crashed_member() {
    // A member marked crashed moments ago may be mid reload-recovery; the
    // sweep must not reclaim its work inside the grace window.
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["worker".to_string()]),
    )])));
    let swarm_coordinators = Arc::new(RwLock::new(HashMap::new()));
    let swarm_plans = running_plan_assigned_to("worker", None);
    {
        let mut plans = swarm_plans.write().await;
        let plan = plans.get_mut("swarm-1").expect("plan");
        let progress = plan.task_progress.get_mut("task-1").expect("progress");
        progress.last_heartbeat_unix_ms = Some(now_unix_ms());
    }
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.status = "crashed".to_string();
    worker.last_status_change = Instant::now();
    swarm_members
        .write()
        .await
        .insert("worker".to_string(), worker);

    refresh_swarm_task_staleness(
        &swarm_members,
        &swarms_by_id,
        &swarm_plans,
        &swarm_coordinators,
    )
    .await;

    let plans = swarm_plans.read().await;
    let plan = plans.get("swarm-1").expect("plan");
    assert_eq!(plan.items[0].status, "running");
    assert_eq!(plan.items[0].assigned_to.as_deref(), Some("worker"));
}

#[tokio::test]
async fn update_member_status_notifies_owner_when_worker_crashes_mid_task() {
    let swarm_members = Arc::new(RwLock::new(HashMap::new()));
    let swarms_by_id = Arc::new(RwLock::new(HashMap::from([(
        "swarm-1".to_string(),
        HashSet::from(["owner".to_string(), "worker".to_string()]),
    )])));
    let (owner, mut owner_rx) = swarm_member("owner", "coordinator", false);
    let (mut worker, _worker_rx) = swarm_member("worker", "agent", true);
    worker.status = "running".to_string();
    worker.report_back_to_session_id = Some("owner".to_string());
    {
        let mut members = swarm_members.write().await;
        members.insert("owner".to_string(), owner);
        members.insert("worker".to_string(), worker);
    }

    update_member_status(
        "worker",
        "crashed",
        Some("client disconnected while processing".to_string()),
        &swarm_members,
        &swarms_by_id,
        None,
        None,
        None,
    )
    .await;

    let owner_events: Vec<_> = std::iter::from_fn(|| owner_rx.try_recv().ok()).collect();
    assert!(
        owner_events.iter().any(|event| matches!(
            event,
            ServerEvent::Notification { message, .. }
                if message.contains("crashed while working")
        )),
        "owner should be notified of the crash, got {owner_events:?}"
    );
}

include!("swarm_tests/broadcasts.rs");
