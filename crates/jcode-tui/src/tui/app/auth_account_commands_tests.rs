use super::*;

#[test]
fn parse_account_doctor_subcommands() {
    assert!(matches!(
        parse_account_command("/account doctor"),
        Some(Ok(AccountCommand::Doctor { provider_id: None }))
    ));
    assert!(matches!(
        parse_account_command("/account openai doctor"),
        Some(Ok(AccountCommand::Doctor { provider_id: Some(provider_id) })) if provider_id == "openai"
    ));
}

#[test]
fn parse_native_jcode_account_actions() {
    assert!(matches!(
        parse_account_command("/account jcode login"),
        Some(Ok(AccountCommand::Login { provider_id })) if provider_id == "jcode"
    ));
    assert!(matches!(
        parse_account_command("/account jcode status"),
        Some(Ok(AccountCommand::JcodeStatus))
    ));
    assert!(matches!(
        parse_account_command("/account jcode manage"),
        Some(Ok(AccountCommand::JcodeManage))
    ));
    assert!(matches!(
        parse_account_command("/account jcode logout"),
        Some(Ok(AccountCommand::JcodeLogout))
    ));
}

#[test]
fn render_auth_doctor_markdown_includes_recovery_steps() {
    let _guard = crate::storage::lock_test_env();
    let markdown = render_auth_doctor_markdown(Some("openai"));
    assert!(markdown.contains("OpenAI (openai)"));
    assert!(markdown.contains("Next steps"));
    assert!(markdown.contains("jcode login --provider openai"));
    assert!(markdown.contains("Review current state: jcode auth status --json"));
}
