use super::*;
use crate::message::{Message, ToolDefinition};
use crate::provider::{EventStream, Provider};

struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        Err(anyhow!("mock provider should not be called"))
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self)
    }
}

#[test]
fn converts_one_based_agent_positions_to_lsp_positions() {
    assert_eq!(
        one_based_position(Some(42), Some(5)).unwrap(),
        Position {
            line: 41,
            character: 4
        }
    );
    assert!(one_based_position(Some(0), Some(1)).is_err());
    assert!(one_based_position(Some(1), Some(0)).is_err());
}

#[tokio::test]
async fn missing_executable_is_a_graceful_status() {
    let config = LspConfig {
        servers: std::collections::BTreeMap::from([(
            "missing".to_owned(),
            jcode_lsp::LspServerConfig {
                command: "jcode-definitely-missing-lsp".to_owned(),
                ..Default::default()
            },
        )]),
        ..LspConfig::default()
    };
    let tool = LspTool::with_config(Arc::new(LspServicePool::new()), config);
    let output = render_status(
        &tool.config(),
        &std::env::current_dir().unwrap(),
        "status-test",
        &tool.pool,
    )
    .await;
    assert!(output.output.contains("missing"));
    assert_eq!(output.metadata.unwrap()["servers"][0]["status"], "missing");
}

#[test]
fn bounded_tail_retains_only_the_newest_characters() {
    assert_eq!(bounded_tail("abcdef", 4), "cdef");
    assert_eq!(bounded_tail("a😀bc", 3), "😀bc");
}

#[test]
fn non_shared_configuration_isolates_session_workspace_identity() {
    let root = Path::new("/workspace");
    let shared = LspConfig::default();
    assert_eq!(workspace_identity(&shared, root, "session-a"), "/workspace");
    assert_eq!(
        workspace_identity(&shared, root, "session-a"),
        workspace_identity(&shared, root, "session-b")
    );

    let isolated = LspConfig {
        shared: false,
        ..LspConfig::default()
    };
    assert_eq!(
        workspace_identity(&isolated, root, "session-a"),
        "/workspace#session=session-a"
    );
    assert_ne!(
        workspace_identity(&isolated, root, "session-a"),
        workspace_identity(&isolated, root, "session-b")
    );
}

#[test]
fn renders_locations_without_raw_protocol_payloads() {
    let root = Path::new("/workspace");
    let value = json!([{
        "uri": "file:///workspace/src/lib.rs",
        "range": {"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 8}}
    }]);
    assert_eq!(
        render_locations(&value, root),
        ("src/lib.rs:5:3".to_owned(), 1)
    );
}

#[test]
fn every_builtin_mutation_path_receives_post_edit_feedback() {
    for tool in [
        "write",
        "edit",
        "multiedit",
        "patch",
        "apply_patch",
        "anchored_edit",
    ] {
        assert!(is_post_edit_tool(tool), "missing post-edit path: {tool}");
    }
    for tool in ["read", "bash", "lsp"] {
        assert!(!is_post_edit_tool(tool));
    }
}

#[test]
fn rename_preview_summarizes_workspace_edits_without_dumping_protocol_json() {
    let edit = json!({
        "changes": {
            "file:///workspace/src/lib.rs": [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                "newText": "renamed"
            }],
            "file:///workspace/src/other.rs": [{
                "range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 7}},
                "newText": "renamed"
            }]
        }
    });
    let (text, files, count) = render_workspace_edit(&edit, Path::new("/workspace")).unwrap();
    assert!(text.contains("src/lib.rs: 1 edit(s)"));
    assert!(text.contains("src/other.rs: 1 edit(s)"));
    assert!(!text.contains("newText"));
    assert_eq!(files.len(), 2);
    assert_eq!(count, 2);
}

