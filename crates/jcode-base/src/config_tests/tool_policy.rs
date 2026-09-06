#[test]
fn tool_config_defaults_to_full_toolset() {
    let config = ToolConfig::default();
    let selection = config.selection();
    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.is_empty());
    assert_eq!(config.mcp_tools, McpToolsMode::Auto);
    assert_eq!(config.mcp_tools_token_threshold, 8_000);
}
#[test]
fn editing_read_guard_defaults_to_warn_and_deserializes_all_modes() {
    let default = Config::default();
    assert_eq!(default.editing.read_guard.mode, ReadGuardMode::Warn);
    assert!(default.editing.read_guard.require_same_revision);
    assert!(default.editing.read_guard.require_covered_ranges);
    assert!(!default.editing.read_guard.allow_full_file_write);

    for (raw, expected) in [
        ("off", ReadGuardMode::Off),
        ("warn", ReadGuardMode::Warn),
        ("block", ReadGuardMode::Block),
    ] {
        let config: Config = toml::from_str(&format!("[editing.read_guard]\nmode = \"{raw}\"\n"))
            .expect("valid read guard mode");
        assert_eq!(config.editing.read_guard.mode, expected);
    }
}
#[test]
fn tool_config_deserializes_all_mcp_exposure_modes() {
    for (raw, expected) in [
        ("auto", McpToolsMode::Auto),
        ("eager", McpToolsMode::Eager),
        ("deferred", McpToolsMode::Deferred),
    ] {
        let config: Config = toml::from_str(&format!("[tools]\nmcp_tools = \"{raw}\"\n"))
            .expect("valid MCP exposure mode");
        assert_eq!(config.tools.mcp_tools, expected);
    }
}
#[test]
fn tool_config_mcp_exposure_env_overrides() {
    let _guard = crate::storage::lock_test_env();
    let previous_mode = std::env::var_os("JCODE_MCP_TOOLS");
    let previous_threshold = std::env::var_os("JCODE_MCP_TOOLS_TOKEN_THRESHOLD");
    crate::env::set_var("JCODE_MCP_TOOLS", "deferred");
    crate::env::set_var("JCODE_MCP_TOOLS_TOKEN_THRESHOLD", "4321");

    let mut config = Config::default();
    config.apply_env_overrides();

    assert_eq!(config.tools.mcp_tools, McpToolsMode::Deferred);
    assert_eq!(config.tools.mcp_tools_token_threshold, 4_321);
    restore_env_var("JCODE_MCP_TOOLS", previous_mode);
    restore_env_var("JCODE_MCP_TOOLS_TOKEN_THRESHOLD", previous_threshold);
}
#[test]
fn tool_config_explicit_enabled_uses_allow_list() {
    let cfg = ToolConfig {
        enabled: vec!["gmail".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();
    let allowed = selection
        .allowed_tools
        .expect("explicit enabled is an allow-list");

    assert!(allowed.contains("gmail"));
    assert!(!selection.disabled_tools.contains("gmail"));
}
#[test]
fn tool_config_all_enabled_sentinel_keeps_unrestricted_toolset() {
    let cfg = ToolConfig {
        enabled: vec!["*".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(!selection.disabled_tools.contains("gmail"));
}
#[test]
fn tool_config_explicit_disabled_overrides_all_enabled_sentinel() {
    let cfg = ToolConfig {
        enabled: vec!["*".to_string()],
        disabled: vec!["gmail".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.contains("gmail"));
}
#[test]
fn tool_config_acp_profile_allows_core_coding_plus_batch() {
    let cfg = ToolConfig {
        profile: "acp".to_string(),
        ..ToolConfig::default()
    };
    let allowed = cfg.allowed_tools().expect("acp profile is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(allowed.contains("write"));
    assert!(allowed.contains("apply_patch"));
    assert!(allowed.contains("agentgrep"));
    assert!(allowed.contains("batch"));
    assert!(allowed.contains("mcp"));
    assert!(!allowed.contains("swarm"));
    assert!(!allowed.contains("subagent"));
    assert!(!allowed.contains("side_panel"));
}
#[test]
fn acp_config_defaults_to_standard_profile_and_acp_tools() {
    let cfg = Config::default();
    assert_eq!(cfg.acp.profile, "standard");
    assert_eq!(cfg.acp.tool_profile, "acp");
}
#[test]
fn tool_config_minimal_profile_allows_core_coding_tools() {
    let cfg = ToolConfig {
        profile: "minimal".to_string(),
        ..ToolConfig::default()
    };
    let allowed = cfg
        .allowed_tools()
        .expect("minimal profile is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(allowed.contains("write"));
    assert!(allowed.contains("apply_patch"));
    assert!(allowed.contains("agentgrep"));
    assert!(!allowed.contains("browser"));
    assert!(!allowed.contains("swarm"));
}
#[test]
fn tool_config_explicit_enabled_and_disabled_lists_compose() {
    let cfg = ToolConfig {
        enabled: vec![
            "shell".to_string(),
            "read_file".to_string(),
            "browser".to_string(),
        ],
        disabled: vec!["browser".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();
    let allowed = selection
        .allowed_tools
        .expect("explicit enabled is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(!allowed.contains("shell"));
    assert!(!allowed.contains("read_file"));
    assert!(!allowed.contains("browser"));
    assert!(selection.disabled_tools.contains("browser"));
}
#[test]
fn tool_config_none_profile_disables_all_tools() {
    let cfg = ToolConfig {
        profile: "none".to_string(),
        ..ToolConfig::default()
    };
    assert!(
        cfg.allowed_tools()
            .expect("none profile is empty")
            .is_empty()
    );
}
#[test]
fn tool_config_disabled_only_keeps_full_profile_with_deny_list() {
    let cfg = ToolConfig {
        disabled: vec!["browser".to_string(), "swarm".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.contains("browser"));
    assert!(selection.disabled_tools.contains("swarm"));
    assert!(!selection.disabled_tools.contains("gmail"));
}
