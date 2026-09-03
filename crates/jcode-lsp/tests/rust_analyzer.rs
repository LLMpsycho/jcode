use std::path::Path;
use std::time::Duration;

use jcode_lsp::{LspProcess, LspServerConfig, ProcessStatus, discover_executable};

#[tokio::test]
async fn installed_rust_analyzer_initializes_and_shuts_down() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("LSP crate should live below the workspace root");
    let config = LspServerConfig {
        command: "rust-analyzer".to_owned(),
        args: Vec::new(),
        root_markers: vec!["Cargo.toml".to_owned()],
        file_extensions: vec!["rs".to_owned()],
    };
    if discover_executable(
        &config.command,
        std::env::var_os("PATH").as_deref(),
        workspace,
    )
    .is_err()
    {
        return;
    }

    let process = LspProcess::spawn(&config, workspace)
        .await
        .expect("installed rust-analyzer should spawn");
    let response = process
        .initialize(workspace, Duration::from_secs(20))
        .await
        .unwrap_or_else(|error| {
            panic!(
                "rust-analyzer initialization failed: {error}; stderr: {}",
                process.recent_stderr()
            )
        });
    assert!(response.get("capabilities").is_some());
    assert_eq!(process.status().await.unwrap(), ProcessStatus::Running);
    process.shutdown(Duration::from_secs(5)).await;
    assert!(matches!(
        process.status().await.unwrap(),
        ProcessStatus::Exited { .. }
    ));
}
