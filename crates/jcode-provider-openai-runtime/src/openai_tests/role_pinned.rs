use super::*;

fn role_provider() -> OpenAIProvider {
    OpenAIProvider::new(CodexCredentials {
        access_token: "test-role-access-token".into(),
        refresh_token: String::new(),
        id_token: None,
        account_id: None,
        expires_at: None,
    })
}

#[test]
fn role_pinned_openai_keeps_unavailable_selected_model_without_substitution() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    jcode_base::auth::codex::set_active_account_override(Some("pinned-role-test".into()));
    jcode_base::provider::models::reset_model_catalog_services_for_tests();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let provider = role_provider();
        *provider.model.write().await = "gpt-5.5".into();
        jcode_base::provider::populate_account_models(vec!["gpt-5.6-sol".into()]);
        jcode_base::provider::record_model_unavailable_for_account("gpt-5.5", "model unavailable");
        assert_eq!(
            jcode_base::provider::get_best_available_openai_model().as_deref(),
            Some("gpt-5.6-sol")
        );

        provider.set_route_pinned(true);
        assert_eq!(provider.model_id().await, "gpt-5.5");
        assert_eq!(provider.model(), "gpt-5.5");

        provider.set_route_pinned(false);
        assert_eq!(provider.model_id().await, "gpt-5.6-sol");
        assert_eq!(provider.model(), "gpt-5.6-sol");
    });
    jcode_base::provider::models::reset_model_catalog_services_for_tests();
    jcode_base::auth::codex::set_active_account_override(None);
}

#[test]
fn role_pinned_openai_forks_preserve_pin_without_changing_primary_or_siblings() {
    let _lock = jcode_base::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _runtime_guard = runtime.enter();
    let primary = role_provider();
    let primary_model = primary.model();
    let role = primary.fork();
    role.set_route_pinned(true);
    let nested = role.fork();
    let sibling = primary.fork();
    assert!(role.route_pinned());
    assert!(nested.route_pinned());
    assert!(!primary.route_pinned());
    assert!(!sibling.route_pinned());
    role.set_route_pinned(false);
    assert!(nested.route_pinned());
    assert!(!role.route_pinned());
    assert_eq!(primary.model(), primary_model);
}
