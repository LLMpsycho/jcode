#![cfg(unix)]

mod breakpoints_control_support;

use std::time::Duration;

use breakpoints_control_support::*;
use jcode_dap::{
    DebugContinueRequest, DebugRemoveBreakpointRequest, DebugSessionStateKind,
    DebugSetBreakpointRequest, DebugSourceBreakpoint, OwnerCleanupCause,
};
use tokio::time::{sleep, timeout};

async fn wait_breakpoint_count(
    manager: &jcode_dap::DebugSessionManager,
    id: jcode_dap::DebugSessionId,
    expected: usize,
) {
    timeout(Duration::from_secs(3), async {
        loop {
            if manager
                .breakpoints("owner", id)
                .is_ok_and(|snapshot| snapshot.total_breakpoints == expected)
            {
                break;
            }
            sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn subprocess_aborted_caller_still_reconciles_while_manager_lives() {
    let workspace = Workspace::new("abort-set");
    workspace.marker("hold-set-breakpoints");
    let source = workspace.source("abort.rs");
    let manager = manager(Duration::from_secs(2));
    let launched = launch_stopped(&workspace, &manager).await;
    let task = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    launched.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    workspace
        .wait_log("marker\twaiting-release-set-breakpoints")
        .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    workspace.marker("release-set-breakpoints");
    wait_breakpoint_count(&manager, launched.id, 1).await;
    let breakpoint_id =
        manager.breakpoints("owner", launched.id).unwrap().sources[0].breakpoints[0].id;
    std::fs::remove_file(workspace.root.join("release-set-breakpoints")).unwrap();
    let remove = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .remove_breakpoint(
                    "owner",
                    launched.id,
                    DebugRemoveBreakpointRequest::new(breakpoint_id),
                )
                .await
        })
    };
    workspace
        .wait_log_count("marker\twaiting-release-set-breakpoints", 2)
        .await;
    remove.abort();
    assert!(remove.await.unwrap_err().is_cancelled());
    workspace.marker("release-set-breakpoints");
    wait_breakpoint_count(&manager, launched.id, 0).await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}

#[tokio::test]
async fn subprocess_aborted_control_caller_still_updates_state() {
    let workspace = Workspace::new("abort-control");
    workspace.marker("hold-control");
    let manager = manager(Duration::from_secs(2));
    let launched = launch_stopped(&workspace, &manager).await;
    let task = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .continue_execution("owner", launched.id, DebugContinueRequest::default())
                .await
        })
    };
    workspace.wait_log("marker\twaiting-release-control").await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    workspace.marker("release-control");
    wait_state(&manager, launched.id, DebugSessionStateKind::Running).await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    manager.terminate("owner", launched.id).await.unwrap();
    wait_tree_gone(adapter_pid, None, None).await;
}

#[tokio::test]
async fn subprocess_final_manager_drop_closes_detached_operation_transport() {
    let workspace = Workspace::new("manager-drop");
    workspace.marker("hold-set-breakpoints");
    let source = workspace.source("drop.rs");
    let manager = manager(Duration::from_secs(5));
    let (attached, target_pid, descendant_pid) = attach_tree(&workspace, &manager).await;
    let task = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    attached.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    workspace
        .wait_log("marker\twaiting-release-set-breakpoints")
        .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let adapter_pid = workspace.logged_pid("adapter_pid");
    drop(manager);
    wait_tree_gone(adapter_pid, Some(target_pid), Some(descendant_pid)).await;
}

#[tokio::test]
async fn subprocess_owner_cleanup_during_pending_operation_removes_adapter_and_target_groups() {
    let workspace = Workspace::new("owner-cleanup");
    workspace.marker("hold-set-breakpoints");
    let source = workspace.source("cleanup.rs");
    let manager = manager(Duration::from_secs(5));
    let (attached, target_pid, descendant_pid) = attach_tree(&workspace, &manager).await;
    let pending = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    attached.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    workspace
        .wait_log("marker\twaiting-release-set-breakpoints")
        .await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    let report = manager
        .cleanup_owner("owner", OwnerCleanupCause::Disconnected)
        .await;
    assert_eq!(report.cleaned + report.already_ended, 1);
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.message.contains("disconnect"))
    );
    assert!(manager.sessions("owner").is_empty());
    assert!(pending.await.unwrap().is_err());
    wait_tree_gone(adapter_pid, Some(target_pid), Some(descendant_pid)).await;
}

#[tokio::test]
async fn subprocess_terminate_during_pending_operation_fences_late_commit() {
    let workspace = Workspace::new("terminate-pending");
    workspace.marker("hold-set-breakpoints");
    let source = workspace.source("terminate.rs");
    let manager = manager(Duration::from_secs(5));
    let (attached, target_pid, descendant_pid) = attach_tree(&workspace, &manager).await;
    let pending = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .set_breakpoint(
                    "owner",
                    attached.id,
                    DebugSetBreakpointRequest::new(source, DebugSourceBreakpoint::new(1)),
                )
                .await
        })
    };
    workspace
        .wait_log("marker\twaiting-release-set-breakpoints")
        .await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    let result = manager.terminate("owner", attached.id).await;
    assert!(
        result.is_err(),
        "blocked adapter must report disconnect timeout"
    );
    assert_eq!(
        manager.snapshot("owner", attached.id).unwrap().state.kind(),
        DebugSessionStateKind::Ended
    );
    assert!(pending.await.unwrap().is_err());
    assert_eq!(
        manager
            .breakpoints("owner", attached.id)
            .unwrap()
            .total_breakpoints,
        0
    );
    wait_tree_gone(adapter_pid, Some(target_pid), Some(descendant_pid)).await;
}

#[tokio::test]
async fn subprocess_shutdown_during_pending_operation_removes_adapter_and_target_groups() {
    let workspace = Workspace::new("shutdown-pending");
    workspace.marker("hold-control");
    let manager = manager(Duration::from_secs(5));
    let (attached, target_pid, descendant_pid) = attach_tree(&workspace, &manager).await;
    let pending = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .continue_execution("owner", attached.id, DebugContinueRequest::default())
                .await
        })
    };
    workspace.wait_log("marker\twaiting-release-control").await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    let report = manager.shutdown_all().await;
    assert_eq!(report.cleaned + report.already_ended, 1);
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.message.contains("disconnect"))
    );
    assert!(manager.sessions("owner").is_empty());
    assert!(pending.await.unwrap().is_err());
    wait_tree_gone(adapter_pid, Some(target_pid), Some(descendant_pid)).await;
}

#[tokio::test]
async fn subprocess_adapter_exit_during_control_finalizes_once_without_descendants() {
    let workspace = Workspace::new("adapter-exit");
    workspace.marker("exit-on-control");
    let manager = manager(Duration::from_secs(2));
    let (attached, target_pid, descendant_pid) = attach_tree(&workspace, &manager).await;
    let adapter_pid = workspace.logged_pid("adapter_pid");
    assert!(
        manager
            .continue_execution("owner", attached.id, DebugContinueRequest::default())
            .await
            .is_err()
    );
    wait_state(&manager, attached.id, DebugSessionStateKind::Ended).await;
    let first = manager.snapshot("owner", attached.id).unwrap();
    sleep(Duration::from_millis(20)).await;
    let second = manager.snapshot("owner", attached.id).unwrap();
    assert_eq!(first.state, second.state);
    wait_tree_gone(adapter_pid, Some(target_pid), Some(descendant_pid)).await;
}
