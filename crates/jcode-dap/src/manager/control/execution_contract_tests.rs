use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::timeout;

use super::*;
use crate::testing::FakeAdapter;
use crate::{Capabilities, DebugSetBreakpointRequest, DebugSourceBreakpoint, Message};

struct Fixture {
    manager: DebugSessionManager,
    id: DebugSessionId,
    adapter: FakeAdapter,
    root: PathBuf,
    source: PathBuf,
}

fn fixture(timeout_ms: u64, cancel: bool, stepping_granularity: Option<Value>) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-execution-contract-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    let source = root.join("main.rs");
    std::fs::write(&program, b"x").unwrap();
    std::fs::write(&source, b"fn main() {}\n").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_millis(timeout_ms),
            ..Default::default()
        },
    )
    .unwrap();
    let (client, adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: "owner".into(),
            workspace: DebugWorkspaceKey::new(&root, "owner").unwrap(),
            adapter_id: "fake".into(),
            start: Some(DebugSessionStart::Launch {
                program,
                cwd: root.clone(),
            }),
        })
        .unwrap();
    reservation.attach_client(client).unwrap();
    reservation.mark_configuring().unwrap();
    let mut additional = BTreeMap::new();
    if let Some(value) = stepping_granularity {
        additional.insert("supportsSteppingGranularity".into(), value);
    }
    reservation
        .set_capabilities(Capabilities {
            supports_cancel_request: Some(cancel),
            additional,
            ..Default::default()
        })
        .unwrap();
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

