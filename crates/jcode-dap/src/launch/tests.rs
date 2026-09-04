use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::json;

use super::*;
use crate::manager::{NewDebugSession, start_protocol};
use crate::testing::FakeAdapter;
use crate::{Capabilities, DebugSessionManager, DebugSessionManagerConfig, Message};

fn workspace(name: &str) -> (PathBuf, DebugWorkspaceKey, PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("jcode-launch-{name}-{stamp}"));
    fs::create_dir_all(&root).unwrap();
    let program = root.join("program");
    fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
    (
        root.clone(),
        DebugWorkspaceKey::new(&root, name).unwrap(),
        program,
    )
}

#[test]
fn launch_arguments_use_canonical_workspace_paths_and_literal_args() {
    let (root, workspace, program) = workspace("literal");
    let resolved = resolve_program(
        &workspace,
        Path::new("program"),
        &["$HOME".to_owned(), "a b".to_owned()],
        None,
    )
    .unwrap();
    let value = AdapterProfile::LldbDap.launch_arguments(&ResolvedLaunch {
        target: resolved,
        stop_on_entry: false,
    });
    assert_eq!(value["program"], json!(program.canonicalize().unwrap()));
    assert_eq!(value["cwd"], json!(root.canonicalize().unwrap()));
    assert_eq!(value["args"], json!(["$HOME", "a b"]));
}

#[test]
fn initialize_advertises_no_run_in_terminal_support() {
    let value = serde_json::to_value(AdapterProfile::LldbDap.initialize_arguments()).unwrap();
    assert_eq!(value["supportsRunInTerminalRequest"], false);
    assert_eq!(value["adapterID"], "lldb");
}

#[test]
fn gdb_profile_uses_native_dap_interpreter_and_adapter_id() {
    assert_eq!(
        AdapterProfile::GdbDap.command_arguments(),
        &["--interpreter=dap"]
    );
    let value = serde_json::to_value(AdapterProfile::GdbDap.initialize_arguments()).unwrap();
    assert_eq!(value["adapterID"], "gdb");
    assert_eq!(
        AdapterProfile::GdbDap.attach_arguments(42),
        json!({"pid":42})
    );
}

#[test]
fn initialize_arguments_advertise_phase_30f_client_capabilities() {
    let arguments = AdapterProfile::LldbDap.initialize_arguments();
    assert_eq!(arguments.supports_variable_type, Some(true));
    assert_eq!(arguments.supports_variable_paging, Some(true));
    assert_eq!(arguments.supports_run_in_terminal_request, Some(false));
}

#[tokio::test]
async fn launch_sends_initialize_launch_configuration_done_in_protocol_order() {
    let (_, workspace, program) = workspace("order");
    let manager = DebugSessionManager::new(DebugSessionManagerConfig::default()).unwrap();
    let (client, mut adapter) = FakeAdapter::pair(1024 * 1024);
    let mut reservation = manager
        .reserve(NewDebugSession {
            owner_session_id: "owner".to_owned(),
            workspace: workspace.clone(),
            adapter_id: "lldb".to_owned(),
            start: Some(DebugSessionStart::Launch {
                program: program.clone(),
                cwd: workspace.canonical_root().to_owned(),
            }),
        })
        .unwrap();
    reservation.attach_client(client).unwrap();
    let resolved = ResolvedLaunch {
        target: resolve_program(&workspace, &program, &[], None).unwrap(),
        stop_on_entry: false,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let task = tokio::spawn(async move {
        start_protocol(
            &reservation,
            AdapterProfile::LldbDap,
            "launch",
            AdapterProfile::LldbDap.launch_arguments(&resolved),
            deadline,
        )
        .await
    });
    let initialize = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    assert_eq!(initialize.command, "initialize");
    adapter
        .respond_ok(
            &initialize,
            Some(
                serde_json::to_value(Capabilities {
                    supports_configuration_done_request: Some(true),
                    ..Default::default()
                })
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let launch = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    assert_eq!(launch.command, "launch");
    adapter.event("initialized", None).await.unwrap();
    let configuration = match adapter.recv().await.unwrap() {
        Message::Request(request) => request,
        _ => panic!(),
    };
    assert_eq!(configuration.command, "configurationDone");
    adapter.respond_ok(&configuration, None).await.unwrap();
    adapter.respond_ok(&launch, None).await.unwrap();
    task.await.unwrap().unwrap();
}

#[test]
fn program_symlink_escape_is_rejected_before_adapter_spawn() {
    let (root, workspace, _) = workspace("escape");
    let outside = std::env::current_exe().unwrap();
    let link = root.join("escape");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside, &link).unwrap();
    assert!(matches!(
        resolve_program(&workspace, &link, &[], None),
        Err(DapError::DebugPathOutsideWorkspace { .. })
    ));
}

#[test]
fn owned_attach_pid_is_not_request_input_data() {
    let request = DebugOwnedAttachRequest::new("program").with_arg("1234");
    assert_eq!(request.args(), &["1234"]);
}
