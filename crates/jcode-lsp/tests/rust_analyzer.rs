use std::path::Path;
use std::time::Duration;

use jcode_lsp::{
    DiagnosticSeverity, LspConfig, LspError, LspProcess, LspServerConfig, LspServicePool, Position,
    ProcessStatus, discover_executable,
};

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

#[tokio::test]
async fn shared_pool_reuses_one_server_but_isolates_worktree_identities() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("LSP crate should live below the workspace root");
    if discover_executable(
        "rust-analyzer",
        std::env::var_os("PATH").as_deref(),
        workspace,
    )
    .is_err()
    {
        return;
    }

    let pool = LspServicePool::new();
    let config = LspConfig::default();
    let first = pool
        .get_or_start(workspace, "worktree-a", "rust-analyzer", &config)
        .await
        .unwrap();
    let reused = pool
        .get_or_start(workspace, "worktree-a", "rust-analyzer", &config)
        .await
        .unwrap();
    let isolated = pool
        .get_or_start(workspace, "worktree-b", "rust-analyzer", &config)
        .await
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &reused));
    assert!(!std::sync::Arc::ptr_eq(&first, &isolated));
    assert_eq!(pool.len().await, 2);
    pool.shutdown_all(Duration::from_secs(5)).await;
    assert!(pool.is_empty().await);
}

#[tokio::test]
async fn shared_pool_reload_replaces_only_the_selected_workspace() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("LSP crate should live below the workspace root");
    if discover_executable(
        "rust-analyzer",
        std::env::var_os("PATH").as_deref(),
        workspace,
    )
    .is_err()
    {
        return;
    }
    let pool = LspServicePool::new();
    let config = LspConfig::default();
    let first = pool
        .get_or_start(workspace, "reload-worktree", "rust-analyzer", &config)
        .await
        .unwrap();
    let replacement = pool
        .reload(workspace, "reload-worktree", "rust-analyzer", &config)
        .await
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &replacement));
    assert_eq!(pool.len().await, 1);
    assert!(matches!(
        first.process().status().await.unwrap(),
        ProcessStatus::Exited { .. }
    ));
    pool.shutdown_all(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn rust_definition_and_introduced_error_are_observable() {
    let path = std::env::var_os("PATH");
    let current = std::env::current_dir().unwrap();
    if discover_executable("rust-analyzer", path.as_deref(), &current).is_err() {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let source_path = project.path().join("src/lib.rs");
    let source = "pub fn target() -> u32 { 1 }\npub fn caller() -> u32 { target() }\n";
    std::fs::write(&source_path, source).unwrap();

    let pool = LspServicePool::new();
    let config = LspConfig::default();
    let workspace = pool
        .get_or_start(project.path(), "fixture", "rust-analyzer", &config)
        .await
        .unwrap();
    let opened = workspace
        .sync_document(&source_path, "rust", source.to_owned())
        .await
        .unwrap();

    let target_column = source.lines().nth(1).unwrap().find("target").unwrap() as u32;
    let mut definition = serde_json::Value::Null;
    for _ in 0..40 {
        match workspace
            .definition(
                &source_path,
                Position {
                    line: 1,
                    character: target_column,
                },
            )
            .await
        {
            Ok(value) => definition = value,
            Err(LspError::Response { code: -32801, .. }) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Err(error) => panic!("definition request failed: {error}"),
        }
        if definition != serde_json::Value::Null
            && definition.as_array().is_none_or(|items| !items.is_empty())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_ne!(definition, serde_json::Value::Null);
    workspace
        .wait_for_diagnostics(&opened, Duration::from_secs(10))
        .await
        .expect("rust-analyzer should publish the initial diagnostic snapshot");

    let broken = "pub fn target() -> u32 { \"wrong\" }\npub fn caller() -> u32 { target() }\n";
    std::fs::write(&source_path, broken).unwrap();
    let document = workspace
        .sync_document_from_disk(&source_path, "rust")
        .await
        .unwrap();
    let diagnostics = workspace
        .current_diagnostics(&document, Duration::from_secs(10))
        .await
        .unwrap();
    let diagnostics = match diagnostics {
        Some(diagnostics) => diagnostics,
        None => {
            let cached = workspace.diagnostics(&source_path).await.unwrap();
            panic!(
                "rust-analyzer did not publish fresh diagnostics; capabilities: {}; cached: {cached:?}; stderr: {}",
                workspace.capabilities(),
                workspace.process().recent_stderr()
            );
        }
    };
    assert!(
        diagnostics
            .items
            .iter()
            .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)),
        "expected an introduced error, got {:?}",
        diagnostics.items
    );

    pool.shutdown_all(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn rust_cross_file_rename_returns_edits_for_definition_and_reference() {
    let path = std::env::var_os("PATH");
    let current = std::env::current_dir().unwrap();
    if discover_executable("rust-analyzer", path.as_deref(), &current).is_err() {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"rename-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let lib_path = project.path().join("src/lib.rs");
    let other_path = project.path().join("src/other.rs");
    let lib = "mod other;\npub fn call() -> u32 { other::target() }\n";
    let other = "pub fn target() -> u32 { 1 }\n";
    std::fs::write(&lib_path, lib).unwrap();
    std::fs::write(&other_path, other).unwrap();

    let pool = LspServicePool::new();
    let config = LspConfig::default();
    let workspace = pool
        .get_or_start(project.path(), "rename", "rust-analyzer", &config)
        .await
        .unwrap();
    workspace
        .sync_document_from_disk(&lib_path, "rust")
        .await
        .unwrap();
    workspace
        .sync_document_from_disk(&other_path, "rust")
        .await
        .unwrap();
    let reference_column = lib.lines().nth(1).unwrap().find("target").unwrap() as u32;
    workspace
        .definition(
            &lib_path,
            Position {
                line: 1,
                character: reference_column,
            },
        )
        .await
        .unwrap();

    let declaration_column = other.find("target").unwrap() as u32;
    let position = Position {
        line: 0,
        character: declaration_column,
    };
    assert!(
        !workspace
            .prepare_rename(&other_path, position)
            .await
            .unwrap()
            .is_null()
    );
    let edit = workspace
        .rename(&other_path, position, "renamed_target")
        .await
        .unwrap();
    let mut touched = std::collections::HashSet::new();
    if let Some(changes) = edit.get("changes").and_then(serde_json::Value::as_object) {
        touched.extend(changes.keys().cloned());
    }
    if let Some(changes) = edit
        .get("documentChanges")
        .and_then(serde_json::Value::as_array)
    {
        touched.extend(changes.iter().filter_map(|change| {
            change
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }));
    }
    assert_eq!(touched.len(), 2, "workspace edit: {edit}");
    pool.shutdown_all(Duration::from_secs(5)).await;
}
