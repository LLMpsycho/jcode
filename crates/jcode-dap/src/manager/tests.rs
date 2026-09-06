use std::time::Duration;

use serde_json::json;
use tokio::time::{sleep, timeout};

use super::*;
use crate::testing::FakeAdapter;
use crate::{
    AdapterCommand, AdapterProcess, Capabilities, DapError, DebugSessionStateKind, Event, Message,
    StoppedState, encode_message,
};

fn manager() -> DebugSessionManager {
    DebugSessionManager::new(DebugSessionManagerConfig {
        termination_grace: Duration::from_millis(10),
        process_poll_interval: Duration::from_millis(5),
        ..Default::default()
    })
    .unwrap()
}

fn spec(owner: &str) -> NewDebugSession {
    NewDebugSession {
        owner_session_id: owner.into(),
        workspace: DebugWorkspaceKey::new(std::path::Path::new("."), owner).unwrap(),
        adapter_id: "fake".into(),
        start: Some(DebugSessionStart::Launch {
            program: std::env::current_dir().unwrap(),
            cwd: std::env::current_dir().unwrap(),
        }),
    }
}

fn attached(manager: &DebugSessionManager, owner: &str) -> (DebugSessionId, FakeAdapter) {
    let (client, adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager.reserve(spec(owner)).unwrap();
    reservation.attach_client(client).unwrap();
    let id = reservation.commit().unwrap();
    (id, adapter)
}

async fn wait_for(
    manager: &DebugSessionManager,
    owner: &str,
    id: DebugSessionId,
    predicate: impl Fn(&DebugSessionState) -> bool,
) -> DebugSessionSnapshot {
    timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(snapshot) = manager.snapshot(owner, id)
                && predicate(&snapshot.state)
            {
                return snapshot;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap()
}

#[test]
fn ids_capacity_and_one_active_owner_are_atomic() {
    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        max_active_sessions: 2,
        ..Default::default()
    })
    .unwrap();
    let first = manager.reserve(spec("a")).unwrap();
    let first_id = first.id();
    assert!(matches!(
        manager.reserve(spec("a")),
        Err(DapError::OwnerAlreadyHasActiveSession { .. })
    ));
    let second = manager.reserve(spec("b")).unwrap();
    assert_ne!(first_id, second.id());
    assert!(matches!(
        manager.reserve(spec("c")),
        Err(DapError::SessionCapacityExceeded { limit: 2 })
    ));
}

#[tokio::test]
async fn reservation_drop_releases_owner_slot_and_closes_transport() {
    let manager = manager();
    let (client, mut adapter) = FakeAdapter::pair(1024);
    let mut first = manager.reserve(spec("a")).unwrap();
    let first_id = first.id();
    first.attach_client(client).unwrap();
    drop(first);
    assert!(matches!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap(),
        Err(DapError::TransportClosed)
    ));
    let second = manager.reserve(spec("a")).unwrap();
    assert_ne!(first_id, second.id());
}

#[test]
fn legal_and_illegal_transitions_are_structured() {
    let manager = manager();
    let reservation = manager.reserve(spec("a")).unwrap();
    assert!(matches!(
        reservation.mark_running(),
        Err(DapError::InvalidSessionTransition {
            state: DebugSessionStateKind::Reserved,
            ..
        })
    ));
    drop(reservation);
}

#[test]
fn extreme_config_durations_are_rejected() {
    assert!(matches!(
        DebugSessionManager::new(DebugSessionManagerConfig {
            termination_grace: Duration::MAX,
            ..Default::default()
        }),
        Err(DapError::InvalidManagerConfiguration { .. })
    ));
}

#[tokio::test]
async fn explicit_lifecycle_methods_follow_legal_flow() {
    let manager = manager();
    let (client, _adapter) = FakeAdapter::pair(1024);
    let mut reservation = manager.reserve(spec("a")).unwrap();
    reservation.attach_client(client).unwrap();
    reservation
        .set_capabilities(Capabilities {
            supports_cancel_request: Some(true),
            ..Default::default()
        })
        .unwrap();
    reservation.mark_configuring().unwrap();
    reservation.mark_running().unwrap();
    let id = reservation.commit().unwrap();
    assert_eq!(
        manager.snapshot("a", id).unwrap().state,
        DebugSessionState::Running
    );
}

