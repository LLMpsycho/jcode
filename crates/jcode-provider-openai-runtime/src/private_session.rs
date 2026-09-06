use super::*;

/// Synchronous fork cannot wait on an authentication writer. Reject this one
/// fork explicitly rather than sharing mutable auth or guessing another route.
pub(super) struct UnavailableFork {
    pub(super) model: String,
}

#[async_trait]
impl Provider for UnavailableFork {
    fn name(&self) -> &str {
        "openai"
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: self.model.clone(),
        })
    }
    async fn complete(
        &self,
        _: &[ChatMessage],
        _: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<EventStream> {
        anyhow::bail!(
            "OpenAI authentication is changing; retry the advisor model selection or review"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            jcode_base::env::set_var(key, value);
            Self { key, previous }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(previous) => jcode_base::env::set_var(self.key, previous),
                None => jcode_base::env::remove_var(self.key),
            }
        }
    }
    fn provider() -> OpenAIProvider {
        OpenAIProvider::new(CodexCredentials {
            access_token: "fixture-oauth-access".into(),
            refresh_token: "fixture-refresh".into(),
            id_token: None,
            account_id: Some("fixture-account".into()),
            expires_at: None,
        })
    }
    #[test]
    fn private_fork_authentication_and_runtime_hint_do_not_change_primary() {
        let _lock = jcode_base::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let _api = EnvGuard::set("OPENAI_API_KEY", "fixture-private-api-key");
        let _runtime = EnvGuard::set("JCODE_RUNTIME_PROVIDER", "openai-oauth");
        let primary = provider();
        let advisor = primary.fork();
        advisor
            .set_credential_mode(OpenAICredentialMode::ApiKey)
            .unwrap();
        assert_eq!(advisor.credential_mode(), OpenAICredentialMode::ApiKey);
        assert_eq!(primary.credential_mode(), OpenAICredentialMode::OAuth);
        assert_eq!(
            primary.credentials.try_read().unwrap().access_token,
            "fixture-oauth-access"
        );
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
            "openai-oauth"
        );
        primary
            .set_credential_mode(OpenAICredentialMode::ApiKey)
            .unwrap();
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
            "openai-api"
        );
        advisor
            .set_credential_mode(OpenAICredentialMode::Auto)
            .unwrap();
        assert_eq!(primary.credential_mode(), OpenAICredentialMode::ApiKey);
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
            "openai-api"
        );
    }
    #[test]
    fn freshly_constructed_private_runtime_keeps_process_hint_and_browser_flag_private() {
        let _lock = jcode_base::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let _api = EnvGuard::set("OPENAI_API_KEY", "fixture-private-api-key");
        let _runtime = EnvGuard::set("JCODE_RUNTIME_PROVIDER", "openai-oauth");
        let primary = OpenAIProvider::new_browser_only();
        let advisor = primary.fork();
        advisor
            .set_credential_mode(OpenAICredentialMode::ApiKey)
            .unwrap();
        assert!(primary.is_browser_only());
        let private = provider();
        private.prepare_private_session();
        private
            .set_credential_mode(OpenAICredentialMode::ApiKey)
            .unwrap();
        assert_eq!(
            std::env::var("JCODE_RUNTIME_PROVIDER").unwrap(),
            "openai-oauth"
        );
    }
    #[test]
    fn fork_during_authentication_write_fails_visibly_without_sharing_locks() {
        let _lock = jcode_base::storage::lock_test_env();
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("JCODE_HOME", temp.path());
        let primary = provider();
        let held = primary.credentials.try_write().unwrap();
        let advisor = primary.fork();
        let result = futures::executor::block_on(advisor.complete(&[], &[], "", None));
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("authentication is changing")
        );
        assert_eq!(held.access_token, "fixture-oauth-access");
    }
}
