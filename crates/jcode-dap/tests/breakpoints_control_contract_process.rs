#![cfg(unix)]

mod breakpoints_control_support;

use std::fs;
use std::os::unix::fs::symlink;
use std::time::Duration;

use breakpoints_control_support::*;
use jcode_dap::{
    DebugBreakpointMutation, DebugContinueRequest, DebugPauseRequest, DebugRemoveBreakpointRequest,
    DebugSessionStateKind, DebugSetBreakpointRequest, DebugSourceBreakpoint, DebugStepRequest,
};

#[tokio::test]
async fn subprocess_full_set_order_and_remove_clear() {
    for iteration in 0..2 {
        let workspace = Workspace::new(&format!("full-set-{iteration}"));
        let source = workspace.source("unicode source λ.rs");
        let manager = manager(Duration::from_secs(2));
        let launched = launch_stopped(&workspace, &manager).await;
        let first = manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(&source, DebugSourceBreakpoint::new(10)),
            )
            .await
            .unwrap();
        let first_id = match first.mutation {
            DebugBreakpointMutation::Created { breakpoint_id } => breakpoint_id,
            other => panic!("unexpected mutation: {other:?}"),
        };
        let second = manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(&source, DebugSourceBreakpoint::new(20)),
            )
            .await
            .unwrap();
        let second_id = match second.mutation {
            DebugBreakpointMutation::Created { breakpoint_id } => breakpoint_id,
            other => panic!("unexpected mutation: {other:?}"),
        };
        manager
            .remove_breakpoint(
                "owner",
                launched.id,
                DebugRemoveBreakpointRequest::new(first_id),
            )
            .await
            .unwrap();
        manager
            .remove_breakpoint(
                "owner",
                launched.id,
                DebugRemoveBreakpointRequest::new(second_id),
            )
            .await
            .unwrap();

        let canonical = fs::canonicalize(&source).unwrap();
        let sets = command_lines(&workspace.log())
            .into_iter()
            .filter(|(command, _)| *command == "setBreakpoints")
            .map(|(_, body)| body)
            .collect::<Vec<_>>();
        assert_eq!(sets.len(), 4);
        assert_eq!(sets[0]["source"]["path"], canonical.to_str().unwrap());
        assert_eq!(sets[0]["breakpoints"], serde_json::json!([{"line":10}]));
        assert_eq!(
            sets[1]["breakpoints"],
            serde_json::json!([{"line":10},{"line":20}])
        );
        assert_eq!(sets[2]["breakpoints"], serde_json::json!([{"line":20}]));
        assert_eq!(sets[3]["breakpoints"], serde_json::json!([]));

        let adapter_pid = workspace.logged_pid("adapter_pid");
        manager.terminate("owner", launched.id).await.unwrap();
        wait_tree_gone(adapter_pid, None, None).await;
    }
}

#[tokio::test]
async fn subprocess_continue_pause_steps_threads_preserve_wire_order() {
    let workspace = Workspace::new("controls");
    let manager = manager(Duration::from_secs(2));
    let launched = launch_stopped(&workspace, &manager).await;
    let threads = manager.threads("owner", launched.id).await.unwrap();
    assert_eq!(threads.threads.len(), 2);

    manager
        .continue_execution("owner", launched.id, DebugContinueRequest::default())
        .await
        .unwrap();
    assert_eq!(
        manager.snapshot("owner", launched.id).unwrap().state.kind(),
        DebugSessionStateKind::Running
    );

    workspace.marker("control-stops");
    manager
        .pause(
            "owner",
            launched.id,
            DebugPauseRequest::default().with_thread_id(threads.threads[0].id),
        )
        .await
        .unwrap();
    wait_state(&manager, launched.id, DebugSessionStateKind::Stopped).await;
    for command in ["next", "stepIn", "stepOut"] {
        match command {
            "next" => manager
                .step_over("owner", launched.id, DebugStepRequest::default())
                .await
                .unwrap(),
            "stepIn" => manager
                .step_in("owner", launched.id, DebugStepRequest::default())
                .await
                .unwrap(),
            _ => manager
                .step_out("owner", launched.id, DebugStepRequest::default())
                .await
                .unwrap(),
        };
        wait_state(&manager, launched.id, DebugSessionStateKind::Stopped).await;
    }

    let log = workspace.log();
    let lines = command_lines(&log);
    let controls = lines
        .iter()
        .filter(|(command, _)| {
            matches!(
                *command,
                "threads" | "continue" | "pause" | "next" | "stepIn" | "stepOut"
            )
        })
        .map(|(command, _)| *command)
        .collect::<Vec<_>>();
    assert_eq!(
        controls,
        [
            "threads", "continue", "threads", "pause", "next", "stepIn", "stepOut",
        ]
    );
    let pause = lines
        .iter()
        .find(|(command, _)| *command == "pause")
        .unwrap();
    assert_eq!(pause.1, serde_json::json!({"threadId":1}));
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}

