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

#[test]
fn advisor_subscription_route_stays_managed_after_auth_refresh_and_fork() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        crate::env::set_var("JCODE_OPENROUTER_MODEL_CATALOG", "0");
        crate::provider_catalog::save_env_value_to_env_file(
            crate::subscription_catalog::JCODE_API_KEY_ENV,
            crate::subscription_catalog::JCODE_ENV_FILE,
            Some("test-managed-subscription-token"),
        )
        .unwrap();
        assert!(std::env::var_os("OPENROUTER_API_KEY").is_none());
        let executions = Arc::new(std::sync::Mutex::new(Vec::<Arc<dyn Provider>>::new()));
        let captured = executions.clone();
        external::register_openrouter_factory(move |spec| {
            assert!(matches!(spec, external::OpenRouterRuntimeSpec::Default));
            let runtime: Arc<dyn Provider> =
                Arc::new(jcode_provider_openrouter_runtime::OpenRouterProvider::new()?);
            let (_, api_method, _) = runtime.direct_openai_compatible_route_parts().unwrap();
            assert_eq!(api_method, "jcode-subscription");
            captured.lock().unwrap().push(runtime.clone());
            Ok(runtime)
        });

        let provider = jcode::JcodeProvider::new();
        provider.set_model("gpt-5.5").unwrap();
        assert_eq!(
            executions.lock().unwrap().last().unwrap().model(),
            "gpt-5.5"
        );
        provider.set_reasoning_effort("high").unwrap();
        provider.on_auth_changed();
        assert_eq!(
            executions.lock().unwrap().last().unwrap().model(),
            "gpt-5.5"
        );
        assert_eq!(provider.reasoning_effort().as_deref(), Some("high"));
        let advisor = provider.fork();
        assert_eq!(advisor.model(), "gpt-5.5");
        assert_eq!(advisor.reasoning_effort().as_deref(), Some("high"));
        assert_eq!(
            executions.lock().unwrap().last().unwrap().model(),
            "gpt-5.5"
        );
        advisor.set_model("gpt-5.6-sol").unwrap();
        assert_eq!(
            executions.lock().unwrap().last().unwrap().model(),
            "gpt-5.6-sol"
        );
        assert_eq!(provider.model(), "gpt-5.5");
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
            selected
                .explicit_provider_pin_for_current_model()
                .as_deref(),
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
fn advisor_toolless_capability_delegates_to_active_claude_slot() {
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

struct AdvisorQuotaRuntime {
    calls: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    quota_error: bool,
    route_pinned: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl Provider for AdvisorQuotaRuntime {
    fn set_route_pinned(&self, pinned: bool) {
        self.route_pinned
            .store(pinned, std::sync::atomic::Ordering::Relaxed);
    }

    fn route_pinned(&self) -> bool {
        self.route_pinned.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        assert_eq!(messages.len(), 1);
        assert!(tools.is_empty());
        assert_eq!(system, "selected review");
        assert_eq!(resume_session_id, Some("review-resume"));
        self.calls
            .lock()
            .unwrap()
            .push(crate::auth::codex::active_account_label());
        if self.quota_error {
            assert!(
                self.route_pinned(),
                "exact route pin reaches the actual runtime"
            );
            anyhow::bail!("429 rate limit exceeded");
        }
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn complete_split(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system_static: &str,
        system_dynamic: &str,
        resume_session_id: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        assert_eq!(system_dynamic, "role-specific dynamic context");
        self.complete(messages, tools, system_static, resume_session_id)
            .await
    }

    fn name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> String {
        "gpt-5.5".to_string()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            calls: self.calls.clone(),
            quota_error: self.quota_error,
            route_pinned: std::sync::atomic::AtomicBool::new(self.route_pinned()),
        })
    }
}

#[test]
fn advisor_selected_route_quota_error_never_rotates_shared_oauth_accounts() {
    with_clean_provider_test_env(|| {
        with_env_var("JCODE_SAME_PROVIDER_ACCOUNT_FAILOVER", "true", || {
            let runtime = enter_test_runtime();
            runtime.block_on(async {
                let provider = test_multi_provider_with_openai();
                let primary = crate::auth::codex::primary_account_label();
                let secondary = crate::auth::codex::upsert_account_from_tokens(
                    "secondary",
                    "test-secondary-access-token",
                    "test-secondary-refresh-token",
                    None,
                    Some(chrono::Utc::now().timestamp_millis() + 86_400_000),
                )
                .unwrap();
                crate::auth::codex::set_active_account_override(Some(primary.clone()));
                assert!(same_provider_account_failover_enabled());
                assert!(
                    crate::auth::codex::list_accounts()
                        .unwrap()
                        .iter()
                        .any(|account| account.label == secondary && account.label != primary)
                );
                let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                *provider.openai.write().unwrap() = Some(Arc::new(AdvisorQuotaRuntime {
                    calls: calls.clone(),
                    quota_error: true,
                    route_pinned: std::sync::atomic::AtomicBool::new(false),
                }));

                let error = provider
                    .complete_on_selected_route(
                        &[Message::user("evidence")],
                        &[],
                        "selected review",
                        Some("review-resume"),
                    )
                    .await
                    .err()
                    .expect("return the selected account's quota failure");
                assert!(error.to_string().contains("429"));
                assert_eq!(*calls.lock().unwrap(), vec![Some(primary.clone())]);
                assert_eq!(crate::auth::codex::active_account_label(), Some(primary));
                assert_eq!(provider.active_provider(), ActiveProvider::OpenAI);
            });
        });
    });
}

#[test]
fn advisor_selected_route_preserves_successful_request_and_resume_contract() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        runtime.block_on(async {
            let provider = test_multi_provider_with_openai();
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            *provider.openai.write().unwrap() = Some(Arc::new(AdvisorQuotaRuntime {
                calls: calls.clone(),
                quota_error: false,
                route_pinned: std::sync::atomic::AtomicBool::new(false),
            }));
            let stream = provider
                .complete_on_selected_route(
                    &[Message::user("evidence")],
                    &[],
                    "selected review",
                    Some("review-resume"),
                )
                .await
                .expect("complete on the selected runtime");
            drop(stream);
            assert_eq!(calls.lock().unwrap().len(), 1);
            assert_eq!(provider.active_provider(), ActiveProvider::OpenAI);
        });
    });
}

