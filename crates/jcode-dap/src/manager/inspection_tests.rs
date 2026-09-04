use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use tokio::time::{sleep, timeout};

use super::*;
use crate::testing::FakeAdapter;
use crate::{
    DebugEvaluateContext, DebugEvaluateOutcome, DebugEvaluateRequest, DebugEvaluateTarget,
    DebugScopesRequest, DebugSessionManagerConfig, DebugStackTraceRequest,
    DebugStepInTargetsRequest, DebugSteppingGranularity, DebugTargetedStepInRequest,
    DebugVariablesRequest, Message,
};

struct InspectionFixture {
    manager: DebugSessionManager,
    id: DebugSessionId,
    adapter: FakeAdapter,
    root: PathBuf,
}

fn fixture() -> InspectionFixture {
    let root = std::env::temp_dir().join(format!(
        "jcode-dap-inspection-{}-{}",
        std::process::id(),
        crate::session::next_manager_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("main");
    std::fs::write(&program, b"x").unwrap();
    let manager = DebugSessionManager::new_with_operation_config(
        DebugSessionManagerConfig::default(),
        DebugOperationConfig {
            operation_timeout: Duration::from_secs(1),
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
    InspectionFixture {
        manager,
        id,
        adapter,
        root,
    }
}

impl Drop for InspectionFixture {
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
        other => panic!("expected request: {other:?}"),
    }
}

async fn stop(fixture: &mut InspectionFixture) {
    fixture
        .adapter
        .event(
            "stopped",
            Some(json!({
                "reason": "breakpoint",
                "threadId": 7,
                "allThreadsStopped": true
            })),
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                fixture.manager.snapshot("owner", fixture.id).unwrap().state,
                DebugSessionState::Stopped(_)
            ) {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn inspection_stack_scopes_variables_and_evaluate_round_trip() {
    let mut fixture = fixture();
    {
        let entry = fixture
            .manager
            .core
            .authorized_entry("owner", fixture.id)
            .unwrap();
        lock(&entry.data)
            .capabilities
            .additional
            .insert("supportsStepInTargetsRequest".into(), json!(true));
    }
    stop(&mut fixture).await;

    let stack_task = {
        let manager = fixture.manager.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .stack_trace("owner", id, DebugStackTraceRequest::new(2))
                .await
        })
    };
    let stack_request = recv(&mut fixture.adapter).await;
    assert_eq!(stack_request.command, "stackTrace");
    assert_eq!(stack_request.arguments, Some(json!({"threadId": 7})));
    fixture
        .adapter
        .respond_ok(
            &stack_request,
            Some(json!({
                "stackFrames": [{"id": 11, "name": "main", "line": 3, "column": 5}],
                "totalFrames": 1
            })),
        )
        .await
        .unwrap();
    let stack = stack_task.await.unwrap().unwrap();
    assert_eq!(stack.thread_id.get(), 7);
    assert_eq!(stack.frames.len(), 1);
    let frame = stack.frames[0].handle.clone();

    let targets_task = {
        let manager = fixture.manager.clone();
        let frame = frame.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .step_in_targets(
                    "owner",
                    id,
                    DebugStepInTargetsRequest {
                        frame,
                        expected_execution_revision: Some(stack.execution_revision),
                    },
                )
                .await
        })
    };
    let targets_request = recv(&mut fixture.adapter).await;
    assert_eq!(targets_request.command, "stepInTargets");
    assert_eq!(targets_request.arguments, Some(json!({"frameId": 11})));
    fixture
        .adapter
        .respond_ok(
            &targets_request,
            Some(json!({
                "targets": [{
                    "id": 41,
                    "label": "call helper",
                    "line": 3,
                    "column": 7,
                    "instructionPointerReference": "0x29"
                }]
            })),
        )
        .await
        .unwrap();
    let targets = targets_task.await.unwrap().unwrap();
    assert_eq!(targets.targets[0].label, "call helper");
    assert_eq!(targets.targets[0].line, Some(3));
    let step_target = targets.targets[0].handle.clone();

    let scopes_task = {
        let manager = fixture.manager.clone();
        let frame = frame.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .scopes(
                    "owner",
                    id,
                    DebugScopesRequest {
                        frame,
                        expected_execution_revision: Some(stack.execution_revision),
                    },
                )
                .await
        })
    };
    let scopes_request = recv(&mut fixture.adapter).await;
    assert_eq!(scopes_request.command, "scopes");
    assert_eq!(scopes_request.arguments, Some(json!({"frameId": 11})));
    fixture
        .adapter
        .respond_ok(
            &scopes_request,
            Some(json!({
                "scopes": [{
                    "name": "Locals",
                    "variablesReference": 21,
                    "expensive": false
                }]
            })),
        )
        .await
        .unwrap();
    let scopes = scopes_task.await.unwrap().unwrap();
    let variables_handle = scopes.scopes[0].variables.clone();
    assert!(variables_handle.is_expandable());

    let variables_task = {
        let manager = fixture.manager.clone();
        let variables = variables_handle.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .variables(
                    "owner",
                    id,
                    DebugVariablesRequest {
                        variables,
                        filter: None,
                        start: 0,
                        count: 2,
                        expected_execution_revision: Some(stack.execution_revision),
                    },
                )
                .await
        })
    };
    let variables_request = recv(&mut fixture.adapter).await;
    assert_eq!(variables_request.command, "variables");
    fixture
        .adapter
        .respond_ok(
            &variables_request,
            Some(json!({
                "variables": [{
                    "name": "answer",
                    "value": "42",
                    "type": "int",
                    "variablesReference": 0
                }]
            })),
        )
        .await
        .unwrap();
    let variables = variables_task.await.unwrap().unwrap();
    assert_eq!(variables.variables[0].value.text, "42");
    assert!(!variables.variables[0].variables.is_expandable());

    let evaluate_task = {
        let manager = fixture.manager.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .evaluate(
                    "owner",
                    id,
                    DebugEvaluateRequest {
                        expression: "answer".into(),
                        context: DebugEvaluateContext::Watch,
                        target: DebugEvaluateTarget::Frame(frame),
                        expected_execution_revision: stack.execution_revision,
                    },
                )
                .await
        })
    };
    let evaluate_request = recv(&mut fixture.adapter).await;
    assert_eq!(evaluate_request.command, "evaluate");
    assert_eq!(
        evaluate_request.arguments,
        Some(json!({"expression": "answer", "context": "watch", "frameId": 11}))
    );
    fixture
        .adapter
        .respond_ok(
            &evaluate_request,
            Some(json!({"result": "42", "type": "int", "variablesReference": 0})),
        )
        .await
        .unwrap();
    match evaluate_task.await.unwrap().unwrap() {
        DebugEvaluateOutcome::Known(result) => {
            assert_eq!(result.result, "42");
            assert!(!result.variables.is_expandable());
        }
        DebugEvaluateOutcome::Unknown(unknown) => panic!("unexpected unknown: {unknown:?}"),
    }

    let step_task = {
        let manager = fixture.manager.clone();
        let id = fixture.id;
        tokio::spawn(async move {
            manager
                .step_in_target(
                    "owner",
                    id,
                    DebugTargetedStepInRequest {
                        target: step_target,
                        expected_execution_revision: Some(stack.execution_revision),
                        granularity: DebugSteppingGranularity::Statement,
                    },
                )
                .await
        })
    };
    let step_request = recv(&mut fixture.adapter).await;
    assert_eq!(step_request.command, "stepIn");
    assert_eq!(
        step_request.arguments,
        Some(json!({"threadId": 7, "targetId": 41}))
    );
    fixture
        .adapter
        .respond_ok(&step_request, None)
        .await
        .unwrap();
    let stepped = step_task.await.unwrap().unwrap();
    assert_eq!(stepped.thread_id.get(), 7);
    assert!(stepped.execution_revision.get() > stack.execution_revision.get());
}
