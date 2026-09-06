use super::*;

#[test]
fn format_error_chain_joins_context_with_root_cause() {
    use anyhow::Context as _;
    let root: anyhow::Result<()> = Err(anyhow::anyhow!("HTTP 429 usage_limit_reached"));
    let wrapped = root
        .context("native provider stream event error")
        .context("open native provider stream")
        .unwrap_err();
    let rendered = format_error_chain(&wrapped);
    // Outer-most context first, root cause last, all joined.
    assert_eq!(
        rendered,
        "open native provider stream: native provider stream event error: HTTP 429 usage_limit_reached"
    );
}

#[test]
fn format_error_chain_dedupes_and_handles_single_error() {
    let single = anyhow::anyhow!("standalone failure");
    assert_eq!(format_error_chain(&single), "standalone failure");
}

#[test]
fn native_provider_kind_maps_every_generic_id() {
    for (id, expected) in [
        ("openai", NativeProviderKind::OpenAi),
        ("gemini", NativeProviderKind::Gemini),
        ("cursor", NativeProviderKind::Cursor),
        ("copilot", NativeProviderKind::Copilot),
        ("bedrock", NativeProviderKind::Bedrock),
        ("jcode", NativeProviderKind::Jcode),
        ("azure-openai", NativeProviderKind::Azure),
    ] {
        assert_eq!(NativeProviderKind::from_normalized(id), Some(expected));
    }
    // Native providers with bespoke drivers are intentionally not generic.
    assert_eq!(NativeProviderKind::from_normalized("claude"), None);
    assert_eq!(NativeProviderKind::from_normalized("antigravity"), None);
    assert_eq!(NativeProviderKind::from_normalized("openrouter"), None);
}

#[test]
fn native_provider_specs_are_self_consistent() {
    // Every generic kind's spec must carry a switch_prefix that the wiring
    // contract's api_method-derived routes will satisfy, and a stable id.
    for kind in [
        NativeProviderKind::OpenAi,
        NativeProviderKind::Gemini,
        NativeProviderKind::Cursor,
        NativeProviderKind::Copilot,
        NativeProviderKind::Bedrock,
        NativeProviderKind::Jcode,
        NativeProviderKind::Azure,
    ] {
        let spec = kind.spec();
        assert!(!spec.provider_id.is_empty(), "{kind:?} has empty id");
        assert!(!spec.label.is_empty(), "{kind:?} has empty label");
        if kind == NativeProviderKind::Jcode {
            assert!(
                spec.contract.switch_prefix.is_empty(),
                "Jcode switches must use bare managed model ids"
            );
        } else {
            assert!(
                spec.contract.switch_prefix.ends_with(':'),
                "{kind:?} switch_prefix must end with ':'"
            );
        }
        // Round-trips through the id map.
        assert_eq!(
            NativeProviderKind::from_normalized(spec.provider_id),
            Some(kind),
            "{kind:?} id does not round-trip"
        );
    }
}

#[test]
fn spend_accumulates_openai_style_usage_and_cost() {
    let mut spend = DoctorSpend::default();
    spend.accumulate(
        Some(&serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 40,
            "total_tokens": 140
        })),
        Some(&serde_json::json!(0.0012)),
    );
    spend.accumulate(
        Some(&serde_json::json!({
            "prompt_tokens": 10,
            "completion_tokens": 5
        })),
        None,
    );
    assert_eq!(spend.billable_calls, 2);
    assert_eq!(spend.prompt_tokens, 110);
    assert_eq!(spend.completion_tokens, 45);
    // Second call has no total_tokens, so it is derived as prompt+completion.
    assert_eq!(spend.total_tokens, 155);
    assert!(spend.has_token_data);
    assert_eq!(spend.reported_cost_usd, Some(0.0012));
    assert!(spend.human_summary().contains("155 tokens"));
    assert!(spend.human_summary().contains("$0.001200"));
}