#[tokio::test]
async fn lifecycle_methods_are_idempotent_after_early_adapter_events() {
    let manager = manager();
    let (client, mut adapter) = FakeAdapter::pair(1024);
    let mut reservation = manager.reserve(spec("a")).unwrap();
    reservation.attach_client(client).unwrap();
    let id = reservation.id();

    adapter.event("initialized", None).await.unwrap();
    wait_for(&manager, "a", id, |state| {
        matches!(state, DebugSessionState::Configuring)
    })
    .await;
    reservation.mark_configuring().unwrap();

    adapter.event("continued", None).await.unwrap();
    wait_for(&manager, "a", id, |state| {
        matches!(state, DebugSessionState::Running)
    })
    .await;
    reservation.mark_running().unwrap();
}

#[tokio::test]
async fn attaching_an_already_closed_client_cannot_strand_initializing() {
    let manager = manager();
    let (client, _adapter) = FakeAdapter::pair(1024);
    client.close();
    let mut reservation = manager.reserve(spec("a")).unwrap();
    reservation.attach_client(client).unwrap();
    let id = reservation.commit().unwrap();
    let snapshot = wait_for(&manager, "a", id, DebugSessionState::is_terminal).await;
    assert!(matches!(snapshot.state, DebugSessionState::Ended(_)));
}

