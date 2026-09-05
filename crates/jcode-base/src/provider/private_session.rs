use super::*;

impl MultiProvider {
    pub(super) fn is_private_session(&self) -> bool {
        self.private_session
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn prepare_private_runtime(&self, provider: Option<Arc<dyn Provider>>) {
        if self.is_private_session()
            && let Some(provider) = provider
        {
            provider.prepare_private_session();
        }
    }

    pub(super) fn prepare_all_private_runtimes(&self) {
        for provider in [
            self.claude_provider(),
            self.anthropic_provider(),
            self.openai_provider(),
            self.copilot_provider(),
            self.antigravity_provider(),
            self.gemini_provider(),
            self.cursor_provider(),
            self.openrouter_provider(),
        ] {
            self.prepare_private_runtime(provider);
        }
        let profiles = self
            .openai_compatible_profiles
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for provider in profiles.values() {
            provider.prepare_private_session();
        }
    }
}