fn assert_pinned_role_quota_preserves_account_and_context(split: bool) {
    with_clean_provider_test_env(|| {
        with_env_var("JCODE_SAME_PROVIDER_ACCOUNT_FAILOVER", "true", || {
            let runtime = enter_test_runtime();
            runtime.block_on(async {
                let provider = test_multi_provider_with_openai();
                let primary_account = crate::auth::codex::primary_account_label();
                crate::auth::codex::upsert_account_from_tokens(
                    "secondary-role-account",
                    "test-secondary-role-access-token",
                    "test-secondary-role-refresh-token",
                    None,
                    Some(chrono::Utc::now().timestamp_millis() + 86_400_000),
                )
                .unwrap();
                crate::auth::codex::set_active_account_override(Some(primary_account.clone()));
                assert!(same_provider_account_failover_enabled());
                assert!(
                    !MultiProvider::same_provider_account_candidates(ActiveProvider::OpenAI)
                        .is_empty()
                );
                let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                let captured = calls.clone();
                external::register_external_provider(external::OPENAI_RUNTIME, move || {
                    Arc::new(AdvisorQuotaRuntime {
                        calls: captured.clone(),
                        quota_error: true,
                        route_pinned: std::sync::atomic::AtomicBool::new(false),
                    })
                });
                let role = provider.fork();
                role.set_route_pinned(true);
                assert!(role.route_pinned());
                assert!(!provider.route_pinned());
                let messages = [Message::user("evidence")];
                let result = if split {
                    role.complete_split(
                        &messages,
                        &[],
                        "selected review",
                        "role-specific dynamic context",
                        Some("review-resume"),
                    )
                    .await
                } else {
                    role.complete(&messages, &[], "selected review", Some("review-resume"))
                        .await
                };
                let error = result.err().expect("selected role quota failure");
                assert_eq!(error.to_string(), "429 rate limit exceeded");
                assert_eq!(*calls.lock().unwrap(), vec![Some(primary_account.clone())]);
                assert_eq!(
                    crate::auth::codex::active_account_label(),
                    Some(primary_account)
                );
                assert!(role.drain_startup_notices().is_empty());
                assert_eq!(provider.active_provider(), ActiveProvider::OpenAI);
                assert!(!provider.route_pinned());
            });
        });
    });
}

#[test]
fn agent_role_pinned_completion_does_not_rotate_oauth_accounts() {
    assert_pinned_role_quota_preserves_account_and_context(false);
}

#[test]
fn agent_role_pinned_split_completion_keeps_dynamic_context_and_account() {
    assert_pinned_role_quota_preserves_account_and_context(true);
}

#[test]
fn agent_role_pinning_is_private_and_preserved_by_nested_forks() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        let primary = test_multi_provider_with_openai();
        let selected_model = primary.model();
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
        assert_eq!(primary.model(), selected_model);
        assert_eq!(primary.active_provider(), ActiveProvider::OpenAI);
    });
}

#[test]
fn agent_role_pinned_missing_runtime_never_uses_another_provider() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        runtime.block_on(async {
            let provider = test_multi_provider_with_openai();
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            *provider.openai.write().unwrap() = Some(Arc::new(AdvisorQuotaRuntime {
                calls: calls.clone(),
                quota_error: false,
                route_pinned: std::sync::atomic::AtomicBool::new(false),
            }));
            provider.set_active_provider(ActiveProvider::Claude);
            provider.set_route_pinned(true);
            let messages = [Message::user("evidence")];
            let error = provider
                .complete(&messages, &[], "selected review", Some("review-resume"))
                .await
                .err()
                .expect("missing selected runtime must fail");
            assert!(error.to_string().contains("Claude credentials not available"));
            assert!(calls.lock().unwrap().is_empty());
            assert_eq!(provider.active_provider(), ActiveProvider::Claude);

            // Ordinary sessions retain their existing automatic availability fallback.
            provider.set_route_pinned(false);
            let stream = provider
                .complete(&messages, &[], "selected review", Some("review-resume"))
                .await
                .expect("ordinary completion can use another configured provider");
            drop(stream);
            assert_eq!(calls.lock().unwrap().len(), 1);
            assert_eq!(provider.active_provider(), ActiveProvider::OpenAI);
        });
    });
}

#[test]
fn agent_role_pinning_survives_managed_subscription_forks() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        let _runtime_guard = runtime.enter();
        crate::provider_catalog::save_env_value_to_env_file(
            crate::subscription_catalog::JCODE_API_KEY_ENV,
            crate::subscription_catalog::JCODE_ENV_FILE,
            Some("test-managed-role-token"),
        )
        .unwrap();
        let primary = jcode::JcodeProvider::new();
        let role = primary.fork();
        role.set_route_pinned(true);
        let nested = role.fork();
        assert!(role.route_pinned());
        assert!(nested.route_pinned());
        assert!(!primary.route_pinned());
        role.set_route_pinned(false);
        assert!(nested.route_pinned());
    });
}