async fn recv(adapter: &mut FakeAdapter) -> crate::Request {
    match timeout(Duration::from_secs(1), adapter.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

async fn stop(f: &mut Fixture, thread_id: Option<i64>, all: bool) {
    let mut body = json!({"reason":"breakpoint","allThreadsStopped":all});
    if let Some(id) = thread_id {
        body["threadId"] = json!(id);
    }
    f.adapter.event("stopped", Some(body)).await.unwrap();
    wait_for_state(&f.manager, f.id, DebugSessionStateKind::Stopped).await;
}

async fn wait_for_state(
    manager: &DebugSessionManager,
    id: DebugSessionId,
    kind: DebugSessionStateKind,
) {
    timeout(Duration::from_secs(1), async {
        loop {
            if manager.snapshot("owner", id).unwrap().state.kind() == kind {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn exact_control_commands_bodies_and_continue_false_semantics() {
    let mut f = fixture(200, false, Some(json!(true)));
    stop(&mut f, Some(3), true).await;
    for (operation, command, granularity) in [
        (
            DebugControlOperation::Continue,
            "continue",
            DebugSteppingGranularity::Statement,
        ),
        (
            DebugControlOperation::StepOver,
            "next",
            DebugSteppingGranularity::Line,
        ),
        (
            DebugControlOperation::StepIn,
            "stepIn",
            DebugSteppingGranularity::Instruction,
        ),
        (
            DebugControlOperation::StepOut,
            "stepOut",
            DebugSteppingGranularity::Statement,
        ),
    ] {
        let manager = f.manager.clone();
        let id = f.id;
        let task = tokio::spawn(async move {
            match operation {
                DebugControlOperation::Continue => {
                    manager
                        .continue_execution("owner", id, DebugContinueRequest::default())
                        .await
                }
                DebugControlOperation::StepOver => {
                    manager
                        .step_over(
                            "owner",
                            id,
                            DebugStepRequest::default().with_granularity(granularity),
                        )
                        .await
                }
                DebugControlOperation::StepIn => {
                    manager
                        .step_in(
                            "owner",
                            id,
                            DebugStepRequest::default().with_granularity(granularity),
                        )
                        .await
                }
                DebugControlOperation::StepOut => {
                    manager
                        .step_out("owner", id, DebugStepRequest::default())
                        .await
                }
                DebugControlOperation::Pause => unreachable!(),
            }
        });
        let request = recv(&mut f.adapter).await;
        assert_eq!(request.command, command);
        let expected = if granularity == DebugSteppingGranularity::Statement
            || operation == DebugControlOperation::Continue
        {
            json!({"threadId":3})
        } else {
            json!({"threadId":3,"granularity": if granularity == DebugSteppingGranularity::Line { "line" } else { "instruction" }})
        };
        assert_eq!(request.arguments, Some(expected));
        assert!(
            request
                .arguments
                .as_ref()
                .unwrap()
                .get("singleThread")
                .is_none()
        );
        assert!(
            request
                .arguments
                .as_ref()
                .unwrap()
                .get("targetId")
                .is_none()
        );
        let body = (operation == DebugControlOperation::Continue)
            .then(|| json!({"allThreadsContinued":false}));
        f.adapter.respond_ok(&request, body).await.unwrap();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result.state.kind(), DebugSessionStateKind::Running);
        if operation == DebugControlOperation::Continue {
            assert_eq!(result.all_threads_continued, Some(false));
        }
        if operation != DebugControlOperation::StepOut {
            stop(&mut f, Some(3), true).await;
        }
    }
}

#[tokio::test]
async fn stopped_event_before_control_response_remains_authoritative() {
    let mut f = fixture(200, false, None);
    stop(&mut f, Some(1), true).await;
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .continue_execution("owner", id, DebugContinueRequest::default())
            .await
    });
    let request = recv(&mut f.adapter).await;
    f.adapter
        .event(
            "stopped",
            Some(json!({"reason":"breakpoint","threadId":2,"allThreadsStopped":true})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = f.manager.snapshot("owner", f.id).unwrap();
            if matches!(snapshot.state, DebugSessionState::Stopped(ref stopped) if stopped.thread_id == Some(2)) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let revision = f
        .manager
        .core
        .entry(f.id)
        .unwrap()
        .data
        .lock()
        .unwrap()
        .execution_revision;
    f.adapter
        .respond_ok(&request, Some(json!({})))
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.state.kind(), DebugSessionStateKind::Stopped);
    assert_eq!(result.execution_revision.get(), revision);
}

#[tokio::test]
async fn continued_event_before_response_is_idempotent() {
    let mut f = fixture(200, false, None);
    stop(&mut f, Some(1), true).await;
    let before = f
        .manager
        .core
        .entry(f.id)
        .unwrap()
        .data
        .lock()
        .unwrap()
        .execution_revision;
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .continue_execution("owner", id, DebugContinueRequest::default())
            .await
    });
    let request = recv(&mut f.adapter).await;
    f.adapter
        .event(
            "continued",
            Some(json!({"threadId":1,"allThreadsContinued":true})),
        )
        .await
        .unwrap();
    wait_for_state(&f.manager, f.id, DebugSessionStateKind::Running).await;
    f.adapter
        .respond_ok(&request, Some(json!({})))
        .await
        .unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.execution_revision.get(), before + 1);
}

#[tokio::test]
async fn malformed_successful_continue_response_conservatively_commits_running() {
    let mut f = fixture(200, false, None);
    stop(&mut f, Some(1), true).await;
    let entry = f.manager.core.entry(f.id).unwrap();
    let before = lock(&entry.data).execution_revision;
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .continue_execution("owner", id, DebugContinueRequest::default())
            .await
    });
    let request = recv(&mut f.adapter).await;
    f.adapter
        .respond_ok(&request, Some(json!({"allThreadsContinued":"invalid"})))
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::InvalidMessage(_))
    ));
    let after = f.manager.snapshot("owner", f.id).unwrap();
    assert_eq!(after.state.kind(), DebugSessionStateKind::Running);
    assert_eq!(lock(&entry.data).execution_revision, before + 1);
}

