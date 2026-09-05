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

/// A failed route restoration must never return a usable default runtime: an
/// inheriting advisor would otherwise send primary evidence to the wrong route.
pub(super) struct UnavailableFork {
    model: String,
    reason: String,
}

impl UnavailableFork {
    pub(super) fn new(model: String, error: &anyhow::Error) -> Self {
        Self {
            model,
            reason: crate::message::redact_secrets(&error.to_string()),
        }
    }
    fn error(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "Could not preserve the selected route in a private provider fork: {}",
            self.reason
        )
    }
}

#[async_trait]
impl Provider for UnavailableFork {
    fn name(&self) -> &str {
        "unavailable-private-route"
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: self.model.clone(),
            reason: self.reason.clone(),
        })
    }
    fn set_model(&self, _: &str) -> Result<()> {
        Err(self.error())
    }
    async fn complete(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<EventStream> {
        Err(self.error())
    }
}
