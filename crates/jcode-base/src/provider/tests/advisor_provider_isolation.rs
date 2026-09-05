use super::*;

fn assert_advisor_fork_isolates_model(
    active: ActiveProvider,
    name: &'static str,
    label: &'static str,
    api_method: &'static str,
    models: &'static [&'static str],
) {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = test_multi_provider_with_cursor();
        *provider.cursor.write().unwrap() = None;
        let original: Arc<dyn Provider> =
            Arc::new(StubExternalRuntime::new(name, label, api_method, models));
        let slot = match active {
            ActiveProvider::Copilot => &provider.copilot_api,
            ActiveProvider::Antigravity => &provider.antigravity,
            ActiveProvider::Gemini => &provider.gemini,
            _ => unreachable!("only shared external provider slots are under test"),
        };
        *slot.write().unwrap() = Some(original.clone());
        provider.set_active_provider(active);
        let initial_model = models[0];
        let advisor_model = models[1];

        let advisor = provider.fork();
        assert_eq!(advisor.model(), initial_model);
        advisor
            .set_model(&format!("{name}:{advisor_model}"))
            .expect("select the advisor's model through normal provider routing");

        assert_eq!(advisor.model(), advisor_model);
        assert_eq!(provider.model(), initial_model);
        assert_eq!(original.model(), initial_model);

        // A sibling helper starts from the primary session's model, and its
        // selection must not change the already configured advisor either.
        let sibling = provider.fork();
        assert_eq!(sibling.model(), initial_model);
        assert_eq!(advisor.model(), advisor_model);
    });
}

#[test]
fn advisor_fork_isolates_copilot_model() {
    assert_advisor_fork_isolates_model(
        ActiveProvider::Copilot,
        "copilot",
        "GitHub Copilot",
        "copilot",
        copilot::FALLBACK_MODELS,
    );
}

#[test]
fn advisor_fork_isolates_antigravity_model() {
    assert_advisor_fork_isolates_model(
        ActiveProvider::Antigravity,
        "antigravity",
        "Antigravity",
        "https",
        antigravity::AVAILABLE_MODELS,
    );
}

#[test]
fn advisor_fork_isolates_gemini_model() {
    assert_advisor_fork_isolates_model(
        ActiveProvider::Gemini,
        "gemini",
        "Gemini",
        "https",
        gemini::AVAILABLE_MODELS,
    );
}

fn assert_advisor_fork_preserves_effort(active: ActiveProvider, prefix: &str) {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = test_multi_provider_with_openai();
        let model = match active {
            ActiveProvider::OpenAI => ALL_OPENAI_MODELS[0],
            ActiveProvider::Claude => {
                if prefix == "anthropic-api" {
                    crate::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-advisor");
                }
                *provider.openai.write().unwrap() = None;
                *provider.anthropic.write().unwrap() = Some(test_anthropic_runtime());
                anthropic::AVAILABLE_MODELS[0]
            }
            _ => unreachable!("only dual-auth provider slots are under test"),
        };
        provider
            .set_model(&format!("{prefix}:{model}"))
            .expect("select the primary authenticated model route");
        provider.set_reasoning_effort("high").unwrap();
        let credential = provider.active_resolved_credential();

        let advisor = provider.fork();
        assert_eq!(advisor.model(), model);
        assert_eq!(advisor.active_resolved_credential(), credential);
        assert_eq!(advisor.reasoning_effort().as_deref(), Some("high"));

        advisor.set_reasoning_effort("low").unwrap();
        assert_eq!(provider.reasoning_effort().as_deref(), Some("high"));
        provider.set_reasoning_effort("medium").unwrap();
        assert_eq!(advisor.reasoning_effort().as_deref(), Some("low"));
    });
}

#[test]
fn advisor_fork_preserves_openai_oauth_effort() {
    assert_advisor_fork_preserves_effort(ActiveProvider::OpenAI, "openai-oauth");
}

#[test]
fn advisor_fork_preserves_openai_api_effort() {
    assert_advisor_fork_preserves_effort(ActiveProvider::OpenAI, "openai-api");
}

#[test]
fn advisor_fork_preserves_claude_oauth_effort() {
    assert_advisor_fork_preserves_effort(ActiveProvider::Claude, "claude-oauth");
}

#[test]
fn advisor_fork_preserves_anthropic_api_effort() {
    assert_advisor_fork_preserves_effort(ActiveProvider::Claude, "anthropic-api");
}
