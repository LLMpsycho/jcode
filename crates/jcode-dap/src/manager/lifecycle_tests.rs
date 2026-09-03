use std::sync::Arc;
use std::time::Duration;

use tokio::time::{sleep, timeout};

use super::*;
use crate::testing::FakeAdapter;
use crate::{DebugContinueRequest, Message, StoppedState};

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
    {
        let mut data = lock(&entry.data);
        data.state = DebugSessionState::Stopped(StoppedState {
            reason: "breakpoint".into(),
            thread_id: Some(7),
            all_threads_stopped: true,
            description: None,
        });
    }
    let caller = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .continue_execution("owner", id, DebugContinueRequest::default())
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
    assert_eq!(request.command, "continue");
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let state_before_drop = entry.snapshot().unwrap();
    drop(entry);
    drop(manager);
    timeout(Duration::from_secs(1), async {
        while weak_core.upgrade().is_some() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(weak_core.upgrade().is_none());
    assert!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(
        state_before_drop.state.kind(),
        DebugSessionStateKind::Stopped
    );
    let _ = std::fs::remove_dir_all(root);
}
