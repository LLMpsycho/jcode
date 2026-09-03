use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use tokio::time::{sleep, timeout};

use super::*;
use crate::testing::FakeAdapter;
use crate::{DebugSessionManagerConfig, DebugSourceBreakpoint, Message, StoppedState};

struct Fixture {
    manager: DebugSessionManager,
    id: DebugSessionId,
    adapter: FakeAdapter,
    root: PathBuf,
    source: PathBuf,
}

fn fixture(owner: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-30e-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("hello world.rs");
    std::fs::write(&source, b"fn main() {\r\n println!(\"hi\");\r\n}\r\n").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .unwrap();
    let (client, adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: owner.to_owned(),
            workspace: DebugWorkspaceKey::new(&root, owner).unwrap(),
            adapter_id: "fake".to_owned(),
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
    Fixture {
        manager,
        id,
        adapter,
        root,
        source,
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn request(adapter: &mut FakeAdapter) -> crate::Request {
    match timeout(Duration::from_secs(2), adapter.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

#[test]
fn exact_30d_public_struct_literals_remain_source_compatible() {
    let _ = DebugSessionManager::new(DebugSessionManagerConfig {
        max_active_sessions: 64,
        max_retained_ended_sessions: 64,
        output_max_events: 1024,
        output_max_bytes: 1024 * 1024,
        output_page_limit: 256,
        termination_grace: Duration::from_secs(2),
        process_poll_interval: Duration::from_millis(250),
        startup_timeout: Duration::from_secs(30),
        disconnect_timeout: Duration::from_secs(2),
    })
    .unwrap();
    let _ = StoppedState {
        reason: "breakpoint".into(),
        description: None,
        thread_id: Some(1),
        all_threads_stopped: true,
    };
}

#[tokio::test]
async fn full_source_set_idempotence_remove_and_exact_revision() {
    let mut f = fixture("owner");
    let first_task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let first = request(&mut f.adapter).await;
    assert_eq!(first.command, "setBreakpoints");
    assert_eq!(
        first.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":1}])
    );
    f.adapter
        .respond_ok(
            &first,
            Some(json!({"breakpoints":[{"id":7,"verified":true,"line":1}]})),
        )
        .await
        .unwrap();
    let first_result = first_task.await.unwrap().unwrap();
    let first_id = match first_result.mutation {
        DebugBreakpointMutation::Created { breakpoint_id } => breakpoint_id,
        _ => panic!(),
    };
    assert_eq!(
        first_result.source.source_revision.byte_len,
        std::fs::metadata(&f.source).unwrap().len()
    );

    let duplicate = f
        .manager
        .set_breakpoint(
            "owner",
            f.id,
            DebugSetBreakpointRequest::new(&f.source, DebugSourceBreakpoint::new(1)),
        )
        .await
        .unwrap();
    assert!(
        matches!(duplicate.mutation,DebugBreakpointMutation::Existing{breakpoint_id} if breakpoint_id==first_id)
    );

    let second_task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(2)),
                )
                .await
        })
    };
    let second = request(&mut f.adapter).await;
    assert_eq!(
        second.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":1},{"line":2}])
    );
    f.adapter.respond_ok(&second,Some(json!({"breakpoints":[{"id":7,"verified":true},{"id":8,"verified":false,"reason":"pending"}]}))).await.unwrap();
    second_task.await.unwrap().unwrap();

    let remove_task = {
        let manager = f.manager.clone();
        tokio::spawn(async move {
            manager
                .remove_breakpoint("owner", f.id, DebugRemoveBreakpointRequest::new(first_id))
                .await
        })
    };
    let remove = request(&mut f.adapter).await;
    assert_eq!(
        remove.arguments.as_ref().unwrap()["breakpoints"],
        json!([{"line":2}])
    );
    f.adapter
        .respond_ok(
            &remove,
            Some(json!({"breakpoints":[{"id":8,"verified":true}]})),
        )
        .await
        .unwrap();
    remove_task.await.unwrap().unwrap();
    assert_eq!(
        f.manager
            .breakpoints("owner", f.id)
            .unwrap()
            .total_breakpoints,
        1
    );
}

#[tokio::test]
async fn id_only_higher_sequence_event_is_applied_after_response() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    f.adapter
        .respond_ok(
            &set,
            Some(json!({"breakpoints":[{"id":42,"verified":true}]})),
        )
        .await
        .unwrap();
    f.adapter.event("breakpoint",Some(json!({"reason":"changed","breakpoint":{"id":42,"verified":false,"reason":"pending"}}))).await.unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(!result.source.breakpoints[0].verified);
    assert_eq!(
        result.source.breakpoints[0].reason,
        Some(DebugBreakpointReason::Pending)
    );
}

#[tokio::test]
async fn wrong_owner_all_breakpoint_apis_emit_zero_traffic() {
    let mut f = fixture("owner");
    assert!(matches!(
        f.manager.breakpoints("wrong", f.id),
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(matches!(
        f.manager
            .set_breakpoint(
                "wrong",
                f.id,
                DebugSetBreakpointRequest::new(&f.source, DebugSourceBreakpoint::new(1))
            )
            .await,
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(matches!(
        f.manager
            .remove_breakpoint(
                "wrong",
                f.id,
                DebugRemoveBreakpointRequest::new(DebugBreakpointId(1))
            )
            .await,
        Err(DapError::SessionAccessDenied { .. })
    ));
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_change_triggers_compensating_empty_clear() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let set = request(&mut f.adapter).await;
    std::fs::write(&f.source, b"changed").unwrap();
    f.adapter
        .respond_ok(
            &set,
            Some(json!({"breakpoints":[{"id":1,"verified":true}]})),
        )
        .await
        .unwrap();
    let clear = request(&mut f.adapter).await;
    assert_eq!(clear.arguments.as_ref().unwrap()["breakpoints"], json!([]));
    f.adapter
        .respond_ok(&clear, Some(json!({"breakpoints":[]})))
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::DebugSourceChangedDuringOperation { .. })
    ));
    sleep(Duration::from_millis(1)).await;
}

#[tokio::test]
async fn ambiguous_timeout_with_unknown_queued_events_is_indeterminate() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        let source = f.source.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    f.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let _set = request(&mut f.adapter).await;
    f.adapter
        .event(
            "breakpoint",
            Some(json!({"reason":"changed","breakpoint":{"id":999,"verified":false}})),
        )
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    let snapshot = f.manager.breakpoints("owner", f.id).unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    assert_eq!(
        snapshot.sources[0].synchronization,
        DebugBreakpointSynchronization::Indeterminate
    );
}

#[tokio::test]
async fn queue_all_events_is_bounded_and_overflow_is_recorded() {
    let f = fixture("owner");
    let entry = f.manager.core.authorized_entry("owner", f.id).unwrap();
    let mut data = lock(&entry.data);
    data.breakpoints.in_flight = Some(BreakpointTransactionInFlight {
        source: f.source.clone(),
        bounded_events: VecDeque::new(),
        overflowed: false,
    });
    queue_breakpoint_event(&mut data, 1, json!({"unknown":"source"}), 1);
    queue_breakpoint_event(&mut data, 2, json!({"breakpoint":{"id":9}}), 1);
    let transaction = data.breakpoints.in_flight.as_ref().unwrap();
    assert_eq!(transaction.bounded_events.len(), 1);
    assert!(transaction.overflowed);
}
