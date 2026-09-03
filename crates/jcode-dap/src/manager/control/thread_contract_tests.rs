use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::timeout;

use super::*;
use crate::Message;
use crate::testing::FakeAdapter;

struct Fixture {
    manager: DebugSessionManager,
    id: DebugSessionId,
    adapter: FakeAdapter,
    root: PathBuf,
}

fn fixture(max_threads: usize, max_thread_name_bytes: usize) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-thread-contract-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    std::fs::write(&program, b"x").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_millis(200),
            max_threads,
            max_thread_name_bytes,
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
        Message::Request(request) => request,
        other => panic!("expected request, got {other:?}"),
    }
}

async fn threads_round_trip(f: &mut Fixture, body: Value) -> Result<DebugThreadsSnapshot> {
    let manager = f.manager.clone();
    let id = f.id;
    let task = tokio::spawn(async move { manager.threads("owner", id).await });
    let request = recv(&mut f.adapter).await;
    assert_eq!(request.command, "threads");
    assert!(request.arguments.is_none());
    f.adapter.respond_ok(&request, Some(body)).await.unwrap();
    task.await.unwrap()
}

#[tokio::test]
async fn threads_round_trip_is_fresh_ephemeral_ordered_and_revisioned() {
    let mut f = fixture(4, 16);
    for expected in [vec![2, 1], vec![3]] {
        let body = json!({"threads": expected.iter().map(|id| json!({"id":id,"name":format!("t{id}")})).collect::<Vec<_>>()});
        let snapshot = threads_round_trip(&mut f, body).await.unwrap();
        assert_eq!(
            snapshot
                .threads
                .iter()
                .map(|thread| thread.id.get())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(snapshot.execution_revision.get(), 0);
        assert_eq!(snapshot.state, DebugSessionStateKind::Running);
    }
}

#[tokio::test]
async fn malformed_thread_matrix_fails_without_partial_cache_and_recovers() {
    let cases = [
        json!({"threads":[{"id":1,"name":"a"},{"id":1,"name":"b"}]}),
        json!({"threads":[{"id":2147483648_i64,"name":"a"}]}),
        json!({"threads":[{"id":1}]}),
        json!({"threads":[{"id":1,"name":"12345"}]}),
        json!({"threads":[{"id":1,"name":"a"},{"id":2,"name":"b"},{"id":3,"name":"c"}]}),
    ];
    for malformed in cases {
        let mut f = fixture(2, 4);
        assert!(threads_round_trip(&mut f, malformed).await.is_err());
        let recovered = threads_round_trip(&mut f, json!({"threads":[{"id":9,"name":"ok"}]}))
            .await
            .unwrap();
        assert_eq!(
            recovered.threads,
            vec![DebugThread {
                id: DebugThreadId::new(9),
                name: "ok".into()
            }]
        );
    }
}

#[tokio::test]
async fn missing_focus_single_thread_selects_but_multiple_threads_are_ambiguous_without_control() {
    for (threads, should_continue) in [
        (json!([{"id":4,"name":"only"}]), true),
        (json!([{"id":4,"name":"one"},{"id":5,"name":"two"}]), false),
    ] {
        let mut f = fixture(4, 16);
        f.adapter
            .event(
                "stopped",
                Some(json!({"reason":"breakpoint","allThreadsStopped":true})),
            )
            .await
            .unwrap();
        wait_for_state(&f.manager, f.id, DebugSessionStateKind::Stopped).await;
        let manager = f.manager.clone();
        let id = f.id;
        let task = tokio::spawn(async move {
            manager
                .continue_execution("owner", id, DebugContinueRequest::default())
                .await
        });
        let lookup = recv(&mut f.adapter).await;
        assert_eq!(lookup.command, "threads");
        f.adapter
            .respond_ok(&lookup, Some(json!({"threads":threads})))
            .await
            .unwrap();
        if should_continue {
            let request = recv(&mut f.adapter).await;
            assert_eq!(request.command, "continue");
            assert_eq!(request.arguments, Some(json!({"threadId":4})));
            f.adapter
                .respond_ok(&request, Some(json!({})))
                .await
                .unwrap();
            task.await.unwrap().unwrap();
        } else {
            assert!(matches!(
                task.await.unwrap(),
                Err(DapError::AmbiguousStoppedThread {
                    observed_threads: 2,
                    ..
                })
            ));
            assert!(
                timeout(Duration::from_millis(20), f.adapter.recv())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn explicit_alternate_stopped_thread_requires_all_stopped_and_fresh_membership() {
    for (all_stopped, listed, expected_error) in [
        (false, true, "unavailable"),
        (true, false, "missing"),
        (true, true, "ok"),
    ] {
        let mut f = fixture(4, 16);
        f.adapter
            .event(
                "stopped",
                Some(json!({"reason":"breakpoint","threadId":1,"allThreadsStopped":all_stopped})),
            )
            .await
            .unwrap();
        wait_for_state(&f.manager, f.id, DebugSessionStateKind::Stopped).await;
        let request = DebugContinueRequest::default().with_thread_id(DebugThreadId::new(2));
        let manager = f.manager.clone();
        let id = f.id;
        let task =
            tokio::spawn(async move { manager.continue_execution("owner", id, request).await });
        if !all_stopped {
            assert!(matches!(
                task.await.unwrap(),
                Err(DapError::StoppedThreadUnavailable { .. })
            ));
            assert!(
                timeout(Duration::from_millis(20), f.adapter.recv())
                    .await
                    .is_err()
            );
            continue;
        }
        let lookup = recv(&mut f.adapter).await;
        assert_eq!(lookup.command, "threads");
        let threads = if listed {
            json!([{"id":1,"name":"focus"},{"id":2,"name":"alternate"}])
        } else {
            json!([{"id":1,"name":"focus"}])
        };
        f.adapter
            .respond_ok(&lookup, Some(json!({"threads":threads})))
            .await
            .unwrap();
        if expected_error == "missing" {
            assert!(matches!(
                task.await.unwrap(),
                Err(DapError::ThreadNotFound { .. })
            ));
            assert!(
                timeout(Duration::from_millis(20), f.adapter.recv())
                    .await
                    .is_err()
            );
        } else {
            let control = recv(&mut f.adapter).await;
            assert_eq!(control.arguments, Some(json!({"threadId":2})));
            f.adapter
                .respond_ok(&control, Some(json!({})))
                .await
                .unwrap();
            task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn pause_always_verifies_explicit_thread_and_omitted_requires_one() {
    for (requested, threads, succeeds) in [
        (
            Some(DebugThreadId::new(7)),
            json!([{"id":7,"name":"chosen"},{"id":8,"name":"other"}]),
            true,
        ),
        (
            Some(DebugThreadId::new(7)),
            json!([{"id":8,"name":"other"}]),
            false,
        ),
        (None, json!([{"id":7,"name":"only"}]), true),
        (
            None,
            json!([{"id":7,"name":"one"},{"id":8,"name":"two"}]),
            false,
        ),
    ] {
        let mut f = fixture(4, 16);
        let request = DebugPauseRequest {
            thread_id: requested,
            ..Default::default()
        };
        let manager = f.manager.clone();
        let id = f.id;
        let task = tokio::spawn(async move { manager.pause("owner", id, request).await });
        let lookup = recv(&mut f.adapter).await;
        assert_eq!(lookup.command, "threads");
        f.adapter
            .respond_ok(&lookup, Some(json!({"threads":threads})))
            .await
            .unwrap();
        if succeeds {
            let pause = recv(&mut f.adapter).await;
            assert_eq!(pause.command, "pause");
            assert_eq!(pause.arguments, Some(json!({"threadId":7})));
            f.adapter.respond_ok(&pause, None).await.unwrap();
            task.await.unwrap().unwrap();
        } else {
            assert!(task.await.unwrap().is_err());
            assert!(
                timeout(Duration::from_millis(20), f.adapter.recv())
                    .await
                    .is_err()
            );
        }
    }
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