#[test]
fn spend_handles_missing_usage_and_anthropic_style_keys() {
    let mut spend = DoctorSpend::default();
    // No usage at all (e.g. provider that omits it).
    spend.accumulate(None, None);
    assert_eq!(spend.billable_calls, 1);
    assert!(!spend.has_token_data);
    assert!(
        spend
            .human_summary()
            .contains("token usage not reported by provider")
    );

    // Anthropic-style input_tokens/output_tokens.
    spend.accumulate(
        Some(&serde_json::json!({"input_tokens": 7, "output_tokens": 3})),
        None,
    );
    assert_eq!(spend.prompt_tokens, 7);
    assert_eq!(spend.completion_tokens, 3);
    assert_eq!(spend.total_tokens, 10);
    assert!(spend.has_token_data);
}

#[test]
fn native_doctor_supports_claude_and_antigravity() {
    assert!(native_doctor_supports_provider("claude"));
    assert!(native_doctor_supports_provider("anthropic"));
    assert!(native_doctor_supports_provider("antigravity"));
    // OpenAI-compatible profiles are driven by the generic doctor, not the
    // native path.
    assert!(!native_doctor_supports_provider("openrouter"));
    assert!(!native_doctor_supports_provider(
        "definitely-not-a-provider"
    ));
}

/// `native_doctor_supports_provider` lives in `jcode_base::auth::doctor`
/// (so base's `live_tests` roster can call it) while the drivers live
/// here. Keep the base predicate in sync with the driver roster: every
/// generic `NativeProviderKind` id plus the bespoke claude/antigravity
/// drivers must be accepted, and nothing else native-flavored.
#[test]
fn native_provider_roster_matches_base_predicate() {
    for kind in [
        NativeProviderKind::OpenAi,
        NativeProviderKind::Gemini,
        NativeProviderKind::Cursor,
        NativeProviderKind::Copilot,
        NativeProviderKind::Bedrock,
        NativeProviderKind::Jcode,
        NativeProviderKind::Azure,
    ] {
        let id = kind.spec().provider_id;
        assert!(
            native_doctor_supports_provider(id),
            "base predicate rejects generic native driver id {id:?}"
        );
    }
    for id in ["claude", "antigravity"] {
        assert!(
            native_doctor_supports_provider(id),
            "base predicate rejects bespoke native driver id {id:?}"
        );
    }
}

#[test]
fn native_antigravity_contract_routes_via_https_prefix() {
    let contract = native_antigravity_wiring_contract();
    assert_eq!(contract.api_method, "https");
    assert_eq!(contract.route_provider, "Antigravity");
    assert_eq!(contract.expected_runtime, "antigravity");
    assert!(contract.expected_namespace.is_none());
    assert_eq!(contract.switch_prefix, "antigravity:");
}

#[test]
fn native_jcode_contract_uses_managed_subscription_identity() {
    let contract = NativeProviderKind::Jcode.spec().contract;
    assert_eq!(
        contract.api_method,
        jcode_base::subscription_catalog::JCODE_ROUTE_API_METHOD
    );
    assert_eq!(
        contract.route_provider,
        jcode_base::subscription_catalog::JCODE_PROVIDER_DISPLAY_NAME
    );
    assert_eq!(contract.expected_runtime, "jcode");
    assert!(contract.expected_namespace.is_none());
    assert!(contract.switch_prefix.is_empty());
}

#[test]
fn cheapest_antigravity_model_prefers_gemini_flash() {
    let catalog = vec![
        "claude-opus-4-6-thinking".to_string(),
        "gemini-3.1-pro-high".to_string(),
        "gemini-3-flash".to_string(),
        "gpt-oss-120b-medium".to_string(),
    ];
    assert_eq!(
        cheapest_antigravity_model(&catalog).as_deref(),
        Some("gemini-3-flash")
    );
}

