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
                if prefix == "claude-api" {
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
        assert_eq!(provider.active_provider(), active);
        let expected_credential = if prefix.ends_with("-oauth") {
            jcode_provider_core::ResolvedCredential::Oauth
        } else {
            jcode_provider_core::ResolvedCredential::ApiKey
        };
        assert_eq!(
            provider.active_resolved_credential(),
            Some(expected_credential)
        );
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
    assert_advisor_fork_preserves_effort(ActiveProvider::Claude, "claude-api");
}

#[test]
fn advisor_fork_preserves_jcode_subscription_effort() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        crate::provider_catalog::save_env_value_to_env_file(
            crate::subscription_catalog::JCODE_API_KEY_ENV,
            crate::subscription_catalog::JCODE_ENV_FILE,
            Some("test-managed-subscription-token"),
        )
        .expect("save isolated subscription credential");
        let provider = jcode::JcodeProvider::new();
        provider.set_model("gpt-5.5").unwrap();
        provider.set_reasoning_effort("high").unwrap();

        let advisor = provider.fork();
        assert_eq!(advisor.model(), "gpt-5.5");
        assert_eq!(advisor.reasoning_effort().as_deref(), Some("high"));
        advisor.set_reasoning_effort("low").unwrap();
        advisor.set_model("gpt-5.6-sol").unwrap();
        assert_eq!(provider.model(), "gpt-5.5");
        assert_eq!(provider.reasoning_effort().as_deref(), Some("high"));
        provider.set_reasoning_effort("medium").unwrap();
        assert_eq!(advisor.reasoning_effort().as_deref(), Some("low"));
    });
}

fn advisor_primary_on_custom_endpoint() -> MultiProvider {
    save_test_openai_compatible_login_config("gpt-5.5");
    let provider = test_multi_provider_with_cursor();
    *provider.cursor.write().unwrap() = None;
    let custom = external::instantiate_openrouter_runtime(
        external::OpenRouterRuntimeSpec::CompatibleProfile(
            crate::provider_catalog::OPENAI_COMPAT_PROFILE,
        ),
    )
    .expect("construct isolated custom endpoint");
    custom.set_model("gpt-5.5").unwrap();
    *provider.openrouter.write().unwrap() = Some(custom);
    provider.set_active_provider(ActiveProvider::OpenRouter);
    provider
}

fn advisor_openrouter_selection() -> RouteSelection {
    RouteSelection::from_model_route(&ModelRoute {
        model: "openai/gpt-5.5".to_string(),
        provider: "OpenAI".to_string(),
        api_method: "openrouter".to_string(),
        available: true,
        detail: String::new(),
        cheapness: None,
    })
}

#[test]
fn advisor_openrouter_route_replaces_custom_endpoint_and_preserves_provider_pin() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        crate::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
        let provider = advisor_primary_on_custom_endpoint();
        let primary_runtime = provider.openrouter_provider().unwrap();
        assert!(!primary_runtime.supports_provider_routing_features());

        provider
            .set_route_selection(&advisor_openrouter_selection())
            .expect("select exact aggregator route");
        let selected = provider.active_openrouter_execution_provider().unwrap();
        assert!(selected.supports_provider_routing_features());
        assert!(selected.direct_openai_compatible_route_parts().is_none());
        assert_eq!(selected.model(), "openai/gpt-5.5");
        assert_eq!(
            selected.explicit_provider_pin_for_current_model().as_deref(),
            Some("OpenAI")
        );
        assert_eq!(primary_runtime.model(), "gpt-5.5");
    });
}

#[test]
fn advisor_openrouter_route_without_credentials_preserves_custom_endpoint() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = advisor_primary_on_custom_endpoint();
        let primary_runtime = provider.openrouter_provider().unwrap();

        assert!(
            provider
                .set_route_selection(&advisor_openrouter_selection())
                .is_err()
        );
        let active = provider.active_openrouter_execution_provider().unwrap();
        assert!(Arc::ptr_eq(&active, &primary_runtime));
        assert_eq!(provider.model(), "gpt-5.5");
        assert!(!active.supports_provider_routing_features());
    });
}

#[test]
fn advisor_gemini_route_leaves_custom_endpoint_for_the_selected_oauth_runtime() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = advisor_primary_on_custom_endpoint();
        let gemini: Arc<dyn Provider> = Arc::new(StubExternalRuntime::new(
            "gemini",
            "Gemini",
            "code-assist-oauth",
            gemini::AVAILABLE_MODELS,
        ));
        let selection = RouteSelection::from_model_route(&gemini.model_routes()[0]);
        *provider.gemini.write().unwrap() = Some(gemini);
        let custom = provider.openrouter_provider().unwrap();

        provider.set_route_selection(&selection).unwrap();
        assert_eq!(provider.active_provider(), ActiveProvider::Gemini);
        assert_eq!(provider.model(), selection.model);
        assert_eq!(custom.model(), "gpt-5.5");
    });
}