#[tokio::test]
async fn wrong_owner_cannot_get_list_request_output_or_terminate() {
    let manager = manager();
    let (id, mut adapter) = attached(&manager, "a");
    assert!(manager.sessions("b").is_empty());
    assert!(matches!(
        manager.snapshot("b", id),
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(matches!(
        manager.output("b", id, None, 1),
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(matches!(
        manager
            .request("b", id, "threads", None, Duration::from_millis(10))
            .await,
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(matches!(
        manager.terminate("b", id).await,
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(
        timeout(Duration::from_millis(20), adapter.recv())
            .await
            .is_err()
    );
    assert_eq!(
        manager.snapshot("a", id).unwrap().state,
        DebugSessionState::Initializing
    );
}

#[tokio::test]
async fn supervisor_handles_stopped_continued_exited_and_stale_events() {
    let manager = manager();
    let (id, mut adapter) = attached(&manager, "a");
    adapter.event("initialized", None).await.unwrap();
    adapter
        .event("stopped", Some(json!({"reason":"breakpoint","threadId":9})))
        .await
        .unwrap();
    let stopped = wait_for(&manager, "a", id, |state| {
        matches!(state, DebugSessionState::Stopped(_))
    })
    .await;
    assert!(matches!(
        stopped.state,
        DebugSessionState::Stopped(StoppedState {
            thread_id: Some(9),
            ..
        })
    ));
    adapter
        .event("continued", Some(json!({"threadId":9})))
        .await
        .unwrap();
    wait_for(&manager, "a", id, |state| {
        matches!(state, DebugSessionState::Running)
    })
    .await;
    adapter
        .event("exited", Some(json!({"exitCode":3})))
        .await
        .unwrap();
    let ended = wait_for(&manager, "a", id, DebugSessionState::is_terminal).await;
    assert!(matches!(
        ended.state,
        DebugSessionState::Ended(DebugSessionEnd {
            reason: DebugSessionEndReason::DebuggeeExited { exit_code: Some(3) },
            ..
        })
    ));
    let _ = adapter.event("continued", None).await;
    sleep(Duration::from_millis(10)).await;
    assert!(manager.snapshot("a", id).unwrap().state.is_terminal());
}

#[tokio::test]
async fn output_bounds_paging_and_utf8_are_enforced() {
    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        output_max_events: 2,
        output_max_bytes: 5,
        output_page_limit: 1,
        ..Default::default()
    })
    .unwrap();
    let (id, mut adapter) = attached(&manager, "a");
    for value in ["a", "bb", "é😊"] {
        adapter
            .event("output", Some(json!({"output":value})))
            .await
            .unwrap();
    }
    timeout(Duration::from_secs(1), async {
        loop {
            if manager.output("a", id, None, 9).unwrap().status.next_cursor == DebugOutputCursor(4)
            {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let page = manager
        .output("a", id, Some(DebugOutputCursor(1)), 9)
        .unwrap();
    assert_eq!(page.records.len(), 1);
    assert!(page.requested_history_was_evicted);
    assert!(page.status.retained_bytes <= 5);
    assert!(page.status.evicted_events > 0);
}

#[tokio::test]
async fn oversized_output_is_counted_and_non_output_loss_is_terminal() {
    let manager = manager();
    let (id, mut adapter) = attached(&manager, "a");
    adapter
        .event(
            "output",
            Some(json!({"output":"x".repeat(crate::MAX_RETAINED_EVENT_SIZE)})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        while manager
            .output("a", id, None, 1)
            .unwrap()
            .status
            .source_dropped_events
            == 0
        {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        manager
            .output("a", id, None, 1)
            .unwrap()
            .status
            .retained_events,
        0
    );

    adapter
        .event(
            "stopped",
            Some(json!({"reason":"x".repeat(crate::MAX_RETAINED_EVENT_SIZE)})),
        )
        .await
        .unwrap();
    let ended = wait_for(&manager, "a", id, DebugSessionState::is_terminal).await;
    assert!(matches!(
        ended.state,
        DebugSessionState::Ended(DebugSessionEnd {
            reason: DebugSessionEndReason::ProtocolError { .. },
            ..
        })
    ));
}

#[tokio::test]
async fn transport_close_ends_session_without_pending_request() {
    let manager = manager();
    let (id, adapter) = attached(&manager, "a");
    drop(adapter);
    let ended = wait_for(&manager, "a", id, DebugSessionState::is_terminal).await;
    assert!(matches!(
        ended.state,
        DebugSessionState::Ended(DebugSessionEnd {
            reason: DebugSessionEndReason::TransportClosed,
            ..
        })
    ));
}

#[tokio::test]
async fn broadcast_lag_ends_session_with_exact_loss() {
    let manager = manager();
    let (id, mut adapter) = attached(&manager, "a");
    let mut frames = Vec::new();
    for seq in 1..=(crate::EVENT_CHANNEL_CAPACITY as i64 + 1) {
        frames.extend(encode_message(&Event::new(seq, "custom", None)).unwrap());
    }
    adapter.send_raw(&frames).await.unwrap();
    let snapshot = wait_for(&manager, "a", id, DebugSessionState::is_terminal).await;
    assert!(matches!(
        snapshot.state,
        DebugSessionState::Ended(DebugSessionEnd {
            reason: DebugSessionEndReason::EventStreamLagged { skipped: 1 },
            ..
        })
    ));
}

#[tokio::test]
async fn request_timeout_is_recoverable() {
    let manager = manager();
    let (client, mut adapter) = FakeAdapter::pair(1024);
    let mut reservation = manager.reserve(spec("a")).unwrap();
    reservation.attach_client(client).unwrap();
    reservation
        .set_capabilities(Capabilities {
            supports_cancel_request: Some(true),
            ..Default::default()
        })
        .unwrap();
    let id = reservation.commit().unwrap();

    let result = manager
        .request("a", id, "threads", None, Duration::from_millis(10))
        .await;
    assert!(matches!(result, Err(DapError::RequestTimeout { .. })));
    assert!(!manager.snapshot("a", id).unwrap().state.is_terminal());
    let timed_out = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    let cancel = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    assert_eq!(cancel.command, "cancel");
    assert_eq!(cancel.arguments, Some(json!({"requestId": timed_out.seq})));

    let request_task = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .request("a", id, "threads", None, Duration::from_secs(1))
                .await
        })
    };
    let retry = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    adapter
        .respond_ok(&retry, Some(json!({"threads":[]})))
        .await
        .unwrap();
    assert!(request_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn terminate_releases_owner_and_retains_terminal_snapshot() {
    let manager = manager();
    let (id, _adapter) = attached(&manager, "a");
    let snapshot = manager.terminate("a", id).await.unwrap();
    assert!(snapshot.state.is_terminal());
    assert!(manager.reserve(spec("a")).is_ok());
    assert_eq!(manager.snapshot("a", id).unwrap().id, id);
}

#[tokio::test]
async fn concurrent_transport_failure_and_terminate_finalize_once() {
    let manager = manager();
    let (id, adapter) = attached(&manager, "a");
    let other = manager.clone();
    let terminate = tokio::spawn(async move { other.terminate("a", id).await });
    drop(adapter);
    let _ = terminate.await.unwrap();
    let snapshot = manager.snapshot("a", id).unwrap();
    assert!(snapshot.state.is_terminal());
    assert!(!matches!(snapshot.state, DebugSessionState::Terminating));
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_cleanup_callers_does_not_strand_sessions() {
    async fn attach_stubborn_process(manager: &DebugSessionManager, owner: &str) -> DebugSessionId {
        let process = AdapterProcess::spawn(
            &AdapterCommand::new(std::fs::canonicalize("/bin/sh").unwrap(), "/")
                .with_arg("-c")
                .with_arg("trap '' TERM; printf ready >&2; while :; do sleep 1; done"),
        )
        .await
        .unwrap();
        timeout(Duration::from_secs(1), async {
            while process.recent_stderr() != b"ready" {
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let mut reservation = manager.reserve(spec(owner)).unwrap();
        reservation
            .attach_transport(process.client().clone(), Some(process), None)
            .unwrap();
        reservation.commit().unwrap()
    }

    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        termination_grace: Duration::from_millis(100),
        process_poll_interval: Duration::from_millis(5),
        ..Default::default()
    })
    .unwrap();
    let id = attach_stubborn_process(&manager, "terminate").await;
    let terminating = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.terminate("terminate", id).await })
    };
    wait_for(&manager, "terminate", id, |state| {
        matches!(state, DebugSessionState::Terminating)
    })
    .await;
    terminating.abort();
    assert!(terminating.await.unwrap_err().is_cancelled());
    wait_for(&manager, "terminate", id, DebugSessionState::is_terminal).await;

    let id = attach_stubborn_process(&manager, "cleanup").await;
    let entry = manager.core.entry(id).unwrap();
    let cleanup = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .cleanup_owner("cleanup", OwnerCleanupCause::Disconnected)
                .await
        })
    };
    timeout(Duration::from_secs(2), async {
        while !matches!(
            entry.snapshot().unwrap().state,
            DebugSessionState::Terminating
        ) {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    cleanup.abort();
    assert!(cleanup.await.unwrap_err().is_cancelled());
    timeout(Duration::from_secs(2), async {
        while !entry.snapshot().unwrap().state.is_terminal() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(manager.reserve(spec("cleanup")).is_ok());

    let id = attach_stubborn_process(&manager, "shutdown").await;
    let entry = manager.core.entry(id).unwrap();
    let shutdown = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.shutdown_all().await })
    };
    timeout(Duration::from_secs(2), async {
        while !matches!(
            entry.snapshot().unwrap().state,
            DebugSessionState::Terminating
        ) {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    shutdown.abort();
    assert!(shutdown.await.unwrap_err().is_cancelled());
    timeout(Duration::from_secs(2), async {
        while !entry.snapshot().unwrap().state.is_terminal() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(manager.sessions("shutdown").is_empty());
}

#[tokio::test]
async fn terminating_keeps_the_owner_slot_until_ended() {
    let manager = manager();
    let (id, _adapter) = attached(&manager, "a");
    let entry = manager.core.entry(id).unwrap();
    let finalization = entry.finalization.lock().await;
    let terminating = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.terminate("a", id).await })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        manager.reserve(spec("a")),
        Err(DapError::OwnerAlreadyHasActiveSession { .. })
    ));
    drop(finalization);
    terminating.await.unwrap().unwrap();
    assert!(manager.reserve(spec("a")).is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn reservation_drop_waits_for_finalization_lock_before_cleanup_and_slot_release() {
    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        termination_grace: Duration::from_millis(20),
        process_poll_interval: Duration::from_millis(5),
        ..Default::default()
    })
    .unwrap();
    let process = AdapterProcess::spawn(
        &AdapterCommand::new(std::fs::canonicalize("/bin/sh").unwrap(), "/")
            .with_arg("-c")
            .with_arg("trap '' TERM; while :; do sleep 1; done"),
    )
    .await
    .unwrap();
    let observer = process.observer();
    let mut reservation = manager.reserve(spec("drop-race")).unwrap();
    let entry = manager.core.entry(reservation.id()).unwrap();
    reservation
        .attach_transport(process.client().clone(), Some(process), None)
        .unwrap();
    let finalization = entry.finalization.lock().await;
    drop(reservation);
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        manager.reserve(spec("drop-race")),
        Err(DapError::OwnerAlreadyHasActiveSession { .. })
    ));
    assert!(matches!(
        observer.status().await.unwrap(),
        Some(ProcessStatus::Running)
    ));
    drop(finalization);
    timeout(Duration::from_secs(2), async {
        loop {
            if manager.reserve(spec("drop-race")).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        observer.status().await.unwrap(),
        None | Some(ProcessStatus::Exited { .. })
    ));
}

#[test]
fn reservation_drop_without_runtime_waits_for_finalization_lock() {
    use std::sync::mpsc;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let manager = manager();
    let reservation = manager.reserve(spec("drop-no-runtime")).unwrap();
    let entry = manager.core.entry(reservation.id()).unwrap();
    let finalization = runtime.block_on(entry.finalization.lock());
    let (started_tx, started_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let dropper = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        drop(reservation);
        finished_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(matches!(
        manager.reserve(spec("drop-no-runtime")),
        Err(DapError::OwnerAlreadyHasActiveSession { .. })
    ));
    assert!(matches!(
        finished_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    drop(finalization);
    finished_rx.recv().unwrap();
    dropper.join().unwrap();
    assert!(manager.reserve(spec("drop-no-runtime")).is_ok());
}

#[tokio::test]
async fn cleanup_owner_and_shutdown_remove_visibility() {
    let manager = manager();
    let (_a, _adapter_a) = attached(&manager, "a");
    let (b, _adapter_b) = attached(&manager, "b");
    let report = manager
        .cleanup_owner("a", OwnerCleanupCause::Disconnected)
        .await;
    assert_eq!(report.cleaned, 1);
    assert!(manager.sessions("a").is_empty());
    assert_eq!(manager.snapshot("b", b).unwrap().id, b);
    let report = manager.shutdown_all().await;
    assert_eq!(report.cleaned, 1);
    assert!(manager.sessions("b").is_empty());
}

#[tokio::test]
async fn terminal_retention_prunes_oldest_and_repairs_indexes() {
    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        max_retained_ended_sessions: 1,
        ..Default::default()
    })
    .unwrap();
    let (first, _adapter) = attached(&manager, "a");
    manager.terminate("a", first).await.unwrap();
    let (second, _adapter) = attached(&manager, "a");
    manager.terminate("a", second).await.unwrap();
    assert!(matches!(
        manager.snapshot("a", first),
        Err(DapError::SessionNotFound { .. })
    ));
    assert_eq!(manager.sessions("a").len(), 1);
}

#[tokio::test]
async fn dropping_last_manager_closes_fake_transport() {
    let manager = manager();
    let (_id, mut adapter) = attached(&manager, "a");
    drop(manager);
    assert!(matches!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap(),
        Err(DapError::TransportClosed)
    ));
}

#[tokio::test]
async fn owner_authorized_request_round_trips() {
    let manager = manager();
    let (id, mut adapter) = attached(&manager, "a");
    let request_task = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .request("a", id, "threads", None, Duration::from_secs(1))
                .await
        })
    };
    let request = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    adapter
        .respond_ok(&request, Some(json!({"threads":[]})))
        .await
        .unwrap();
    assert!(request_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn reservation_drop_retains_cleanup_failure_for_owner_inspection() {
    let manager = manager();
    let (client, mut adapter) = FakeAdapter::pair(4096);
    let mut reservation = manager.reserve(spec("cleanup-failure")).unwrap();
    let id = reservation.id();
    reservation.attach_client(client).unwrap();
    let entry = manager.core.entry(id).unwrap();
    lock(&entry.data)
        .transport
        .as_mut()
        .unwrap()
        .termination_policy = Some(DebugTerminationPolicy::AdapterLaunched);

    drop(reservation);
    let Message::Request(disconnect) = adapter.recv().await.unwrap() else {
        panic!("expected disconnect request");
    };
    assert_eq!(disconnect.command, "disconnect");
    adapter
        .respond_error(&disconnect, "fixture cleanup rejected")
        .await
        .unwrap();

    let snapshot = wait_for(
        &manager,
        "cleanup-failure",
        id,
        DebugSessionState::is_terminal,
    )
    .await;
    let DebugSessionState::Ended(end) = snapshot.state else {
        panic!("expected ended session");
    };
    assert!(
        end.cleanup_error
            .unwrap()
            .contains("fixture cleanup rejected")
    );
    assert!(manager.reserve(spec("cleanup-failure")).is_ok());
    assert!(manager.snapshot("different-owner", id).is_err());
}

#[tokio::test]
async fn successful_reservation_drop_does_not_evict_retained_terminal_history() {
    let manager = DebugSessionManager::new(DebugSessionManagerConfig {
        max_retained_ended_sessions: 1,
        ..Default::default()
    })
    .unwrap();
    let (previous, _previous_adapter) = attached(&manager, "history-owner");
    manager.terminate("history-owner", previous).await.unwrap();
    let (client, mut adapter) = FakeAdapter::pair(1024);
    let mut reservation = manager.reserve(spec("history-owner")).unwrap();
    reservation.attach_client(client).unwrap();
    let canceled = reservation.id();
    drop(reservation);
    assert!(matches!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap(),
        Err(DapError::TransportClosed)
    ));
    timeout(Duration::from_secs(2), async {
        while manager.snapshot("history-owner", canceled).is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        manager
            .snapshot("history-owner", previous)
            .unwrap()
            .state
            .is_terminal()
    );
}
