use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use tokio::time::{sleep, timeout};

use super::*;
use crate::testing::FakeAdapter;
use crate::{DebugBreakpointId, DebugSessionManagerConfig, Event, Message, StoppedState};

struct Fixture {
    manager: DebugSessionManager,
    id: DebugSessionId,
    adapter: FakeAdapter,
    root: PathBuf,
}
fn fixture(owner: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-control-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    std::fs::write(&program, b"x").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap();
    let (client, adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: owner.into(),
            workspace: DebugWorkspaceKey::new(&root, owner).unwrap(),
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
    Fixture {
        manager,
        id,
        adapter,
        root,
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn recv(adapter: &mut FakeAdapter) -> crate::Request {
    match timeout(Duration::from_secs(1), adapter.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Message::Request(r) => r,
        other => panic!("expected request: {other:?}"),
    }
}
async fn stop(f: &mut Fixture, thread: i64, all: bool) {
    f.adapter
        .event(
            "stopped",
            Some(json!({"reason":"breakpoint","threadId":thread,"allThreadsStopped":all})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                f.manager.snapshot("owner", f.id).unwrap().state,
                DebugSessionState::Stopped(_)
            ) {
                break;
            }
            sleep(Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn threads_are_ephemeral_bounded_and_preserve_order() {
    let mut f = fixture("owner");
    let task = {
        let manager = f.manager.clone();
        tokio::spawn(async move { manager.threads("owner", f.id).await })
    };
    let request = recv(&mut f.adapter).await;
    assert_eq!(request.command, "threads");
    assert!(request.arguments.is_none());
    f.adapter
        .respond_ok(
            &request,
            Some(json!({"threads":[{"id":2,"name":"worker"},{"id":1,"name":"main"}]})),
        )
        .await
        .unwrap();
    let snapshot = task.await.unwrap().unwrap();
    assert_eq!(
        snapshot
            .threads
            .iter()
            .map(|t| t.id.get())
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(snapshot.execution_revision.get(), 0);
}

#[tokio::test]
async fn continue_uses_stopped_thread_and_does_not_require_continued_event() {
    let mut f = fixture("owner");
    stop(&mut f, 7, true).await;
    let task = {
        let manager = f.manager.clone();
        tokio::spawn(async move {
            manager
                .continue_execution("owner", f.id, DebugContinueRequest::default())
                .await
        })
    };
    let request = recv(&mut f.adapter).await;
    assert_eq!(request.command, "continue");
    assert_eq!(request.arguments, Some(json!({"threadId":7})));
    f.adapter
        .respond_ok(&request, Some(json!({})))
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.all_threads_continued, Some(true));
    assert_eq!(result.state.kind(), DebugSessionStateKind::Running);
}

#[tokio::test]
async fn pause_and_steps_use_exact_commands_and_response_event_order() {
    let mut f = fixture("owner");
    let pause = {
        let manager = f.manager.clone();
        tokio::spawn(async move {
            manager
                .pause("owner", f.id, DebugPauseRequest::default())
                .await
        })
    };
    let threads = recv(&mut f.adapter).await;
    assert_eq!(threads.command, "threads");
    f.adapter
        .respond_ok(&threads, Some(json!({"threads":[{"id":3,"name":"main"}]})))
        .await
        .unwrap();
    let request = recv(&mut f.adapter).await;
    assert_eq!(request.command, "pause");
    f.adapter.respond_ok(&request, None).await.unwrap();
    assert_eq!(
        pause.await.unwrap().unwrap().state.kind(),
        DebugSessionStateKind::Running
    );
    f.adapter
        .event(
            "stopped",
            Some(json!({"reason":"pause","threadId":3,"allThreadsStopped":true})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                f.manager.snapshot("owner", f.id).unwrap().state,
                DebugSessionState::Stopped(_)
            ) {
                break;
            }
            sleep(Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
    for operation in [
        DebugControlOperation::StepOver,
        DebugControlOperation::StepIn,
        DebugControlOperation::StepOut,
    ] {
        let manager = f.manager.clone();
        let task = tokio::spawn(async move {
            match operation {
                DebugControlOperation::StepOver => {
                    manager
                        .step_over("owner", f.id, DebugStepRequest::default())
                        .await
                }
                DebugControlOperation::StepIn => {
                    manager
                        .step_in("owner", f.id, DebugStepRequest::default())
                        .await
                }
                _ => {
                    manager
                        .step_out("owner", f.id, DebugStepRequest::default())
                        .await
                }
            }
        });
        let request = recv(&mut f.adapter).await;
        assert_eq!(request.command, command(operation));
        f.adapter.respond_ok(&request, None).await.unwrap();
        assert_eq!(
            task.await.unwrap().unwrap().state.kind(),
            DebugSessionStateKind::Running
        );
        f.adapter
            .event(
                "stopped",
                Some(json!({"reason":"step","threadId":3,"allThreadsStopped":true})),
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn stale_revision_and_wrong_owner_emit_zero_control_traffic() {
    let mut f = fixture("owner");
    stop(&mut f, 1, true).await;
    let current = f.manager.threads("wrong", f.id).await;
    assert!(matches!(current, Err(DapError::SessionAccessDenied { .. })));
    for result in [
        f.manager
            .continue_execution("wrong", f.id, DebugContinueRequest::default())
            .await,
        f.manager
            .pause("wrong", f.id, DebugPauseRequest::default())
            .await,
        f.manager
            .step_over("wrong", f.id, DebugStepRequest::default())
            .await,
        f.manager
            .step_in("wrong", f.id, DebugStepRequest::default())
            .await,
        f.manager
            .step_out("wrong", f.id, DebugStepRequest::default())
            .await,
    ] {
        assert!(matches!(result, Err(DapError::SessionAccessDenied { .. })));
    }
    let stale = DebugContinueRequest {
        thread_id: Some(DebugThreadId::new(1)),
        expected_execution_revision: Some(DebugExecutionRevision(0)),
    };
    assert!(matches!(
        f.manager.continue_execution("owner", f.id, stale).await,
        Err(DapError::StaleExecutionRevision { .. })
    ));
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn continue_timeout_is_conservative_and_later_stop_recovers() {
    let mut f = fixture("owner");
    stop(&mut f, 1, true).await;
    let result = f
        .manager
        .continue_execution("owner", f.id, DebugContinueRequest::default())
        .await;
    assert!(matches!(result, Err(DapError::RequestTimeout { .. })));
    assert_eq!(
        f.manager.snapshot("owner", f.id).unwrap().state.kind(),
        DebugSessionStateKind::Running
    );
    let _timed_out = recv(&mut f.adapter).await;
    f.adapter
        .event(
            "stopped",
            Some(json!({"reason":"breakpoint","threadId":1,"allThreadsStopped":true})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if f.manager.snapshot("owner", f.id).unwrap().state.kind()
                == DebugSessionStateKind::Stopped
            {
                break;
            }
            sleep(Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
}

#[test]
fn public_id_accessors_and_formatting_are_stable() {
    assert_eq!(DebugThreadId::new(-7).get(), -7);
    assert_eq!(DebugThreadId::new(-7).to_string(), "-7");
    assert_eq!(DebugBreakpointId(9).get(), 9);
    assert_eq!(DebugBreakpointId(9).to_string(), "9");
    assert_eq!(DebugExecutionRevision(11).get(), 11);
    assert_eq!(DebugExecutionRevision(11).to_string(), "11");
    let _compat = StoppedState {
        reason: "step".into(),
        description: None,
        thread_id: None,
        all_threads_stopped: false,
    };
    let _ = Event::new(1, "custom", None);
}

#[tokio::test]
async fn final_manager_drop_closes_transport_despite_detached_operation() {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-drop-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    std::fs::write(&program, b"x").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .unwrap();
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
    let caller = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .pause("owner", id, DebugPauseRequest::default())
                .await
        })
    };
    let threads = recv(&mut adapter).await;
    assert_eq!(threads.command, "threads");
    caller.abort();
    let _ = caller.await;
    drop(manager);
    assert!(matches!(
        timeout(Duration::from_secs(1), adapter.recv())
            .await
            .unwrap(),
        Err(DapError::TransportClosed)
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn exhausted_control_revision_is_rejected_before_dispatch_without_closing_session() {
    let mut f = fixture("owner");
    stop(&mut f, 7, true).await;
    let entry = f.manager.core.entry(f.id).unwrap();
    {
        let mut data = lock(&entry.data);
        data.execution_revision = u64::MAX;
    }
    let error = f
        .manager
        .continue_execution("owner", f.id, DebugContinueRequest::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        DapError::ExecutionRevisionExhausted { session_id: f.id }
    );
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
    assert!(!entry.closed.load(Ordering::Acquire));
    assert_eq!(
        f.manager.snapshot("owner", f.id).unwrap().state.kind(),
        DebugSessionStateKind::Stopped
    );
}
