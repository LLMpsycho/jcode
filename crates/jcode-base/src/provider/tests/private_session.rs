use super::*;

#[tokio::test]
async fn private_named_anthropic_selection_rejects_before_process_profile_activation() {
    with_clean_provider_test_env(|| {
        let config = crate::config::Config::path().unwrap();
        std::fs::write(config, "[providers.company]\ntype = 'anthropic-compatible'\nbase_url = 'https://fixture.invalid/v1'\napi_key_env = 'COMPANY_TEST_KEY'\n").unwrap();
        crate::env::set_var("JCODE_RUNTIME_PROVIDER", "openai-oauth");
        let primary = test_multi_provider_with_openai();
        let advisor = primary.fork();
        let before = std::env::var_os("JCODE_NAMED_PROVIDER_PROFILE");
        let result = advisor.set_model("company:claude-sonnet-4");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("process-wide activation")
        );
        assert_eq!(std::env::var_os("JCODE_NAMED_PROVIDER_PROFILE"), before);
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
            "openai-oauth"
        );
        assert_eq!(primary.active_provider(), ActiveProvider::OpenAI);
    });
}

#[test]
fn private_fork_rejects_unrestorable_route_instead_of_using_runtime_default() {
    with_clean_provider_test_env(|| {
        let primary = test_multi_provider_with_cursor();
        let original_model = primary.model();
        external::register_external_provider_fallible(external::CURSOR_RUNTIME, || None);
        let advisor = primary.fork();
        assert_eq!(advisor.model(), original_model);
        let failure = futures::executor::block_on(advisor.complete(&[], &[], "", None))
            .err()
            .unwrap();
        assert!(
            failure
                .to_string()
                .contains("Could not preserve the selected route")
        );
        assert!(advisor.set_model("another-model").is_err());
        assert_eq!(primary.model(), original_model);
        assert_eq!(primary.active_provider(), ActiveProvider::Cursor);
    });
}