#[tokio::test]
async fn subprocess_unicode_spaces_and_symlink_containment() {
    let workspace = Workspace::new("symlinks");
    let manager = manager(Duration::from_secs(2));
    let launched = launch_stopped(&workspace, &manager).await;
    let inside = workspace.source("inside source λ.rs");
    let inside_link = workspace.root.join("inside link λ.rs");
    symlink(&inside, &inside_link).unwrap();
    manager
        .set_breakpoint(
            "owner",
            launched.id,
            DebugSetBreakpointRequest::new(&inside_link, DebugSourceBreakpoint::new(1)),
        )
        .await
        .unwrap();
    let before = command_lines(&workspace.log()).len();
    let outside_root = Workspace::new("outside");
    let outside = outside_root.source("outside.rs");
    let escaping = workspace.root.join("escaping link.rs");
    symlink(outside, &escaping).unwrap();
    assert!(
        manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(escaping, DebugSourceBreakpoint::new(2)),
            )
            .await
            .is_err()
    );
    assert_eq!(before, command_lines(&workspace.log()).len());
    let log = workspace.log();
    let set = command_lines(&log)
        .into_iter()
        .find(|(command, _)| *command == "setBreakpoints")
        .unwrap();
    assert_eq!(
        set.1["source"]["path"],
        fs::canonicalize(inside).unwrap().to_str().unwrap()
    );
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}

#[tokio::test]
async fn subprocess_source_mutation_runs_compensating_clear() {
    let workspace = Workspace::new("mutation");
    workspace.marker("mutate-source-on-set");
    let source = workspace.source("mutable.rs");
    let manager = manager(Duration::from_secs(2));
    let launched = launch_stopped(&workspace, &manager).await;
    let error = manager
        .set_breakpoint(
            "owner",
            launched.id,
            DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("changed"));
    let sets = command_lines(&workspace.log())
        .into_iter()
        .filter(|(command, _)| *command == "setBreakpoints")
        .map(|(_, body)| body)
        .collect::<Vec<_>>();
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0]["breakpoints"], serde_json::json!([{"line":1}]));
    assert_eq!(sets[1]["breakpoints"], serde_json::json!([]));
    assert_eq!(
        manager
            .breakpoints("owner", launched.id)
            .unwrap()
            .total_breakpoints,
        0
    );
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}

#[tokio::test]
async fn subprocess_timeout_emits_cancel_hint_and_recovers() {
    let workspace = Workspace::new("timeout");
    workspace.marker("hang-set-breakpoints");
    let source = workspace.source("timeout.rs");
    let manager = manager(Duration::from_millis(100));
    let launched = launch_stopped(&workspace, &manager).await;
    assert!(
        manager
            .set_breakpoint(
                "owner",
                launched.id,
                DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
            )
            .await
            .is_err()
    );
    workspace.wait_log("cancel\t").await;
    let threads = manager.threads("owner", launched.id).await.unwrap();
    assert_eq!(threads.threads.len(), 2);
    let commands = command_lines(&workspace.log())
        .into_iter()
        .map(|(command, _)| command.to_owned())
        .collect::<Vec<_>>();
    let set = commands
        .iter()
        .position(|command| command == "setBreakpoints")
        .unwrap();
    let cancel = commands
        .iter()
        .position(|command| command == "cancel")
        .unwrap();
    let threads = commands
        .iter()
        .rposition(|command| command == "threads")
        .unwrap();
    assert!(set < cancel && cancel < threads);
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}
