#[tokio::test]
async fn test_context_guard_small_output_passes_through() {
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(200_000)));
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let output = ToolOutput::new("small output");
    let result = registry.guard_context_overflow("test", output, false).await;
    assert_eq!(result.output, "small output");
}
#[tokio::test]
async fn test_context_guard_withholds_huge_single_output_by_default() {
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(1000)));
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    // 30% of 1000 = 300 tokens = 1200 chars max for a single output
    // Create output that's way larger
    let big_output = "x".repeat(8000); // 2000 tokens, well over 30% of 1000
    let output = ToolOutput::new(big_output.clone());
    let result = registry.guard_context_overflow("test", output, false).await;

    // The whole point of the refusal: none of the payload is spent.
    assert!(
        !result.output.contains(&"x".repeat(100)),
        "withheld output must not leak the payload"
    );
    assert!(
        result.output.contains("OUTPUT WITHHELD"),
        "should say the output was withheld, got: {}",
        result.output
    );
    assert!(
        result.output.contains("accept_large_output"),
        "should name the opt-in flag so the caller can retry"
    );
    // A refusal that costs as much as the payload would defeat itself.
    assert!(
        result.output.len() < 1200,
        "refusal should be cheap, was {} chars",
        result.output.len()
    );
}
#[tokio::test]
async fn test_context_guard_returns_truncated_output_when_caller_accepts() {
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(1000)));
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let big_output = "x".repeat(8000);
    let output = ToolOutput::new(big_output.clone());
    let result = registry.guard_context_overflow("test", output, true).await;

    assert!(
        result.output.len() < big_output.len(),
        "opt-in still truncates to what the budget allows"
    );
    assert!(
        result.output.contains("TRUNCATED"),
        "should say the output was truncated, got: {}",
        result.output
    );
    assert!(
        result.output.starts_with(&"x".repeat(200)),
        "opt-in must actually return the payload prefix"
    );
}
#[tokio::test]
async fn test_context_guard_reports_the_real_cost_and_affordable_size() {
    // 200k budget, 40k already used. A 90k-token result is over the 30%
    // single-output ceiling (60k), so it is withheld. The quoted numbers must
    // match the actual arithmetic, since the caller decides based on them.
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(200_000)));
    {
        let mut mgr = compaction.write().await;
        mgr.update_observed_input_tokens(40_000);
    }
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let output = ToolOutput::new("x".repeat(360_000)); // ~90k tokens
    let result = registry.guard_context_overflow("test", output, false).await;

    assert!(result.output.contains("OUTPUT WITHHELD"));
    assert!(
        result.output.contains("90k tokens"),
        "should quote the real output size, got: {}",
        result.output
    );
    assert!(
        result.output.contains("45%"),
        "should quote the share of budget (90k of 200k), got: {}",
        result.output
    );
    assert!(
        result.output.contains("200k context budget"),
        "should quote the budget, got: {}",
        result.output
    );
    assert!(
        result.output.contains("40k is already used"),
        "should quote context already spent, got: {}",
        result.output
    );
    assert!(
        result.output.contains("50k"),
        "should quote the affordable size, now bounded by the absolute \
         single-output ceiling rather than 30% of the budget, got: {}",
        result.output
    );
}
#[tokio::test]
async fn test_context_guard_truncates_when_context_nearly_full() {
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(10_000)));
    {
        let mut mgr = compaction.write().await;
        mgr.update_observed_input_tokens(9500); // 95% full
    }
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    // Even a modest output should get truncated when context is 95% full
    let output = ToolOutput::new("x".repeat(4000)); // 1000 tokens
    let result = registry.guard_context_overflow("test", output, false).await;
    assert!(
        result.output.contains("WITHHELD") || result.output.contains("CONTEXT LIMIT"),
        "Should warn about context limits when nearly full"
    );
}
#[tokio::test]
async fn test_context_guard_still_refuses_when_context_is_exhausted() {
    // With almost no room left there is nothing to spend, so accepting the cost
    // cannot buy anything. The opt-in must not become a way to blow past the
    // window entirely.
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(10_000)));
    {
        let mut mgr = compaction.write().await;
        mgr.update_observed_input_tokens(9_990);
    }
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let payload = "x".repeat(400_000);
    let result = registry
        .guard_context_overflow("test", ToolOutput::new(payload.clone()), true)
        .await;
    assert!(
        result.output.len() < 2_000,
        "exhausted context must not return the payload, got {} chars",
        result.output.len()
    );
    assert!(
        result.output.contains("CONTEXT LIMIT REACHED"),
        "should report the hard limit, got: {}",
        result.output
    );
}
#[tokio::test]
async fn test_context_guard_zero_budget_passes_through() {
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(0)));
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let output = ToolOutput::new("x".repeat(100_000));
    let result = registry.guard_context_overflow("test", output, false).await;
    assert_eq!(
        result.output.len(),
        100_000,
        "Zero budget should pass through"
    );
}
#[test]
fn test_accepts_large_output_requires_an_unambiguous_yes() {
    use super::accepts_large_output;

    assert!(accepts_large_output(
        &serde_json::json!({ "accept_large_output": true })
    ));
    // Models routinely stringify booleans, so accept the string spelling too.
    assert!(accepts_large_output(
        &serde_json::json!({ "accept_large_output": "true" })
    ));
    assert!(accepts_large_output(
        &serde_json::json!({ "accept_large_output": "TRUE" })
    ));

    // Everything else means no. Spending the rest of the window should never
    // happen because of a truthy-looking value.
    for input in [
        serde_json::json!({}),
        serde_json::json!({ "accept_large_output": false }),
        serde_json::json!({ "accept_large_output": "false" }),
        serde_json::json!({ "accept_large_output": 1 }),
        serde_json::json!({ "accept_large_output": "yes" }),
        serde_json::json!({ "accept_large_output": serde_json::Value::Null }),
        serde_json::json!({ "query": "accept_large_output" }),
    ] {
        assert!(
            !accepts_large_output(&input),
            "should not opt in for {input}"
        );
    }
}
#[tokio::test]
async fn test_context_guard_never_spends_more_than_it_reports() {
    // State-space sweep over budget, fill level, and payload size. Two
    // invariants must hold in every combination, because the whole point of the
    // guard is that a caller can trust the accounting:
    //   1. Without the opt-in, the returned text is small. Refusing has to be
    //      cheap or it reproduces the bug it prevents.
    //   2. The returned text never exceeds the remaining safety headroom, with
    //      or without the opt-in. Otherwise "accept the cost" would silently
    //      overrun the window.
    for budget in [10_000usize, 50_000, 200_000] {
        for fill_percent in [0usize, 25, 50, 80, 89, 95] {
            for payload_tokens in [1usize, 500, 5_000, 100_000] {
                for accept in [false, true] {
                    let compaction =
                        Arc::new(RwLock::new(CompactionManager::new().with_budget(budget)));
                    let used = budget * fill_percent / 100;
                    if used > 0 {
                        let mut mgr = compaction.write().await;
                        mgr.update_observed_input_tokens(used as u64);
                    }
                    let registry = Registry {
                        tools: Arc::new(RwLock::new(HashMap::new())),
                        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
                        compaction,
                        lsp_pool: None,
                    };

                    let payload = "x".repeat(payload_tokens * 4);
                    let result = registry
                        .guard_context_overflow("test", ToolOutput::new(payload.clone()), accept)
                        .await;
                    let returned_tokens = result.output.len() / 4;

                    let threshold = (budget as f32 * 0.90) as usize;
                    let headroom = threshold.saturating_sub(used);
                    let passed_through = result.output == payload;

                    if !accept && !passed_through {
                        assert!(
                            result.output.len() < 1_500,
                            "refusal must stay cheap: budget={budget} fill={fill_percent} \
                             payload={payload_tokens} returned {} chars",
                            result.output.len()
                        );
                    }

                    // Allow a small slack for the notice text appended after the slice.
                    assert!(
                        returned_tokens <= headroom.max(1_000) + 500,
                        "returned ~{returned_tokens}k tokens with only {headroom} headroom: \
                         budget={budget} fill={fill_percent} payload={payload_tokens} \
                         accept={accept}"
                    );
                }
            }
        }
    }
}
#[tokio::test]
async fn test_context_guard_refusal_reads_clearly_for_todays_regression() {
    // The exact shape that motivated this change: a 233k-token agentgrep result
    // against a 200k budget with 18k already used. Printed so the wording stays
    // reviewable, and asserted so it keeps naming the cost and the escape hatch.
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(200_000)));
    {
        let mut mgr = compaction.write().await;
        mgr.update_observed_input_tokens(18_000);
    }
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    let result = registry
        .guard_context_overflow("agentgrep", ToolOutput::new("x".repeat(932_000)), false)
        .await;
    println!("---\n{}\n---", result.output);

    assert!(result.output.contains("233k tokens"));
    assert!(result.output.contains("116%"), "got: {}", result.output);
    assert!(result.output.contains("18k is already used"));
    assert!(result.output.contains("accept_large_output"));
    assert!(result.output.contains("paths_only"));
}
#[tokio::test]
async fn test_execute_withholds_oversized_output_by_default() {
    // The guard is only useful if it runs on the real call path. Every other
    // test calls guard_context_overflow directly, which would still pass if the
    // flag were never plumbed through execute().
    let output = execute_big_output(serde_json::json!({ "intent": "test" })).await;
    assert!(
        output.contains("OUTPUT WITHHELD"),
        "execute() must apply the guard, got: {output}"
    );
    assert!(
        output.len() < 1_500,
        "withheld output should be cheap, got {} chars",
        output.len()
    );
}
#[tokio::test]
async fn test_execute_honors_accept_large_output_from_raw_input() {
    // Proves the flag survives the trip through execute(): the tool itself never
    // declares or reads `accept_large_output`, so this only works because the
    // registry reads it off the raw input.
    let output =
        execute_big_output(serde_json::json!({ "intent": "test", "accept_large_output": true }))
            .await;
    assert!(
        output.contains("OUTPUT TRUNCATED"),
        "opt-in should return truncated payload, got: {}",
        &output[..output.len().min(200)]
    );
    assert!(
        output.starts_with(&"x".repeat(200)),
        "opt-in must actually return payload"
    );
}
#[tokio::test]
async fn test_execute_ignores_a_non_boolean_accept_flag() {
    // A truthy-looking value must not spend the window.
    let output =
        execute_big_output(serde_json::json!({ "intent": "test", "accept_large_output": 1 })).await;
    assert!(
        output.contains("OUTPUT WITHHELD"),
        "numeric 1 must not opt in, got: {}",
        &output[..output.len().min(200)]
    );
}
#[tokio::test]
async fn test_every_tool_advertises_the_large_output_escape_hatch() {
    // The guard applies to every tool, so every tool must document the way out.
    // Asserted over the real definition list rather than per tool, because the
    // failure mode is a new tool nobody remembered to annotate.
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = Registry::new(provider).await;
    registry.register_ambient_tools().await;

    let defs = registry.definitions(None).await;
    assert!(
        defs.len() > 20,
        "expected the full tool set, got {}",
        defs.len()
    );

    let mut missing = Vec::new();
    for def in &defs {
        let flag = &def.input_schema["properties"][jcode_tool_core::ACCEPT_LARGE_OUTPUT_KEY];
        if flag.get("type").and_then(Value::as_str) != Some("boolean") {
            missing.push(def.name.clone());
        }
        // Advertising it as required would force the model to answer a question
        // about token budgets on every single call.
        if let Some(required) = def.input_schema["required"].as_array() {
            assert!(
                !required
                    .iter()
                    .any(|v| v.as_str() == Some(jcode_tool_core::ACCEPT_LARGE_OUTPUT_KEY)),
                "{} must not require accept_large_output",
                def.name
            );
        }
    }
    assert!(
        missing.is_empty(),
        "tools missing the accept_large_output escape hatch: {missing:?}"
    );
}
#[tokio::test]
async fn test_large_output_flag_costs_little_across_the_whole_tool_set() {
    // Adding a property to every schema is paid on every request, forever. Keep
    // the total honest: ~20 tokens per tool is acceptable, a paragraph is not.
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = Registry::new(provider).await;
    registry.register_ambient_tools().await;
    let defs = registry.definitions(None).await;

    let property =
        serde_json::to_string(&crate::tool::accept_large_output_schema_property_for_test())
            .expect("serializable");
    let per_tool = crate::util::estimate_tokens(&property);
    let total = per_tool * defs.len();

    assert!(
        per_tool <= 25,
        "per-tool cost {per_tool} tokens is too high: {property}"
    );
    assert!(
        total < 1_500,
        "{} tools x {per_tool} tokens = {total} tokens of permanent prompt overhead",
        defs.len()
    );
}
#[tokio::test]
async fn test_batch_guards_both_its_subcalls_and_its_own_aggregate() {
    // Batch is how oversized results actually arrive in practice: several
    // searches fan out at once. Two separate guard applications matter here, and
    // the aggregate one is the load-bearing case: batch concatenates every
    // sub-result, so even if each sub-call were individually acceptable the
    // combined output can blow the window. That aggregate is what withheld
    // today's regression.
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let registry = Registry::new(provider).await;
    {
        let mut mgr = registry.compaction.write().await;
        *mgr = CompactionManager::new().with_budget(10_000);
    }
    registry
        .register(
            "big_output".to_string(),
            Arc::new(BigOutputTool { chars: 400_000 }),
        )
        .await;

    let ctx = |name: &str| ToolContext {
        session_id: format!("test-batch-context-guard-{name}"),
        message_id: "test".to_string(),
        tool_call_id: "test".to_string(),
        working_dir: Some(std::env::temp_dir()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::Direct,
    };
    let calls = serde_json::json!([
        { "tool": "big_output", "intent": "one" },
        { "tool": "big_output", "intent": "two" },
    ]);

    // Without an opt-in anywhere, nothing large escapes: no payload reaches the
    // transcript, only the refusal.
    let withheld = registry
        .execute(
            "batch",
            serde_json::json!({ "intent": "test", "tool_calls": calls }),
            ctx("withheld"),
        )
        .await
        .expect("batch should succeed")
        .output;
    assert!(
        withheld.contains("OUTPUT WITHHELD"),
        "batch output must be guarded, got: {}",
        &withheld[..withheld.len().min(300)]
    );
    assert!(
        !withheld.contains(&"x".repeat(100)),
        "no payload should survive when nothing opted in"
    );

    // Opting in at the batch level returns the aggregate, which is the level a
    // caller reads. The sub-calls' own refusals are inside it, since each was
    // guarded separately and neither sub-call opted in.
    let accepted = registry
        .execute(
            "batch",
            serde_json::json!({
                "intent": "test",
                "accept_large_output": true,
                "tool_calls": calls,
            }),
            ctx("accepted"),
        )
        .await
        .expect("batch should succeed")
        .output;
    // The aggregate is now returned rather than withheld: it carries the
    // per-subcall section headers, which the withheld version never reaches.
    assert!(
        !accepted.starts_with("⚠️ OUTPUT WITHHELD"),
        "batch-level opt-in should return the aggregate, got: {}",
        &accepted[..accepted.len().min(200)]
    );
    assert!(
        accepted.contains("--- [1] big_output ---"),
        "aggregate should contain per-subcall sections, got: {}",
        &accepted[..accepted.len().min(300)]
    );
    assert!(
        accepted.matches("OUTPUT WITHHELD").count() >= 1,
        "each sub-call is guarded on its own; neither opted in"
    );
}
#[tokio::test]
async fn test_guard_withholds_large_output_on_a_million_token_window() {
    // The regression that made every other test in this file misleading. They
    // all pinned budgets of 1k to 200k, where 30% of the budget is a small
    // number. Production reported a 1M-token window, so 30% permitted a 300k
    // single result and a repo-wide grep costing 233k tokens sailed straight
    // through a guard that had unit tests passing.
    let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(1_000_000)));
    {
        let mut mgr = compaction.write().await;
        mgr.update_observed_input_tokens(21_000);
    }
    let registry = Registry {
        tools: Arc::new(RwLock::new(HashMap::new())),
        skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
        compaction,
        lsp_pool: None,
    };

    // ~233k tokens: the real size of the agentgrep result that started this.
    let output = ToolOutput::new("x".repeat(932_000));
    let result = registry
        .guard_context_overflow("agentgrep", output, false)
        .await;

    assert!(
        result.output.contains("OUTPUT WITHHELD"),
        "a 233k-token result must be withheld even on a 1M window, got: {}",
        &result.output[..result.output.len().min(200)]
    );
    assert!(
        result.output.len() < 1_500,
        "refusal should cost ~120 tokens, not {} chars",
        result.output.len()
    );
}
#[tokio::test]
async fn test_single_output_ceiling_is_absolute_not_only_proportional() {
    // Guards the invariant directly: however large the window, one tool result
    // may never exceed the absolute ceiling. Without this, raising a model's
    // advertised context window silently raises the per-call blast radius.
    for budget in [200_000usize, 1_000_000, 2_000_000, 10_000_000] {
        let compaction = Arc::new(RwLock::new(CompactionManager::new().with_budget(budget)));
        let registry = Registry {
            tools: Arc::new(RwLock::new(HashMap::new())),
            skills: Arc::new(RwLock::new(crate::skill::SkillRegistry::default())),
            compaction,
            lsp_pool: None,
        };

        // Just over the absolute ceiling, but a trivial fraction of a huge window.
        let over_ceiling_tokens = Registry::SINGLE_OUTPUT_MAX_TOKENS + 10_000;
        let result = registry
            .guard_context_overflow(
                "test",
                ToolOutput::new("x".repeat(over_ceiling_tokens * 4)),
                false,
            )
            .await;
        assert!(
            result.output.contains("OUTPUT WITHHELD"),
            "budget={budget}: {over_ceiling_tokens} tokens must exceed the absolute ceiling"
        );
    }
}
