use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use super::*;
use crate::testing::FakeAdapter;
use crate::{DebugSetBreakpointRequest, DebugSourceBreakpoint, Message};

#[tokio::test]
async fn manager_drop_closes_transport_with_detached_operation_and_releases_core() {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-lifecycle-drop-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    std::fs::write(&program, b"x").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_secs(5),
            ..Default::default()
        },
    )
    .unwrap();
    let weak_core = Arc::downgrade(&manager.core);
    let (client, mut adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: "owner".into(),
            workspace: DebugWorkspaceKey::new(&root, "drop").unwrap(),
            adapter_id: "fake".into(),
            start: Some(DebugSessionStart::Launch {
                program,
                cwd: root.clone(),
            }),
        })
        .unwrap();
    reservation.attach_client(client).unwrap();
    reservation.mark_configuring().unwrap();
    reservation.mark_running().unwrap();
    let id = reservation.commit().unwrap();
    let entry = manager.core.authorized_entry("owner", id).unwrap();
    let validation_gate = Arc::clone(&entry.breakpoint_validation)
        .acquire_owned()
        .await
        .unwrap();
    let caller = {
        let manager = manager.clone();
        let program = root.join("main");
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    id,
                    DebugSetBreakpointRequest::new(program, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    let request = match timeout(Duration::from_secs(1), adapter.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Message::Request(request) => request,
        other => panic!("expected continue request, got {other:?}"),
    };
    assert_eq!(request.command, "setBreakpoints");
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    adapter
        .respond_ok(
            &request,
            Some(serde_json::json!({"breakpoints":[{"id":1,"verified":true}]})),
        )
        .await
        .unwrap();
    entry
        .breakpoint_test_gates
        .response_validation_entered
        .notified()
        .await;
    adapter
        .event(
            "breakpoint",
            Some(serde_json::json!({
                "reason":"changed",
                "breakpoint":{"id":1,"verified":false,"message":"late"}
            })),
        )
        .await
        .unwrap();
    entry.breakpoint_test_gates.event_queued.notified().await;
    // The response and event are now inside the detached operation's pipeline. If it does not
    // recheck the terminal fence after its await, releasing the validation gate can commit them.
    let state_before_drop = entry.snapshot().unwrap();
    assert!(breakpoints::breakpoint_snapshot(&entry).sources.is_empty());
    drop(manager);
    timeout(Duration::from_secs(1), async {
        while weak_core.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(weak_core.upgrade().is_none());
    drop(validation_gate);
    timeout(Duration::from_secs(1), async {
        while Arc::strong_count(&entry) > 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(
        state_before_drop.state.kind(),
        DebugSessionStateKind::Running
    );
    assert_eq!(entry.snapshot().unwrap(), state_before_drop);
    assert!(breakpoints::breakpoint_snapshot(&entry).sources.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