#[test]
fn code_actions_require_an_explicit_unique_enabled_selector() {
    let actions = json!([
        {"title": "Import Arc", "kind": "quickfix", "edit": {"changes": {}}},
        {"title": "Disabled", "disabled": {"reason": "not applicable"}},
        {"title": "Duplicate"},
        {"title": "Duplicate"}
    ]);
    assert_eq!(
        select_code_action(&actions, "Import Arc").unwrap()["title"],
        "Import Arc"
    );
    assert_eq!(
        select_code_action(&actions, "#1").unwrap()["title"],
        "Import Arc"
    );
    assert!(select_code_action(&actions, "Disabled").is_err());
    assert!(select_code_action(&actions, "Duplicate").is_err());
    assert!(select_code_action(&actions, "99").is_err());

    let (text, summaries) = render_code_actions(&actions);
    assert!(text.contains("1. Import Arc [quickfix]"));
    assert_eq!(summaries.len(), 4);
    assert_eq!(summaries[0]["title"], "Import Arc");
}

#[test]
fn expanded_read_actions_are_advertised_and_shaped_without_raw_payloads() {
    let tool = LspTool::with_config(Arc::new(LspServicePool::new()), LspConfig::default());
    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    for action in [
        "implementation",
        "type_definition",
        "signature_help",
        "incoming_calls",
        "outgoing_calls",
        "code_actions",
    ] {
        assert!(
            actions.iter().any(|value| value == action),
            "missing {action}"
        );
    }

    assert_eq!(
        render_signature_help(&json!({"signatures": [{"label": "fn run(value: u32)"}]})),
        ("fn run(value: u32)".to_owned(), 1)
    );
    let (text, actions) =
        render_code_actions(&json!([{"title": "Import item", "kind": "quickfix"}]));
    assert_eq!(text, "1. Import item [quickfix]");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Import item");
    let calls = json!([{"from": {"name": "caller", "uri": "file:///workspace/src/lib.rs"}}]);
    assert_eq!(
        render_call_hierarchy(&calls, Path::new("/workspace"), LspAction::IncomingCalls),
        ("caller — src/lib.rs".to_owned(), 1)
    );
}

#[tokio::test]
async fn service_registry_exposes_lsp_without_starting_a_process() {
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let pool = Arc::new(LspServicePool::new());
    let registry = super::super::Registry::new_with_services(
        provider,
        crate::server::FileSnapshotLedger::new(),
        Arc::clone(&pool),
    )
    .await;
    assert!(registry.tool_names().await.iter().any(|name| name == "lsp"));
    assert!(pool.is_empty().await);
}

#[tokio::test]
async fn new_without_server_services_does_not_expose_lsp() {
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = super::super::Registry::new(provider).await;
    assert!(!registry.tool_names().await.iter().any(|name| name == "lsp"));
}

