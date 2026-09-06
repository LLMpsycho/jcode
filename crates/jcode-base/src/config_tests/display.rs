#[test]
fn test_ambient_visible_defaults_to_true() {
    assert!(AmbientConfig::default().visible);
}
#[test]
fn test_display_auto_server_reload_defaults_to_true() {
    assert!(DisplayConfig::default().auto_server_reload);
}
#[test]
fn test_display_alignment_defaults_to_left() {
    assert!(!DisplayConfig::default().centered);
}
#[test]
fn display_emoji_defaults_on_and_deserializes_off() {
    assert!(DisplayConfig::default().emoji);
    let cfg: Config = toml::from_str("[display]\nemoji = false\n").expect("config parses");
    assert!(!cfg.display.emoji);
}
#[test]
fn test_provider_failover_defaults_match_new_behavior() {
    let provider = Config::default().provider;
    assert_eq!(
        provider.cross_provider_failover,
        super::CrossProviderFailoverMode::Countdown
    );
    assert!(provider.same_provider_account_failover);
}
#[test]
fn test_provider_failover_disabled_aliases_parse_as_manual() {
    for value in ["off", "false", "disabled", "none"] {
        let cfg: Config = toml::from_str(&format!(
            "[provider]\ncross_provider_failover = \"{value}\"\n"
        ))
        .unwrap_or_else(|error| panic!("{value} should parse: {error}"));
        assert_eq!(
            cfg.provider.cross_provider_failover,
            super::CrossProviderFailoverMode::Manual
        );
        assert_eq!(
            super::CrossProviderFailoverMode::parse(value),
            Some(super::CrossProviderFailoverMode::Manual)
        );
    }
}
#[test]
fn test_native_scrollbars_default_to_enabled() {
    let display = DisplayConfig::default();
    assert!(display.native_scrollbars.chat);
    assert!(display.native_scrollbars.side_panel);
}
#[test]
fn test_copy_badge_alt_label_defaults_to_auto_and_deserializes() {
    assert!(DisplayConfig::default().copy_badge_alt_label.is_empty());

    let cfg: Config = toml::from_str(
        r#"
        [display]
        copy_badge_alt_label = "Option"
        "#,
    )
    .expect("config should deserialize");

    assert_eq!(cfg.display.copy_badge_alt_label, "Option");
}
#[test]
fn test_session_picker_resume_action_defaults_to_current_terminal() {
    assert_eq!(
        Config::default().keybindings.session_picker_enter,
        SessionPickerResumeAction::CurrentTerminal
    );
    assert_eq!(
        SessionPickerResumeAction::CurrentTerminal.alternate(),
        SessionPickerResumeAction::NewTerminal
    );
}
#[test]
fn test_session_picker_resume_action_deserializes_kebab_case() {
    let cfg: Config = toml::from_str(
        r#"
        [keybindings]
        session_picker_enter = "current-terminal"
        "#,
    )
    .expect("config should deserialize");

    assert_eq!(
        cfg.keybindings.session_picker_enter,
        SessionPickerResumeAction::CurrentTerminal
    );
}
#[test]
fn test_env_override_auto_server_reload() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("JCODE_AUTO_SERVER_RELOAD");
    crate::env::set_var("JCODE_AUTO_SERVER_RELOAD", "false");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();

    assert!(!cfg.display.auto_server_reload);

    if let Some(prev) = prev {
        crate::env::set_var("JCODE_AUTO_SERVER_RELOAD", prev);
    } else {
        crate::env::remove_var("JCODE_AUTO_SERVER_RELOAD");
    }
}
#[test]
fn no_emoji_environment_override_disables_emoji() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("JCODE_NO_EMOJI");
    crate::env::set_var("JCODE_NO_EMOJI", "1");
    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert!(!cfg.display.emoji);

    crate::env::set_var("JCODE_NO_EMOJI", "false");
    cfg.display.emoji = false;
    cfg.apply_env_overrides();
    assert!(cfg.display.emoji);

    restore_env_var("JCODE_NO_EMOJI", prev);
}
#[test]
fn test_env_override_native_scrollbars() {
    let _guard = crate::storage::lock_test_env();
    let prev_chat = std::env::var_os("JCODE_CHAT_NATIVE_SCROLLBAR");
    let prev_side = std::env::var_os("JCODE_SIDE_PANEL_NATIVE_SCROLLBAR");
    crate::env::set_var("JCODE_CHAT_NATIVE_SCROLLBAR", "true");
    crate::env::set_var("JCODE_SIDE_PANEL_NATIVE_SCROLLBAR", "false");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();

    assert!(cfg.display.native_scrollbars.chat);
    assert!(!cfg.display.native_scrollbars.side_panel);

    if let Some(prev) = prev_chat {
        crate::env::set_var("JCODE_CHAT_NATIVE_SCROLLBAR", prev);
    } else {
        crate::env::remove_var("JCODE_CHAT_NATIVE_SCROLLBAR");
    }
    if let Some(prev) = prev_side {
        crate::env::set_var("JCODE_SIDE_PANEL_NATIVE_SCROLLBAR", prev);
    } else {
        crate::env::remove_var("JCODE_SIDE_PANEL_NATIVE_SCROLLBAR");
    }
}
#[test]
fn test_env_override_diff_mode_full_inline() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("JCODE_DIFF_MODE");
    crate::env::set_var("JCODE_DIFF_MODE", "full-inline");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();

    assert_eq!(cfg.display.diff_mode, DiffDisplayMode::FullInline);

    if let Some(prev) = prev {
        crate::env::set_var("JCODE_DIFF_MODE", prev);
    } else {
        crate::env::remove_var("JCODE_DIFF_MODE");
    }
}