#[test]
fn cheapest_antigravity_model_falls_back_to_any_gemini_then_any_model() {
    // No flash tier: any Gemini wins.
    let gemini_only = vec![
        "claude-sonnet-4-6".to_string(),
        "gemini-3.1-pro-low".to_string(),
    ];
    assert_eq!(
        cheapest_antigravity_model(&gemini_only).as_deref(),
        Some("gemini-3.1-pro-low")
    );
    // No Gemini at all: first non-alias model wins.
    let no_gemini = vec!["default".to_string(), "claude-sonnet-4-6".to_string()];
    assert_eq!(
        cheapest_antigravity_model(&no_gemini).as_deref(),
        Some("claude-sonnet-4-6")
    );
    // Only the alias: nothing usable.
    let alias_only = vec!["default".to_string()];
    assert!(cheapest_antigravity_model(&alias_only).is_none());
}

#[test]
fn native_antigravity_auth_is_secret_free() {
    let with_account = native_antigravity_auth("user@example.com");
    // The source mentions the account but never carries a secret fingerprint.
    assert!(with_account.source.contains("user@example.com"));
    let anonymous = native_antigravity_auth("");
    assert!(anonymous.source.contains("Antigravity Google OAuth"));
}

#[test]
fn tool_stage_detail_surfaces_multi_and_parallel_phases() {
    let verified = LiveVerificationStage::passed(checkpoints::TOOL_CALL_PARSE)
        .with_evidence("multi_tool_replay", serde_json::json!("verified"))
        .with_evidence("parallel_tool_calls", serde_json::json!("verified"));
    let detail = tool_stage_detail(&verified);
    assert!(detail.contains("tool call parsed and executed"));
    assert!(detail.contains("multi-call signature replay verified"));
    assert!(detail.contains("parallel tool calls verified"));

    let skipped = LiveVerificationStage::passed(checkpoints::TOOL_CALL_PARSE)
        .with_evidence("multi_tool_replay", serde_json::json!("skipped"))
        .with_evidence("parallel_tool_calls", serde_json::json!("skipped"));
    let detail = tool_stage_detail(&skipped);
    assert!(detail.contains("multi-call signature replay skipped"));
    assert!(detail.contains("parallel tool calls skipped"));

    // With no evidence the base string is unchanged (back-compat).
    let bare = LiveVerificationStage::passed(checkpoints::TOOL_CALL_PARSE);
    assert_eq!(tool_stage_detail(&bare), "tool call parsed and executed");
}

#[test]
fn reasoning_stage_detail_describes_each_classification() {
    for (value, needle) in [
        ("streamed", "reasoning streamed"),
        ("opaque", "reasoning hidden but signaled"),
        ("none", "no reasoning signal observed"),
    ] {
        let stage = LiveVerificationStage::passed(checkpoints::REASONING_CAPABILITY)
            .with_evidence("reasoning_capability", serde_json::json!(value));
        assert!(
            reasoning_stage_detail(&stage).contains(needle),
            "classification {value} should mention {needle}"
        );
    }
}

#[test]
fn push_reasoning_check_records_pass_for_clean_turn() {
    let mut checks = Vec::new();
    let mut spend = DoctorSpend::default();
    let stage = LiveVerificationStage::passed(checkpoints::REASONING_CAPABILITY)
        .with_evidence("reasoning_capability", serde_json::json!("opaque"));
    push_reasoning_check(Ok(stage), &mut checks, &mut spend);
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].checkpoint, checkpoints::REASONING_CAPABILITY);
    assert_eq!(checks[0].status, LiveVerificationStageStatus::Passed);
    assert!(!checks[0].is_failure());
}

#[test]
fn push_reasoning_check_skips_never_fails_on_probe_error() {
    // The observe-only reasoning checkpoint must never produce a failure that
    // could flip the tier to not-ready; a probe error is recorded as skipped.
    let mut checks = Vec::new();
    let mut spend = DoctorSpend::default();
    push_reasoning_check(
        Err(anyhow::anyhow!("network blip")),
        &mut checks,
        &mut spend,
    );
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].status, LiveVerificationStageStatus::Skipped);
    assert!(!checks[0].is_failure());
}