#[tokio::test]
async fn installed_rust_analyzer_definition_flows_through_the_tool_contract() {
    let current = std::env::current_dir().unwrap();
    if discover_executable(
        "rust-analyzer",
        std::env::var_os("PATH").as_deref(),
        &current,
    )
    .is_err()
    {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"tool-lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let source = "pub fn target() {}\npub fn caller() { target(); }\n";
    std::fs::write(project.path().join("src/lib.rs"), source).unwrap();
    let character = source.lines().nth(1).unwrap().find("target").unwrap() as u32 + 1;

    let pool = Arc::new(LspServicePool::new());
    let tool = LspTool::with_config(Arc::clone(&pool), LspConfig::default());
    let output = tool
        .execute(
            json!({
                "action": "definition",
                "file": "src/lib.rs",
                "line": 2,
                "character": character,
                "intent": "Verify the public LSP tool path"
            }),
            ToolContext {
                session_id: "lsp-test".to_owned(),
                message_id: "message".to_owned(),
                tool_call_id: "call".to_owned(),
                working_dir: Some(project.path().to_owned()),
                stdin_request_tx: None,
                graceful_shutdown_signal: None,
                execution_mode: super::super::ToolExecutionMode::Direct,
            },
        )
        .await
        .unwrap();
    assert!(output.output.contains("src/lib.rs:1:"), "{}", output.output);
    assert_eq!(output.metadata.as_ref().unwrap()["freshness"], "fresh");
    pool.shutdown_all(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn write_returns_introduced_rust_error_on_the_same_tool_result() {
    let current = std::env::current_dir().unwrap();
    if discover_executable(
        "rust-analyzer",
        std::env::var_os("PATH").as_deref(),
        &current,
    )
    .is_err()
    {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"write-lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/lib.rs"),
        "pub fn value() -> u32 { 1 }\n",
    )
    .unwrap();

    let pool = Arc::new(LspServicePool::new());
    let config = LspConfig::default();
    let workspace = pool
        .get_or_start(
            project.path(),
            project.path().canonicalize().unwrap().display().to_string(),
            "rust-analyzer",
            &config,
        )
        .await
        .unwrap();
    let opened = workspace
        .sync_document_from_disk(&project.path().join("src/lib.rs"), "rust")
        .await
        .unwrap();
    workspace
        .current_diagnostics(&opened, Duration::from_secs(2))
        .await
        .unwrap();
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = super::super::Registry::new_with_services(
        provider,
        crate::server::FileSnapshotLedger::new(),
        Arc::clone(&pool),
    )
    .await;
    let output = registry
        .execute(
            "write",
            json!({
                "file_path": "src/lib.rs",
                "content": "pub fn value() -> u32 { \"wrong\" }\n",
                "intent": "Introduce a deterministic type error"
            }),
            ToolContext {
                session_id: "lsp-write-test".to_owned(),
                message_id: "message".to_owned(),
                tool_call_id: "write-call".to_owned(),
                working_dir: Some(project.path().to_owned()),
                stdin_request_tx: None,
                graceful_shutdown_signal: None,
                execution_mode: super::super::ToolExecutionMode::Direct,
            },
        )
        .await
        .unwrap();
    assert!(
        output.output.contains("Diagnostics delta after edit"),
        "{}\nmetadata: {:?}",
        output.output,
        output.metadata
    );
    assert_eq!(
        output.metadata.as_ref().unwrap()["semantic_verification"]["status"],
        "issues_found"
    );
    pool.shutdown_all(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn read_then_lsp_apply_performs_atomic_cross_file_rename() {
    let current = std::env::current_dir().unwrap();
    if discover_executable(
        "rust-analyzer",
        std::env::var_os("PATH").as_deref(),
        &current,
    )
    .is_err()
    {
        return;
    }

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"tool-rename-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let lib = "mod other;\npub fn call() -> u32 { other::target() }\n";
    let other = "pub fn target() -> u32 { 1 }\n";
    std::fs::write(project.path().join("src/lib.rs"), lib).unwrap();
    std::fs::write(project.path().join("src/other.rs"), other).unwrap();

    let pool = Arc::new(LspServicePool::new());
    let ledger = crate::server::FileSnapshotLedger::new();
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry =
        super::super::Registry::new_with_services(provider, ledger, Arc::clone(&pool)).await;
    let context = |call: &str| ToolContext {
        session_id: "lsp-rename-test".to_owned(),
        message_id: "message".to_owned(),
        tool_call_id: call.to_owned(),
        working_dir: Some(project.path().to_owned()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: super::super::ToolExecutionMode::Direct,
    };
    for file in ["src/lib.rs", "src/other.rs"] {
        registry
            .execute(
                "read",
                json!({"file_path": file, "intent": "Read before semantic rename"}),
                context(&format!("read-{file}")),
            )
            .await
            .unwrap();
    }

    let output = registry
        .execute(
            "lsp",
            json!({
                "action": "rename",
                "file": "src/other.rs",
                "line": 1,
                "character": other.find("target").unwrap() + 1,
                "new_name": "renamed_target",
                "apply": true,
                "intent": "Apply a read-guarded cross-file rename"
            }),
            context("rename"),
        )
        .await
        .unwrap();
    assert!(
        output.output.contains("Applied semantic rename"),
        "{}",
        output.output
    );
    assert!(
        std::fs::read_to_string(project.path().join("src/lib.rs"))
            .unwrap()
            .contains("renamed_target")
    );
    assert!(
        std::fs::read_to_string(project.path().join("src/other.rs"))
            .unwrap()
            .contains("renamed_target")
    );
    assert_eq!(output.metadata.as_ref().unwrap()["rename_applied"], true);
    assert_eq!(
        output.metadata.as_ref().unwrap()["files"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    pool.shutdown_all(Duration::from_secs(5)).await;
}
