#[tokio::test]
async fn mcp_exposure_modes_select_eager_or_fixed_definitions() {
    let _guard = crate::storage::lock_test_env();

    let mut eager = agent_with_fake_mcp_surface(crate::config::McpToolsMode::Eager, 0).await;
    let eager_names: Vec<String> = eager
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(eager_names.iter().any(|name| name == "mcp__test__verbose"));
    assert!(!eager_names.iter().any(|name| name == "mcp_search"));
    assert!(!eager_names.iter().any(|name| name == "mcp_call"));

    let mut deferred =
        agent_with_fake_mcp_surface(crate::config::McpToolsMode::Deferred, usize::MAX).await;
    let deferred_names: Vec<String> = deferred
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(!deferred_names.iter().any(|name| name.starts_with("mcp__")));
    assert!(deferred_names.iter().any(|name| name == "mcp_search"));
    assert!(deferred_names.iter().any(|name| name == "mcp_call"));

    let mut auto_eager =
        agent_with_fake_mcp_surface(crate::config::McpToolsMode::Auto, usize::MAX).await;
    let auto_eager_names: Vec<String> = auto_eager
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        auto_eager_names
            .iter()
            .any(|name| name == "mcp__test__verbose")
    );

    let mut auto_deferred = agent_with_fake_mcp_surface(crate::config::McpToolsMode::Auto, 1).await;
    let auto_deferred_names: Vec<String> = auto_deferred
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        !auto_deferred_names
            .iter()
            .any(|name| name.starts_with("mcp__"))
    );
    assert!(auto_deferred_names.iter().any(|name| name == "mcp_search"));
    assert!(auto_deferred_names.iter().any(|name| name == "mcp_call"));
    let stable_auto_names: Vec<String> = auto_deferred
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(auto_deferred_names, stable_auto_names);
    assert!(auto_deferred.mcp_late_register_resolved);
}
#[tokio::test]
async fn deferred_mcp_surface_ignores_late_per_tool_registration() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    register_fake_deferred_mcp_surface(&registry).await;
    let mut agent = Agent::new(provider, registry);
    agent.mcp_tools_mode = crate::config::McpToolsMode::Deferred;

    let before: Vec<String> = agent
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    agent
        .registry
        .register(
            "mcp__late__tool".to_string(),
            Arc::new(FakeMcpTool {
                name: "late".to_string(),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;
    let after: Vec<String> = agent
        .tool_definitions()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect();

    assert_eq!(
        before, after,
        "fixed deferred surface must stay cache-stable"
    );
    assert!(agent.mcp_late_register_resolved);
    assert!(!after.iter().any(|name| name.starts_with("mcp__")));
}
#[tokio::test]
async fn auto_mode_rechecks_late_mcp_definitions_before_deferring() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    register_fake_deferred_mcp_surface(&registry).await;
    let mut agent = Agent::new(provider, registry);
    agent.mcp_tools_mode = crate::config::McpToolsMode::Auto;
    agent.mcp_tools_token_threshold = 1;

    let before = agent.tool_definitions().await;
    assert!(!before.iter().any(|tool| tool.name == "mcp_search"));
    agent
        .registry
        .register(
            "mcp__late__large".to_string(),
            Arc::new(VerboseFakeMcpTool {
                name: "large".to_string(),
                description: "late large definition ".repeat(32),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;

    let after = agent.tool_definitions().await;
    assert!(after.iter().any(|tool| tool.name == "mcp_search"));
    assert!(after.iter().any(|tool| tool.name == "mcp_call"));
    assert!(!after.iter().any(|tool| tool.name.starts_with("mcp__")));
    assert!(agent.mcp_late_register_resolved);
}
/// Reproduction for #206: MCP tools that register on the registry *after* the
/// first turn locks the tool snapshot never reach the provider, because
/// `tool_definitions()` returns the frozen `locked_tools` snapshot and the only
/// unlock path (`unlock_tools_if_needed`) fires solely when the LLM invokes the
/// `"mcp"` management tool — which it never does, since it cannot see the
/// `mcp__*` tools it would need to trigger that unlock.
#[tokio::test]
async fn mcp_tools_registered_after_lock_are_visible_to_agent() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);

    // First turn locks the snapshot (this is what happens before the async MCP
    // registration spawn completes).
    let before = agent.tool_definitions().await;
    let before_len = before.len();
    assert!(
        !before.iter().any(|t| t.name.starts_with("mcp__")),
        "precondition: no mcp tools before async registration completes"
    );

    // Simulate the spawned MCP registration task finishing: a new mcp__* tool
    // lands on the shared registry.
    agent
        .registry
        .register(
            "mcp__test__write_memory".to_string(),
            Arc::new(FakeMcpTool {
                name: "mcp__test__write_memory".to_string(),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;

    // The next turn should now advertise the MCP tool to the provider.
    let after = agent.tool_definitions().await;
    assert!(
        after.iter().any(|t| t.name == "mcp__test__write_memory"),
        "regression #206: MCP tool registered after the first turn never reaches \
         the agent's tool surface (locked snapshot of {} tools is reused forever)",
        before_len
    );

    // Once MCP tools are present in the locked snapshot, subsequent turns must
    // return the *same* stable snapshot so provider prompt-cache hits stay warm
    // (the whole point of locked_tools). The #206 fix must not flap.
    let names =
        |defs: &[ToolDefinition]| -> Vec<String> { defs.iter().map(|t| t.name.clone()).collect() };
    let stable_a = agent.tool_definitions().await;
    let stable_b = agent.tool_definitions().await;
    assert_eq!(
        names(&stable_a),
        names(&stable_b),
        "tool snapshot must be stable across turns once MCP tools are present"
    );
    assert_eq!(
        names(&stable_a),
        names(&after),
        "snapshot must not change after MCP tools are already included"
    );
}
/// The intentional, MCP-driven prompt-cache miss must happen at most ONCE per
/// locked snapshot. After the first late-registered `mcp__*` tool is picked up
/// (the one accepted miss), a *second* MCP tool that registers even later must
/// NOT trigger another rebuild — otherwise a server that connects in waves would
/// thrash the provider prompt cache. Guards the `mcp_late_register_resolved`
/// one-shot flag (#206 follow-up).
#[tokio::test]
async fn mcp_late_registration_rebuild_happens_at_most_once() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);

    // First turn locks the snapshot with no MCP tools yet.
    let _ = agent.tool_definitions().await;

    // First MCP tool arrives -> one accepted rebuild exposes it.
    agent
        .registry
        .register(
            "mcp__test__first".to_string(),
            Arc::new(FakeMcpTool {
                name: "mcp__test__first".to_string(),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;
    let after_first = agent.tool_definitions().await;
    assert!(
        after_first.iter().any(|t| t.name == "mcp__test__first"),
        "first late MCP tool must be picked up by the one accepted rebuild"
    );
    assert!(
        agent.mcp_late_register_resolved,
        "one-shot guard must latch after the accepted rebuild"
    );

    // A SECOND MCP tool registers even later (server connected in a second
    // wave). The one-shot guard means we do NOT rebuild again, so the snapshot
    // stays cache-stable and this tool is intentionally not surfaced until the
    // tool list is explicitly unlocked.
    agent
        .registry
        .register(
            "mcp__test__second".to_string(),
            Arc::new(FakeMcpTool {
                name: "mcp__test__second".to_string(),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;
    let after_second = agent.tool_definitions().await;
    let names: Vec<String> = after_second.iter().map(|t| t.name.clone()).collect();
    assert!(
        names.iter().any(|n| n == "mcp__test__first"),
        "previously surfaced MCP tool must remain"
    );
    assert!(
        !names.iter().any(|n| n == "mcp__test__second"),
        "second-wave MCP tool must NOT trigger a second cache-busting rebuild"
    );

    // An explicit unlock (e.g. the `mcp` reload tool) re-arms the one-shot guard
    // and lets the next snapshot pick up everything currently registered.
    agent.unlock_tools();
    assert!(
        !agent.mcp_late_register_resolved,
        "explicit unlock must re-arm the one-shot guard"
    );
    let after_unlock = agent.tool_definitions().await;
    let unlocked_names: Vec<String> = after_unlock.iter().map(|t| t.name.clone()).collect();
    assert!(
        unlocked_names.iter().any(|n| n == "mcp__test__second"),
        "after explicit unlock, the second-wave MCP tool must finally surface"
    );
}
/// Without any newly-registered MCP tools, the locked snapshot must be returned
/// verbatim on every turn (no rebuild, no cache invalidation). Guards the #206
/// fix against re-snapshotting on turns where nothing changed.
#[tokio::test]
async fn tool_snapshot_is_stable_without_new_mcp_tools() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);

    let first = agent.tool_definitions().await;
    // Register a NON-mcp tool after locking — this should NOT trigger a rebuild,
    // because the cache-stability optimization only yields to MCP arrival.
    agent
        .registry
        .register(
            "not_an_mcp_tool".to_string(),
            Arc::new(FakeMcpTool {
                name: "not_an_mcp_tool".to_string(),
            }) as Arc<dyn crate::tool::Tool>,
        )
        .await;
    let second = agent.tool_definitions().await;
    let first_names: Vec<String> = first.iter().map(|t| t.name.clone()).collect();
    let second_names: Vec<String> = second.iter().map(|t| t.name.clone()).collect();
    assert_eq!(
        first_names, second_names,
        "non-MCP registry changes must not invalidate the locked tool snapshot"
    );
    assert!(
        !second_names.iter().any(|n| n == "not_an_mcp_tool"),
        "non-MCP tool registered after lock must not leak into the snapshot"
    );
}
#[test]
fn empty_post_tool_response_gets_more_than_one_retry() {
    // Regression guard for the Claude Opus 5 benchmark incident. A provider can
    // return an empty response immediately after tool results; that is a
    // transient hiccup, not a finished task. With only one retry allowed, a
    // single empty response (observed once in 43 turns) ended a 20-hour agent
    // run with the work half-done and the submission unoptimized.
    assert!(
        Agent::MAX_EMPTY_POST_TOOL_CONTINUATION_ATTEMPTS > 1,
        "a single retry lets one transient empty response end a long run"
    );
    // Bounded, so a genuinely finished agent still exits instead of looping.
    assert!(Agent::MAX_EMPTY_POST_TOOL_CONTINUATION_ATTEMPTS <= 10);
}