#[test]
fn advisor_gemini_route_without_runtime_preserves_custom_endpoint() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = advisor_primary_on_custom_endpoint();
        let selection = RouteSelection::from_model_route(&ModelRoute {
            model: gemini::AVAILABLE_MODELS[0].to_string(),
            provider: "Gemini".to_string(),
            api_method: "code-assist-oauth".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        });

        assert!(provider.set_route_selection(&selection).is_err());
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert_eq!(provider.model(), "gpt-5.5");
    });
}

#[test]
fn advisor_unnamed_compatible_route_does_not_use_primary_openai_credentials() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        crate::env::set_var(
            "JCODE_OPENROUTER_API_BASE",
            "https://unnamed-endpoint.test/v1",
        );
        crate::env::set_var("OPENROUTER_API_KEY", "test-custom-endpoint-key");
        let custom = test_openrouter_runtime().unwrap();
        custom.set_model("gpt-5.5").unwrap();
        let (label, api_method, _) = custom.direct_openai_compatible_route_parts().unwrap();
        let selection = RouteSelection::from_model_route(&ModelRoute {
            model: "gpt-5.5".to_string(),
            provider: label,
            api_method,
            available: true,
            detail: String::new(),
            cheapness: None,
        });
        assert_eq!(
            selection.runtime_key,
            RuntimeKey::OpenAiCompatible { profile_id: None }
        );
        let provider = test_multi_provider_with_openai();
        let openai = provider.openai_provider().unwrap();
        let initial_model = openai.model();
        *provider.openrouter.write().unwrap() = Some(custom.clone());

        provider.set_route_selection(&selection).unwrap();
        assert_eq!(provider.active_provider(), ActiveProvider::OpenRouter);
        assert!(Arc::ptr_eq(
            &provider.active_openrouter_execution_provider().unwrap(),
            &custom
        ));
        assert_eq!(openai.model(), initial_model);
    });
}

#[test]
fn advisor_unnamed_compatible_route_without_runtime_preserves_openai() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let provider = test_multi_provider_with_openai();
        let initial_model = provider.model();
        let selection = RouteSelection::from_model_route(&ModelRoute {
            model: "gpt-5.5".to_string(),
            provider: "OpenAI-compatible".to_string(),
            api_method: "openai-compatible".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        });

        assert!(provider.set_route_selection(&selection).is_err());
        assert_eq!(provider.active_provider(), ActiveProvider::OpenAI);
        assert_eq!(provider.model(), initial_model);
    });
}

struct AdvisorInternalRuntime {
    supports_toolless: bool,
}

#[async_trait::async_trait]
impl Provider for AdvisorInternalRuntime {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        anyhow::bail!("capability test must not contact an internal agent")
    }

    fn name(&self) -> &str {
        "internal-agent"
    }

    fn model(&self) -> String {
        "internal-model".to_string()
    }

    fn handles_tools_internally(&self) -> bool {
        true
    }

    fn supports_toolless_requests(&self) -> bool {
        self.supports_toolless
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            supports_toolless: self.supports_toolless,
        })
    }
}

#[test]
fn advisor_toolless_capability_delegates_to_active_internal_profile() {
    with_clean_provider_test_env(|| {
        let provider = test_multi_provider_with_cursor();
        *provider.cursor.write().unwrap() = None;
        provider.set_active_provider(ActiveProvider::OpenRouter);
        assert!(
            !provider.supports_toolless_requests(),
            "missing runtime is unsafe"
        );

        let registry = ProviderRegistry::new(&provider);
        for supported in [false, true] {
            registry.install_compatible_profile(
                "internal-agent".to_string(),
                Arc::new(AdvisorInternalRuntime {
                    supports_toolless: supported,
                }),
            );
            registry.set_active_compatible_profile("internal-agent".to_string());
            assert_eq!(provider.supports_toolless_requests(), supported);
        }
    });
}

#[test]
fn advisor_toolless_capability_delegates_to_claude_cli_override() {
    with_clean_provider_test_env(|| {
        let provider = test_multi_provider_with_cursor();
        *provider.cursor.write().unwrap() = None;
        *provider.claude.write().unwrap() = Some(Arc::new(AdvisorInternalRuntime {
            supports_toolless: true,
        }));
        provider.set_active_provider(ActiveProvider::Claude);
        assert!(provider.handles_tools_internally());
        assert!(provider.supports_toolless_requests());

        // The dispatch path prefers the installed Anthropic runtime even when
        // legacy CLI mode is configured; the capability must use the same one.
        *provider.anthropic.write().unwrap() = Some(Arc::new(AdvisorInternalRuntime {
            supports_toolless: false,
        }));
        assert!(!provider.supports_toolless_requests());
    });
}
