use super::*;

/// A synchronous fork cannot await an authentication writer or assume Auto.
/// Keep the rejected fork inert until its caller retries with a stable mode.
pub(super) struct UnavailableFork {
    pub(super) model: String,
}

#[async_trait]
impl Provider for UnavailableFork {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: self.model.clone(),
        })
    }

    fn set_model(&self, _: &str) -> Result<()> {
        anyhow::bail!(
            "Anthropic authentication is changing; retry the advisor model selection or review"
        )
    }

    async fn complete(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> Result<EventStream> {
        anyhow::bail!(
            "Anthropic authentication is changing; retry the advisor model selection or review"
        )
    }
}