#[tokio::test]
async fn newer_stopped_event_remains_authoritative_after_malformed_continue_response() {
    let mut f = fixture(200, false, None);
    stop(&mut f, Some(1), true).await;
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .continue_execution("owner", id, DebugContinueRequest::default())
            .await
    });
    let request = recv(&mut f.adapter).await;
    f.adapter
        .event(
            "stopped",
            Some(json!({"reason":"breakpoint","threadId":2,"allThreadsStopped":true})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                f.manager.snapshot("owner", f.id).unwrap().state,
                DebugSessionState::Stopped(ref stopped) if stopped.thread_id == Some(2)
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let authoritative = f.manager.snapshot("owner", f.id).unwrap();
    f.adapter
        .respond_ok(&request, Some(json!({"allThreadsContinued":"invalid"})))
        .await
        .unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(f.manager.snapshot("owner", f.id).unwrap(), authoritative);
}

#[tokio::test]
async fn output_and_breakpoint_events_do_not_advance_execution_revision() {
    let mut f = fixture(200, false, None);
    let entry = f.manager.core.entry(f.id).unwrap();
    let before = lock(&entry.data).execution_revision;
    f.adapter
        .event("output", Some(json!({"category":"stdout","output":"x"})))
        .await
        .unwrap();
    f.adapter
        .event(
            "breakpoint",
            Some(json!({"reason":"changed","breakpoint":{"id":999,"verified":false}})),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if !f
                .manager
                .output("owner", f.id, None, 10)
                .unwrap()
                .records
                .is_empty()
                && f.manager
                    .breakpoints("owner", f.id)
                    .unwrap()
                    .unmatched_adapter_events
                    == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(lock(&entry.data).execution_revision, before);
}

#[tokio::test]
async fn expected_revision_mismatch_and_unsupported_granularity_emit_zero_control_traffic() {
    let mut f = fixture(200, false, Some(json!(false)));
    stop(&mut f, Some(1), true).await;
    let stale =
        DebugContinueRequest::default().with_expected_execution_revision(DebugExecutionRevision(0));
    assert!(matches!(
        f.manager.continue_execution("owner", f.id, stale).await,
        Err(DapError::StaleExecutionRevision { .. })
    ));
    let unsupported = DebugStepRequest::default().with_granularity(DebugSteppingGranularity::Line);
    assert!(matches!(
        f.manager.step_over("owner", f.id, unsupported).await,
        Err(DapError::UnsupportedDapCapability { .. })
    ));
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn continue_and_step_timeouts_leave_running_and_later_stop_recovers() {
    for operation in [
        DebugControlOperation::Continue,
        DebugControlOperation::StepOver,
    ] {
        let mut f = fixture(40, false, None);
        stop(&mut f, Some(1), true).await;
        let manager = f.manager.clone();
        let id = f.id;
        let result = match operation {
            DebugControlOperation::Continue => {
                manager
                    .continue_execution("owner", id, DebugContinueRequest::default())
                    .await
            }
            _ => {
                manager
                    .step_over("owner", id, DebugStepRequest::default())
                    .await
            }
        };
        assert!(matches!(result, Err(DapError::RequestTimeout { .. })));
        assert_eq!(
            f.manager.snapshot("owner", f.id).unwrap().state.kind(),
            DebugSessionStateKind::Running
        );
        let request = recv(&mut f.adapter).await;
        assert_eq!(request.command, command(operation));
        f.adapter
            .event(
                "stopped",
                Some(json!({"reason":"step","threadId":1,"allThreadsStopped":true})),
            )
            .await
            .unwrap();
        wait_for_state(&f.manager, f.id, DebugSessionStateKind::Stopped).await;
    }
}

#[tokio::test]
async fn pause_timeout_leaves_running_and_session_recovers_with_threads() {
    let mut f = fixture(50, false, None);
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move {
        manager
            .pause(
                "owner",
                id,
                DebugPauseRequest::default().with_thread_id(DebugThreadId::new(1)),
            )
            .await
    });
    let lookup = recv(&mut f.adapter).await;
    f.adapter
        .respond_ok(&lookup, Some(json!({"threads":[{"id":1,"name":"main"}]})))
        .await
        .unwrap();
    let pause = recv(&mut f.adapter).await;
    assert_eq!(pause.command, "pause");
    assert!(matches!(
        task.await.unwrap(),
        Err(DapError::RequestTimeout { .. })
    ));
    assert_eq!(
        f.manager.snapshot("owner", f.id).unwrap().state.kind(),
        DebugSessionStateKind::Running
    );
    let manager = f.manager.clone();
    let id = f.id;
    let retry = tokio::spawn(async move { manager.threads("owner", id).await });
    let request = recv(&mut f.adapter).await;
    f.adapter
        .respond_ok(&request, Some(json!({"threads":[]})))
        .await
        .unwrap();
    retry.await.unwrap().unwrap();
}

#[tokio::test]
async fn capability_driven_cancel_is_emitted_only_when_advertised() {
    for advertised in [false, true] {
        let mut f = fixture(40, advertised, None);
        stop(&mut f, Some(1), true).await;
        let result = f
            .manager
            .continue_execution("owner", f.id, DebugContinueRequest::default())
            .await;
        assert!(matches!(result, Err(DapError::RequestTimeout { .. })));
        let primary = recv(&mut f.adapter).await;
        assert_eq!(primary.command, "continue");
        if advertised {
            let cancel = recv(&mut f.adapter).await;
            assert_eq!(cancel.command, "cancel");
            assert_eq!(cancel.arguments, Some(json!({"requestId":primary.seq})));
        } else {
            assert!(
                timeout(Duration::from_millis(20), f.adapter.recv())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn adapter_rejection_never_synthesizes_control_state_transition() {
    for pause in [false, true] {
        let mut f = fixture(200, false, None);
        if !pause {
            stop(&mut f, Some(1), true).await;
        }
        let before = f.manager.snapshot("owner", f.id).unwrap().state.kind();
        let manager = f.manager.clone();
        let id = f.id;
        let task = tokio::spawn(async move {
            if pause {
                manager
                    .pause(
                        "owner",
                        id,
                        DebugPauseRequest::default().with_thread_id(DebugThreadId::new(1)),
                    )
                    .await
            } else {
                manager
                    .continue_execution("owner", id, DebugContinueRequest::default())
                    .await
            }
        });
        if pause {
            let lookup = recv(&mut f.adapter).await;
            f.adapter
                .respond_ok(&lookup, Some(json!({"threads":[{"id":1,"name":"main"}]})))
                .await
                .unwrap();
        }
        let request = recv(&mut f.adapter).await;
        f.adapter.respond_error(&request, "rejected").await.unwrap();
        assert!(task.await.unwrap().is_err());
        assert_eq!(
            f.manager.snapshot("owner", f.id).unwrap().state.kind(),
            before
        );
    }
}

#[tokio::test]
async fn concurrent_controls_serialize_and_second_rechecks_stopped_state() {
    let mut f = fixture(200, false, None);
    stop(&mut f, Some(1), true).await;
    let first_manager = f.manager.clone();
    let second_manager = f.manager.clone();
    let id = f.id;
    let first = tokio::spawn(async move {
        first_manager
            .continue_execution("owner", id, DebugContinueRequest::default())
            .await
    });
    let request = recv(&mut f.adapter).await;
    let second = tokio::spawn(async move {
        second_manager
            .step_over("owner", id, DebugStepRequest::default())
            .await
    });
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
    f.adapter
        .respond_ok(&request, Some(json!({})))
        .await
        .unwrap();
    first.await.unwrap().unwrap();
    assert!(second.await.unwrap().is_err());
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn breakpoint_mutation_and_control_share_one_operation_gate() {
    let mut f = fixture(300, false, None);
    let set_manager = f.manager.clone();
    let id = f.id;
    let source = f.source.clone();
    let set = tokio::spawn(async move {
        set_manager
            .set_breakpoint(
                "owner",
                id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
            )
            .await
    });
    let breakpoint = recv(&mut f.adapter).await;
    assert_eq!(breakpoint.command, "setBreakpoints");
    let pause_manager = f.manager.clone();
    let pause = tokio::spawn(async move {
        pause_manager
            .pause("owner", id, DebugPauseRequest::default())
            .await
    });
    assert!(
        timeout(Duration::from_millis(20), f.adapter.recv())
            .await
            .is_err()
    );
    f.adapter
        .respond_ok(
            &breakpoint,
            Some(json!({"breakpoints":[{"id":9,"verified":true}]})),
        )
        .await
        .unwrap();
    set.await.unwrap().unwrap();
    let threads = recv(&mut f.adapter).await;
    assert_eq!(threads.command, "threads");
    f.adapter
        .respond_ok(&threads, Some(json!({"threads":[{"id":1,"name":"main"}]})))
        .await
        .unwrap();
    let request = recv(&mut f.adapter).await;
    assert_eq!(request.command, "pause");
    f.adapter.respond_ok(&request, None).await.unwrap();
    pause.await.unwrap().unwrap();
}
