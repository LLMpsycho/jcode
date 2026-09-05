use super::*;

impl MultiProvider {
    pub(super) fn explicit_tool_runtime(&self) -> Option<Arc<dyn Provider>> {
        match self.active_provider() {
            ActiveProvider::Claude => self.anthropic_provider().or_else(|| self.claude_provider()),
            ActiveProvider::OpenAI => self.openai_provider(),
            ActiveProvider::Copilot => self.copilot_provider(),
            ActiveProvider::Antigravity => self.antigravity_provider(),
            ActiveProvider::Gemini => self.gemini_provider(),
            ActiveProvider::Cursor => self.cursor_provider(),
            ActiveProvider::Bedrock => self
                .bedrock_provider()
                .map(|runtime| runtime as Arc<dyn Provider>),
            ActiveProvider::OpenRouter => self.active_openrouter_execution_provider(),
        }
    }
}
