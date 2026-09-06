use super::*;

fn profile(tools: &[&str]) -> crate::session::SessionAgentProfile {
    crate::session::SessionAgentProfile {
        name: "debug".into(),
        content: "PROFILE_SENTINEL: diagnose before changing code".into(),
        allowed_tools: Some(tools.iter().map(|name| (*name).into()).collect()),
    }
}

#[tokio::test]
async fn agent_profile_keeps_normal_prompt_and_enforces_policy_through_restore_and_clear() {
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let session = Session::create_with_id(
        "session_agent_profile_restore".into(),
        None,
        Some("worker".into()),
    );
    let mut agent = Agent::new_with_session(
        provider,
        registry.clone(),
        session,
        Some(HashSet::from([
            "read".into(),
            "edit".into(),
            "batch".into(),
        ])),
    );
    let baseline_prompt = agent.build_system_prompt_split(None).static_part;
    let baseline_model = agent.provider_model();
    let id = agent.session_id().to_string();
    agent.disabled_tools.insert("edit".into());
    agent
        .set_swarm_identity(
            Some(profile(&["read", "edit", "bash", "batch"])),
            Some("debug agent".into()),
        )
        .unwrap();
    assert_eq!(agent.session_id(), id);
    assert_eq!(agent.session_short_name(), Some("debug agent"));
    assert_eq!(agent.provider_model(), baseline_model);
    let prompt = agent.build_system_prompt_split(None).static_part;
    assert!(prompt.starts_with(&baseline_prompt));
    assert!(prompt.contains("PROFILE_SENTINEL"));
    assert!(prompt.contains("coordinator receives your final response automatically"));
    assert!(agent.validate_tool_allowed("read").is_ok());
    for tool in ["bash", "edit", "swarm"] {
        assert!(agent.validate_tool_allowed(tool).is_err());
    }
    let ctx = crate::tool::ToolContext {
        session_id: id.clone(),
        message_id: "profile".into(),
        tool_call_id: "profile".into(),
        working_dir: None,
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    };
    assert!(
        registry
            .execute(
                "shell_exec",
                serde_json::json!({"command":"true"}),
                ctx.clone()
            )
            .await
            .is_err()
    );
    let nested = registry.execute("batch", serde_json::json!({"tool_calls":[{"tool":"bash","intent":"must be blocked","command":"true"}]}), ctx).await.unwrap();
    assert!(nested.output.contains("not allowed"), "{}", nested.output);
    agent.clear();
    assert!(agent.session.agent_profile.is_none());
    assert!(
        !agent
            .build_system_prompt_split(None)
            .static_part
            .contains("PROFILE_SENTINEL")
    );
    agent.restore_session(&id).unwrap();
    assert_eq!(agent.session_short_name(), Some("debug agent"));
    assert!(agent.validate_tool_allowed("bash").is_err());
    assert!(
        agent
            .build_system_prompt_split(None)
            .static_part
            .contains("PROFILE_SENTINEL")
    );
    agent
        .set_swarm_identity(Some(profile(&[])), Some("empty agent".into()))
        .unwrap();
    assert!(agent.validate_tool_allowed("read").is_err());
    agent.clear();
    assert!(
        agent.validate_tool_allowed("read").is_ok(),
        "clear restores baseline rather than previous profile limits"
    );
}

#[tokio::test]
async fn agent_profile_never_expands_mcp_dispatch_permissions() {
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new_with_session(
        provider,
        registry,
        Session::create(None, Some("profile MCP".into())),
        Some(HashSet::from(["mcp_call".into(), "mcp__allowed".into()])),
    );
    agent
        .set_swarm_identity(Some(profile(&["mcp__allowed"])), Some("debug agent".into()))
        .unwrap();
    assert!(!agent.allowed_tools.as_ref().unwrap().contains("mcp_call"));
    assert!(crate::tool::session_mcp_dispatch_is_allowed(
        agent.session_id(),
        "mcp__allowed",
        "mcp_call"
    ));
    assert!(!crate::tool::session_mcp_dispatch_is_allowed(
        agent.session_id(),
        "mcp__other",
        "mcp_call"
    ));
}
