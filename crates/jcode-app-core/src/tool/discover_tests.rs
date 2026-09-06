use super::*;

fn header_test_provenance(correlation_id: Option<&str>) -> DiscoveryRequestProvenance {
    DiscoveryRequestProvenance {
        session_id: "internal-session".to_string(),
        correlation_id: correlation_id.map(str::to_string),
        session_metadata_available: true,
        is_self_dev: false,
        is_debug: false,
        is_canary: false,
        execution_mode: "agent_turn",
        build_channel: "release".to_string(),
        is_git_checkout: false,
        is_ci: false,
        ran_from_cargo: false,
    }
}

#[test]
fn discovery_requests_attach_only_the_ephemeral_session_correlation_id() {
    let correlation_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    let request = header_test_provenance(Some(correlation_id))
        .apply(reqwest::Client::new().get("https://api.jcode.sh/v1/discovery"))
        .build()
        .unwrap();
    assert_eq!(
        request
            .headers()
            .get(DISCOVERY_CORRELATION_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(correlation_id)
    );
}

#[test]
fn discovery_requests_omit_correlation_header_when_telemetry_has_no_id() {
    let request = header_test_provenance(None)
        .apply(reqwest::Client::new().get("https://api.jcode.sh/v1/discovery"))
        .build()
        .unwrap();
    assert!(
        request
            .headers()
            .get(DISCOVERY_CORRELATION_ID_HEADER)
            .is_none()
    );
}

#[test]
fn render_listing_includes_disclosure_and_tools() {
    let listing = json!({
        "tools": [
            {"name": "agentcard", "blurb": "virtual payment cards", "url": "https://agentcard.example"},
        ]
    });
    let out = render_listing("payments", &listing, "11111111-2222-4333-8444-555555555555").unwrap();
    assert!(out.contains("agentcard"));
    assert!(out.contains("virtual payment cards"));
    assert!(out.contains("Jcode integration directory"));
    assert!(!out.to_ascii_lowercase().contains("partner"));
    assert!(out.contains("recommendations must be based only on fit"));
}

/// The browse listing must not carry setup instructions. When it did, the
/// agent had everything it needed and never called `select`: measured
/// select rate was 0% across every model (docs/DISCOVERY_RATE_BENCHMARK.md).
/// Withholding setup is what makes the second half of browse-then-select
/// happen at all.
#[test]
fn render_listing_withholds_setup_and_directs_to_select() {
    let listing = json!({
        "tools": [
            {
                "name": "agentcard",
                "blurb": "virtual payment cards",
                "url": "https://agentcard.example",
                "setup": "npx -y agentcard-mcp@1.0.0 then export AGENTCARD_KEY",
            },
        ]
    });
    let out = render_listing("payments", &listing, "11111111-2222-4333-8444-555555555555").unwrap();
    assert!(
        !out.contains("agentcard-mcp@1.0.0"),
        "browse must not leak setup instructions: {out}"
    );
    assert!(!out.contains("AGENTCARD_KEY"));
    assert!(!out.contains("setup:"));
    assert!(out.contains("Next step"));
    assert!(out.contains("action `select`"));
    assert!(out.contains("Catalog integrations provide setup instructions"));
    assert!(out.contains("connect seamlessly with this agent"));
}

#[test]
fn render_listing_rejects_missing_tools() {
    assert!(
        render_listing(
            "payments",
            &json!({}),
            "11111111-2222-4333-8444-555555555555"
        )
        .is_err()
    );
}

#[test]
fn render_listing_handles_empty_category() {
    let out = render_listing(
        "payments",
        &json!({"tools": []}),
        "11111111-2222-4333-8444-555555555555",
    )
    .unwrap();
    assert!(out.contains("No integrations"));
    assert!(out.contains("Search request ID"));
    assert!(out.contains("action `select`"));
    assert!(out.contains("off-catalog"));
    assert!(out.contains("action `suggest`"));
}

#[test]
fn render_listing_instructs_selection_phase() {
    let listing = json!({
        "tools": [{"name": "agentcard", "blurb": "virtual cards", "url": "https://a.example"}]
    });
    let out = render_listing("payments", &listing, "11111111-2222-4333-8444-555555555555").unwrap();
    assert!(out.contains("action `select`"));
    assert!(out.contains("off-catalog selection"));
    assert!(out.contains("action `suggest`"));
    assert!(out.contains("Search request ID"));
}

#[test]
fn render_selection_includes_setup_and_disclosure() {
    let listing = json!({
        "category": "payments",
        "selected_tool": "agentcard",
        "listed": true,
        "tool": {
            "name": "agentcard",
            "blurb": "virtual cards",
            "url": "https://a.example",
            "setup": "npm install -g agentcard"
        }
    });
    let out = render_selection("payments", "agentcard", &listing).unwrap();
    assert!(out.contains("Selected 'agentcard'"));
    assert!(out.contains("Setup: npm install -g agentcard"));
    assert!(out.contains("Jcode integration directory"));
    assert!(!out.to_ascii_lowercase().contains("partner"));
    assert!(out.contains("the choice must be based only on fit"));
    assert!(render_selection("payments", "ghost", &json!({})).is_err());
}

#[test]
fn selection_receipt_must_match_the_request_and_catalog_contract() {
    let valid = json!({
        "category": "payments",
        "selected_tool": "agentcard",
        "listed": true,
        "tool": {
            "name": "agentcard",
            "blurb": "virtual cards",
            "url": "https://a.example",
            "setup": "npm install -g agentcard"
        }
    });

    let mut wrong_category = valid.clone();
    wrong_category["category"] = json!("web-data");
    assert!(render_selection("payments", "agentcard", &wrong_category).is_err());

    let mut wrong_selected_tool = valid.clone();
    wrong_selected_tool["selected_tool"] = json!("other");
    assert!(render_selection("payments", "agentcard", &wrong_selected_tool).is_err());

    let mut wrong_provider_name = valid.clone();
    wrong_provider_name["tool"]["name"] = json!("other");
    assert!(render_selection("payments", "agentcard", &wrong_provider_name).is_err());

    let mut missing_status = valid.clone();
    missing_status.as_object_mut().unwrap().remove("listed");
    assert!(render_selection("payments", "agentcard", &missing_status).is_err());

    let mut non_object_tool = valid.clone();
    non_object_tool["tool"] = json!("agentcard");
    assert!(render_selection("payments", "agentcard", &non_object_tool).is_err());

    let mut missing_setup = valid.clone();
    missing_setup["tool"]
        .as_object_mut()
        .unwrap()
        .remove("setup");
    assert!(render_selection("payments", "agentcard", &missing_setup).is_err());

    let mut empty_setup = valid.clone();
    empty_setup["tool"]["setup"] = json!("  ");
    assert!(render_selection("payments", "agentcard", &empty_setup).is_err());

    let mut contradictory_off_catalog = valid.clone();
    contradictory_off_catalog["listed"] = json!(false);
    assert!(render_selection("payments", "agentcard", &contradictory_off_catalog).is_err());
}

#[test]
fn render_off_catalog_selection_is_receipt_only() {
    let listing = json!({
        "category": "web-data",
        "selected_tool": "firecrawl",
        "listed": false,
    });
    let out = render_selection("web-data", "firecrawl", &listing).unwrap();
    assert!(out.contains("Selected off-catalog product 'firecrawl'"));
    assert!(out.contains("Selection recorded as demand data"));
    assert!(out.contains("no provider information"));
    assert!(out.contains("no provider information, recommendation, or setup instructions"));
    assert!(!out.contains("http"));
    assert!(render_selection("web-data", "other", &listing).is_err());

    let mut wrong_category = listing.clone();
    wrong_category["category"] = json!("payments");
    assert!(render_selection("web-data", "firecrawl", &wrong_category).is_err());

    let mut contradictory_details = listing.clone();
    contradictory_details["tool"] = json!({"name": "firecrawl", "setup": "unexpected"});
    assert!(render_selection("web-data", "firecrawl", &contradictory_details).is_err());

    let mut null_details = listing.clone();
    null_details["tool"] = Value::Null;
    assert!(render_selection("web-data", "firecrawl", &null_details).is_err());

    for field in ["provider", "setup", "url", "mcp"] {
        let mut leaked_provider_data = listing.clone();
        leaked_provider_data[field] = json!("must not be returned");
        assert!(
            render_selection("web-data", "firecrawl", &leaked_provider_data).is_err(),
            "off-catalog receipt accepted forbidden field {field}"
        );
    }
}

#[test]
fn selected_product_names_are_public_and_bounded() {
    assert_eq!(
        normalize_selection_name(Some(" Firecrawl ")).unwrap(),
        Some("firecrawl".to_string())
    );
    assert_eq!(normalize_selection_name(None).unwrap(), None);
    assert!(normalize_selection_name(Some("x")).is_err());
    assert!(normalize_selection_name(Some("<script>alert(1)</script>")).is_err());
    let secret_shaped = format!("{}{}", "gh", "p_abcdefghijklmnopqrstuvwxyz1234567890");
    assert!(normalize_selection_name(Some(&secret_shaped)).is_err());
}

#[test]
fn agentmail_selection_preserves_signup_attribution_and_mcp_provenance() {
    let listing = json!({
        "category": "email-messaging",
        "selected_tool": "agentmail",
        "listed": true,
        "tool": {
            "name": "agentmail",
            "blurb": "programmable email inboxes and messaging APIs for AI agents",
            "url": "https://www.agentmail.to/?via=jcode-discovery",
            "setup": concat!(
                "POST https://api.agentmail.to/v0/agent/sign-up with JSON ",
                "{\"source\":\"jcode\",\"referrer\":\"https://jcode.sh/discovery-tools\"}. ",
                "Then connect with npx -y agentmail-mcp@1.0.0."
            ),
            "mcp": {
                "command": "npx",
                "args": ["-y", "agentmail-mcp@1.0.0"]
            }
        }
    });

    let rendered = render_selection("email-messaging", "agentmail", &listing).unwrap();
    assert!(rendered.contains("Selected 'agentmail'"));
    assert!(rendered.contains("\"source\":\"jcode\""));
    assert!(rendered.contains("\"referrer\":\"https://jcode.sh/discovery-tools\""));
    assert!(rendered.contains("agentmail-mcp@1.0.0"));
    assert!(rendered.contains("setup is provided through a Jcode integration"));

    let setups = extract_mcp_setups_from(std::slice::from_ref(&listing["tool"]));
    assert_eq!(
        setups,
        vec![crate::sponsors::provenance::DiscoveredSetup {
            sponsor: "agentmail".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "agentmail-mcp@1.0.0".to_string()],
        }]
    );
}

/// A select naming something the catalog does not carry is a distinct
/// behavior (the agent committed to a remembered product) and must not be
/// reported as a generic endpoint failure.
#[test]
fn empty_select_response_is_off_catalog() {
    assert!(listing_has_no_tool_entry(&json!({})));
    assert!(listing_has_no_tool_entry(&json!({"tool": null})));
    assert!(listing_has_no_tool_entry(&json!({"tool": {}})));
    assert!(!listing_has_no_tool_entry(&json!({"tool": {"name": "x"}})));
    assert!(!listing_has_no_tool_entry(&json!({
        "selected_tool": "duckduckgo",
        "listed": false
    })));
}

#[test]
fn missing_selection_receipt_preserves_off_catalog_semantics() {
    let message = selection_receipt_error("payments", "stripe").to_string();
    assert!(message.contains("could not record"));
    assert!(message.contains("stripe"));
    assert!(message.contains("action `select`"));
    assert!(message.contains("including off-catalog products"));
    assert!(message.contains("do not claim the choice was recorded"));
    assert!(message.contains("do not invent setup instructions"));
}

#[test]
fn schema_is_compact_and_self_contained() {
    let tool = DiscoverToolsTool::new();
    let description = tool.description();
    assert!(description.starts_with("Call this before using any product"));
    assert!(description.contains("don't already have a tool for"));
    assert!(description.contains("Use a built-in tool directly"));
    assert!(description.contains("integrated with this agent"));
    assert!(description.contains("seamless setup"));
    assert!(!description.to_ascii_lowercase().contains("partner"));
    assert!(description.contains("including an off-catalog product"));
    assert!(
        description.len() < 500,
        "discovery description should stay compact, got {} bytes",
        description.len()
    );

    let parameters = tool.parameters_schema();
    assert_eq!(
        parameters["required"],
        json!(["category", "query", "reason"])
    );
    assert_eq!(
        parameters["properties"]["query"]["minLength"],
        DISCOVERY_QUERY_MIN_CHARS
    );
    assert_eq!(
        parameters["properties"]["reason"]["minLength"],
        DISCOVERY_REASON_MIN_CHARS
    );
    let schema = serde_json::to_string(&parameters).unwrap();
    assert!(schema.contains("Missing capability category; infer it from the user's goal."));
    assert!(schema.contains("details investigates one without selecting it"));
    assert!(schema.contains("May be shared with integration providers"));
    assert!(schema.contains("never secrets or personal data"));
    assert!(schema.contains("Why the candidate is relevant"));
    assert!(schema.contains("known_product"));
    assert!(schema.contains("capability_gap"));
    assert!(schema.contains("prior_request_id"));
    assert!(schema.contains("Off-catalog selections are recorded without provider information"));
    assert_eq!(
        parameters["properties"]["action"]["enum"],
        json!(["search", "details", "select", "suggest"])
    );
    assert_eq!(
        parameters["properties"]["category"]["enum"],
        json!(crate::sponsors::DISCOVERY_CATEGORIES)
    );
    assert!(
        parameters["properties"]["category"]["enum"]
            .as_array()
            .is_some_and(|categories| categories.contains(&json!("git")))
    );
    assert!(
        schema.len() < 6_500,
        "discovery schema should stay compact, got {} bytes",
        schema.len()
    );
}

#[test]
fn discovery_action_is_explicit_but_backwards_compatible() {
    assert_eq!(
        DiscoveryAction::parse(None, false).unwrap(),
        DiscoveryAction::Search
    );
    assert_eq!(
        DiscoveryAction::parse(None, true).unwrap(),
        DiscoveryAction::Select
    );
    assert_eq!(
        DiscoveryAction::parse(Some("select"), true).unwrap(),
        DiscoveryAction::Select
    );
    assert_eq!(
        DiscoveryAction::parse(Some("details"), true).unwrap(),
        DiscoveryAction::Details
    );
    assert_eq!(
        DiscoveryAction::parse(Some("suggest"), false).unwrap(),
        DiscoveryAction::Suggest
    );
    assert!(DiscoveryAction::parse(Some("select"), false).is_err());
    assert!(DiscoveryAction::parse(Some("details"), false).is_err());
    assert!(DiscoveryAction::parse(Some("search"), true).is_err());
    assert!(DiscoveryAction::parse(Some("suggest"), true).is_err());
}

#[test]
fn details_validation_requires_structured_relevance_and_goal() {
    let mut input: DiscoverToolsInput = serde_json::from_value(json!({
        "action": "details",
        "category": "payments",
        "query": "confirm metered subscription billing and webhook reconciliation support",
        "reason": "the current SaaS billing workflow needs usage reporting and reliable invoice state updates",
        "tool": "Stripe",
        "work_relevance": "core_requirement",
        "investigation_goal": "capability_fit",
        "requirements": ["TypeScript SDK", "Webhook status updates"],
        "topics": ["capabilities", "limitations"],
        "prior_request_id": "11111111-2222-4333-8444-555555555555"
    })).unwrap();
    let details = validate_details(&input).unwrap();
    assert_eq!(details.work_relevance, "core_requirement");
    assert_eq!(details.investigation_goal, "capability_fit");
    assert_eq!(details.topics, vec!["capabilities", "limitations"]);

    input.work_relevance = Some("interesting".to_string());
    assert!(validate_details(&input).is_err());
    input.work_relevance = Some("core_requirement".to_string());
    input.topics = Some(vec!["pricing".to_string(), "pricing".to_string()]);
    assert!(validate_details(&input).is_err());
}

#[test]
fn render_details_includes_decision_brief_and_both_source_links() {
    let rendered = render_details("payments", "stripe", &json!({
        "tool": "Stripe",
        "fit": "partial",
        "summary": "Metered billing is supported, but the requested reconciliation flow needs an additional webhook.",
        "capabilities": ["Usage meters", "Invoices"],
        "limitations": ["No automatic replay endpoint"],
        "freshness": { "status": "current", "checked_at": "2026-08-24T00:00:00Z" },
        "sources": [{
            "title": "Usage billing",
            "provider_url": "https://docs.example.com/billing",
            "cached_url": "https://jcode.sh/docs/example/billing"
        }],
        "next_action": "select"
    })).unwrap();
    assert!(rendered.contains("Fit: partial"));
    assert!(rendered.contains("https://docs.example.com/billing"));
    assert!(rendered.contains("Jcode snapshot: https://jcode.sh/docs/example/billing"));
    assert!(rendered.contains("Details do not select or connect"));
}

/// Old action names stay valid so resumed sessions and saved benchmark
/// baselines keep parsing.
#[test]
fn legacy_action_names_still_parse() {
    assert_eq!(
        DiscoveryAction::parse(Some("browse"), false).unwrap(),
        DiscoveryAction::Search
    );
    assert_eq!(
        DiscoveryAction::parse(Some("setup"), true).unwrap(),
        DiscoveryAction::Select
    );
    assert!(DiscoveryAction::parse(Some("setup"), false).is_err());
    assert!(DiscoveryAction::parse(Some("browse"), true).is_err());
}

#[test]
fn suggestion_validation_distinguishes_product_and_capability_gap() {
    let capability = DiscoverToolsInput {
        action: Some("suggest".to_string()),
        category: "payments".to_string(),
        query: Some("manage Stripe sandbox products through scoped agent access".to_string()),
        reason: Some(
            "the current payment listing only provides cards and cannot manage Stripe test data"
                .to_string(),
        ),
        tool: None,
        suggestion_kind: Some("capability_gap".to_string()),
        product_name: None,
        product_url: None,
        gap_evidence: Some(
            "Agentcard provides virtual cards rather than sandbox catalog administration."
                .to_string(),
        ),
        requirements: Some(vec![
            "Scoped authentication without secret keys".to_string(),
        ]),
        prior_request_id: Some("11111111-2222-4333-8444-555555555555".to_string()),
        work_relevance: None,
        investigation_goal: None,
        topics: None,
    };
    let validated = validate_suggestion(&capability).unwrap();
    assert_eq!(validated.kind, "capability_gap");
    assert!(validated.product_name.is_none());

    let mut known = capability;
    known.suggestion_kind = Some("known_product".to_string());
    known.product_name = Some("Example Stripe MCP".to_string());
    known.product_url = Some("https://example.com/tool?via=jcode#setup".to_string());
    let validated = validate_suggestion(&known).unwrap();
    assert_eq!(
        validated.product_name.as_deref(),
        Some("Example Stripe MCP")
    );
    assert_eq!(
        validated.product_url.as_deref(),
        Some("https://example.com/tool")
    );
}

#[test]
fn suggestion_validation_rejects_private_or_mismatched_fields() {
    let mut input = DiscoverToolsInput {
        action: Some("suggest".to_string()),
        category: "databases".to_string(),
        query: Some("managed database provisioning through scoped agent access".to_string()),
        reason: Some(
            "the current catalog does not include a database provisioning integration".to_string(),
        ),
        tool: None,
        suggestion_kind: Some("known_product".to_string()),
        product_name: Some("Private database tool".to_string()),
        product_url: Some("https://user:password@example.com/setup".to_string()),
        gap_evidence: None,
        requirements: Some(Vec::new()),
        prior_request_id: Some("11111111-2222-4333-8444-555555555555".to_string()),
        work_relevance: None,
        investigation_goal: None,
        topics: None,
    };
    assert!(validate_suggestion(&input).is_err());
    input.product_url = None;
    input.suggestion_kind = Some("capability_gap".to_string());
    assert!(validate_suggestion(&input).is_err());
    input.product_name = None;
    input.requirements = Some(vec!["api_key=abcdefghijklmnop".to_string()]);
    assert!(validate_suggestion(&input).is_err());
}

#[test]
fn optional_suggestion_fields_accept_explicit_nulls() {
    let input: DiscoverToolsInput = serde_json::from_value(json!({
        "action": "browse",
        "category": "payments",
        "query": "compare agent payment card tools for controlled automated purchasing",
        "reason": "visually verify discovery results with useful catalog details in the interface",
        "tool": null,
        "suggestion_kind": null,
        "product_name": null,
        "product_url": null,
        "gap_evidence": null,
        "requirements": null,
        "prior_request_id": null
    }))
    .unwrap();

    assert!(input.requirements.is_none());
    assert!(input.tool.is_none());
}

#[test]
fn render_suggestion_is_clear_about_review_status_and_recipient() {
    let suggestion = ValidatedSuggestion {
        kind: "known_product".to_string(),
        product_name: Some("Stripe sandbox MCP".to_string()),
        product_url: Some("https://example.com/stripe-mcp".to_string()),
        gap_evidence: Some("The listed card tool cannot manage Stripe objects.".to_string()),
        requirements: vec!["Scoped test-mode access".to_string()],
        prior_request_id: "11111111-2222-4333-8444-555555555555".to_string(),
    };
    let out = render_suggestion(
        "payments",
        "manage Stripe sandbox products and recurring prices",
        "the listed payment tool cannot administer Stripe test data",
        &suggestion,
        &json!({
            "suggestion_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
            "status": "received"
        }),
    )
    .unwrap();
    assert!(out.contains("Catalog suggestion submitted"));
    assert!(out.contains("Product: Stripe sandbox MCP"));
    assert!(out.contains("Suggestions are not sent to integration providers"));
    assert!(out.contains("does not mean the tool has integrated with Jcode"));
    assert!(!out.to_ascii_lowercase().contains("partner"));
}

#[test]
fn discovery_text_requires_substantive_content() {
    let missing = validate_discovery_text(None, "query", 20, 500).unwrap_err();
    assert_eq!(missing.failure_reason, "missing_query");
    let short = validate_discovery_text(Some("payment tool"), "query", 20, 500).unwrap_err();
    assert_eq!(short.failure_reason, "query_too_short");
    let padded = validate_discovery_text(Some("tool tool tool tool tool tool"), "query", 20, 500)
        .unwrap_err();
    assert_eq!(padded.failure_reason, "query_not_specific");
    let valid = validate_discovery_text(
        Some("  virtual card for a capped online checkout  "),
        "query",
        20,
        500,
    )
    .unwrap();
    assert_eq!(valid, "virtual card for a capped online checkout");
}

#[test]
fn discovery_text_rejects_recognizable_secrets_and_card_numbers() {
    let stripe_shaped_key = ["sk_", "live_", "abcdefghijklmnopqrstuvwxyz"].concat();
    let sensitive = [
        "Need a service using api_key=abcdefghijklmnop for the request".to_string(),
        "Forward Authorization: Bearer abcdefghijklmnopqrstuvwxyz".to_string(),
        format!("Use {stripe_shaped_key} for this payment workflow"),
        "Use card 4242 4242 4242 4242 for the partner tool checkout".to_string(),
        "Use eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnopqrstuvwxyz"
            .to_string(),
        "Credential follows -----BEGIN PRIVATE KEY----- abcdefghijklmnop".to_string(),
        "Contact private-person@example.com to configure the partner capability".to_string(),
        "Use customer identifier 123-45-6789 while selecting the external service".to_string(),
        "Fetch https://private-user:private-password@example.com/config for setup".to_string(),
        "Send the account alert to +1-202-555-0147 after the external setup completes".to_string(),
    ];
    for value in sensitive {
        let err = validate_discovery_text(Some(&value), "reason", 40, 2_000).unwrap_err();
        assert_eq!(err.failure_reason, "reason_sensitive_data", "{value}");
        assert!(!err.message.contains(&value));
    }
}

#[test]
fn discovery_text_allows_non_secret_capability_language() {
    for value in [
        "Need an API-key management service with scoped access controls",
        "Need public tourism data about Slovakia for a travel planning tool",
        "Need OAuth bearer-token support without transmitting any token value",
    ] {
        assert!(
            validate_discovery_text(Some(value), "reason", 40, 2_000).is_ok(),
            "{value}"
        );
    }
}

/// Minimal one-shot HTTP server that answers a single request with the
/// given body, returning the request line + headers it received.
async fn one_shot_server(
    status_line: &'static str,
    body: String,
) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let response = format!(
            "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.ok();
        request
    });
    (format!("http://{addr}"), handle)
}

fn test_discovery_request<'a>(
    client: &'a reqwest::Client,
    endpoint: &'a str,
    request_id: &'a str,
    benchmark_run: bool,
) -> DiscoveryRequestContext<'a> {
    DiscoveryRequestContext {
        client,
        endpoint,
        request_id,
        category: "payments",
        query: "virtual card for checkout",
        reason: "task needs an online payment capability",
        benchmark_run,
        provenance: test_provenance(),
    }
}

fn test_provenance() -> DiscoveryRequestProvenance {
    DiscoveryRequestProvenance {
        session_id: "session-test-1".to_string(),
        correlation_id: Some("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string()),
        session_metadata_available: true,
        is_self_dev: true,
        is_debug: false,
        is_canary: true,
        execution_mode: "agent_turn",
        build_channel: "selfdev".to_string(),
        is_git_checkout: true,
        is_ci: false,
        ran_from_cargo: true,
    }
}

#[tokio::test]
async fn fetch_listing_round_trips_and_sends_only_expected_params() {
    let body = json!({"tools": [{"name": "agentcard", "blurb": "virtual cards", "url": "https://a.example"}]}).to_string();
    let (endpoint, server) = one_shot_server("HTTP/1.1 200 OK", body).await;
    let client = reqwest::Client::new();
    let request = test_discovery_request(&client, &endpoint, "request-test-1", true);
    let listing = fetch_listing(&request, None).await.unwrap();
    assert_eq!(listing.listing["tools"][0]["name"], "agentcard");
    assert_eq!(listing.http_status, 200);
    assert!(listing.response_bytes > 0);

    let request = server.await.unwrap();
    let request_line = request.lines().next().unwrap();
    // Exactly the three disclosed query parameters. Provenance is carried
    // in bounded headers so it cannot be confused with model-authored text.
    assert!(request_line.contains("category=payments"), "{request_line}");
    assert!(request_line.contains("q=virtual"), "{request_line}");
    assert!(request_line.contains("reason=task"), "{request_line}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-jcode-discovery-request-id: request-test-1"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-jcode-discovery-benchmark: 1"),
        "{request}"
    );
    for expected in [
        "x-jcode-discovery-session-id: session-test-1",
        "x-jcode-session-correlation-id: aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "x-jcode-discovery-session-metadata: 1",
        "x-jcode-discovery-self-dev: 1",
        "x-jcode-discovery-debug: 0",
        "x-jcode-discovery-canary: 1",
        "x-jcode-discovery-execution-mode: agent_turn",
        "x-jcode-discovery-build-channel: selfdev",
        "x-jcode-discovery-git-checkout: 1",
        "x-jcode-discovery-ci: 0",
        "x-jcode-discovery-ran-from-cargo: 1",
    ] {
        assert!(request.to_ascii_lowercase().contains(expected), "{request}");
    }
}

#[tokio::test]
async fn fetch_details_posts_decision_context_and_returns_agent_brief() {
    let response = json!({
        "tool": "Stripe",
        "fit": "strong",
        "summary": "The requested metered billing workflow is supported.",
        "capabilities": ["Usage meters", "Webhook invoice updates"],
        "freshness": { "status": "current", "checked_at": "2026-08-24T00:00:00Z" },
        "sources": [{
            "title": "Metered billing",
            "provider_url": "https://docs.stripe.com/billing/subscriptions/usage-based",
            "cached_url": "https://jcode.sh/docs/stripe/usage-based"
        }],
        "next_action": "select"
    });
    let (endpoint, server) = one_shot_server("HTTP/1.1 200 OK", response.to_string()).await;
    let client = reqwest::Client::new();
    let context = test_discovery_request(
        &client,
        &endpoint,
        "11111111-2222-4333-8444-555555555555",
        false,
    );
    let details = ValidatedDetails {
        work_relevance: "core_requirement".to_string(),
        investigation_goal: "capability_fit".to_string(),
        requirements: vec!["Webhook status updates".to_string()],
        topics: vec!["capabilities".to_string(), "limitations".to_string()],
        prior_request_id: Some("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string()),
    };

    let fetched = fetch_details(&context, "stripe", &details).await.unwrap();
    let request = server.await.unwrap();
    assert!(request.starts_with("POST /details HTTP/1.1"));
    for expected in [
        "\"tool\":\"stripe\"",
        "\"work_relevance\":\"core_requirement\"",
        "\"investigation_goal\":\"capability_fit\"",
        "\"requirements\":[\"Webhook status updates\"]",
        "\"topics\":[\"capabilities\",\"limitations\"]",
        "\"prior_request_id\":\"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\"",
    ] {
        assert!(
            request.contains(expected),
            "missing request field: {expected}"
        );
    }
    let rendered = render_details("payments", "stripe", &fetched.listing).unwrap();
    assert!(rendered.contains("Fit: strong"));
    assert!(rendered.contains("Usage meters"));
    assert!(rendered.contains("https://docs.stripe.com/billing/subscriptions/usage-based"));
    assert!(rendered.contains("Jcode snapshot: https://jcode.sh/docs/stripe/usage-based"));
    assert!(rendered.contains("Suggested next action: `select`"));
}

#[tokio::test]
async fn fetch_listing_hard_fails_on_http_error() {
    let (endpoint, _server) =
        one_shot_server("HTTP/1.1 500 Internal Server Error", "{}".to_string()).await;
    let client = reqwest::Client::new();
    let request = test_discovery_request(&client, &endpoint, "request-test-2", false);
    let err = fetch_listing(&request, None).await.unwrap_err();
    assert!(err.to_string().contains("discovery unavailable"));
    assert_eq!(err.failure_reason, "http_error");
    assert_eq!(err.http_status, Some(500));
}

#[tokio::test]
async fn fetch_listing_hard_fails_when_endpoint_unreachable() {
    // Reserved port with no listener: connection refused, no fallback.
    let client = reqwest::Client::new();
    let request = test_discovery_request(&client, "http://127.0.0.1:9", "request-test-3", false);
    let err = fetch_listing(&request, None).await.unwrap_err();
    assert!(err.to_string().contains("discovery unavailable"));
    assert_eq!(err.failure_reason, "connect_error");
}

#[tokio::test]
async fn submit_suggestion_posts_structured_maintainer_only_payload() {
    let body = json!({
        "suggestion_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "message": "received"
    })
    .to_string();
    let (endpoint, server) = one_shot_server("HTTP/1.1 202 Accepted", body).await;
    let suggestion = ValidatedSuggestion {
        kind: "known_product".to_string(),
        product_name: Some("Stripe sandbox MCP".to_string()),
        product_url: Some("https://example.com/stripe-mcp".to_string()),
        gap_evidence: Some(
            "Agentcard provides cards rather than Stripe object administration.".to_string(),
        ),
        requirements: vec!["Scoped test-mode access".to_string()],
        prior_request_id: "11111111-2222-4333-8444-555555555555".to_string(),
    };
    let client = reqwest::Client::new();
    let request = DiscoveryRequestContext {
        client: &client,
        endpoint: &endpoint,
        request_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        category: "payments",
        query: "manage Stripe sandbox products through scoped agent access",
        reason: "the current payment listing only provides cards and cannot manage Stripe test data",
        benchmark_run: true,
        provenance: test_provenance(),
    };
    let result = submit_suggestion(&request, &suggestion).await.unwrap();
    assert_eq!(result.http_status, 202);
    // Successful receipts from older deployments omitted `status`.
    assert_eq!(result.listing["status"], "received");

    let request = server.await.unwrap();
    let lower = request.to_ascii_lowercase();
    assert!(
        request.starts_with("POST /suggestions HTTP/1.1"),
        "{request}"
    );
    assert!(
        lower.contains("x-jcode-discovery-request-id: aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"),
        "{request}"
    );
    assert!(
        lower.contains("x-jcode-discovery-benchmark: 1"),
        "{request}"
    );
    assert!(request.contains("\"suggestion_kind\":\"known_product\""));
    assert!(request.contains("\"prior_request_id\":\"11111111-2222-4333-8444-555555555555\""));
    assert!(request.contains("\"product_name\":\"Stripe sandbox MCP\""));
    assert!(request.contains("\"requirements\":[\"Scoped test-mode access\"]"));
}

#[tokio::test]
async fn submit_suggestion_treats_duplicate_receipt_as_success() {
    let body = json!({
        "suggestion_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "status": "duplicate",
        "message": "already recorded"
    })
    .to_string();
    let (endpoint, _server) = one_shot_server("HTTP/1.1 409 Conflict", body).await;
    let suggestion = ValidatedSuggestion {
        kind: "capability_gap".to_string(),
        product_name: None,
        product_url: None,
        gap_evidence: None,
        requirements: Vec::new(),
        prior_request_id: "11111111-2222-4333-8444-555555555555".to_string(),
    };
    let client = reqwest::Client::new();
    let request = DiscoveryRequestContext {
        client: &client,
        endpoint: &endpoint,
        request_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        category: "payments",
        query: "manage Stripe sandbox products through scoped agent access",
        reason: "the current payment listing only provides cards and cannot manage Stripe test data",
        benchmark_run: false,
        provenance: test_provenance(),
    };
    let result = submit_suggestion(&request, &suggestion).await.unwrap();
    assert_eq!(result.http_status, 409);
    assert_eq!(result.listing["status"], "duplicate");
}

fn test_ctx() -> crate::tool::ToolContext {
    crate::tool::ToolContext {
        session_id: "test".into(),
        message_id: "test".into(),
        tool_call_id: "test".into(),
        working_dir: None,
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    }
}

#[tokio::test]
async fn execute_records_off_catalog_selection_without_provider_information() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let temp = tempfile::tempdir().unwrap();
    crate::env::set_var("JCODE_HOME", temp.path());

    let body = json!({
        "category": "web-data",
        "selected_tool": "firecrawl",
        "listed": false,
    })
    .to_string();
    let (endpoint, server) = one_shot_server("HTTP/1.1 200 OK", body).await;
    std::fs::write(
        temp.path().join("config.toml"),
        format!("[sponsors]\nenabled = true\nendpoint = \"{endpoint}\"\n"),
    )
    .unwrap();
    crate::config::Config::invalidate_cache();

    let output = DiscoverToolsTool::new()
        .execute(
            json!({
                "action": "select",
                "category": "web-data",
                "query": "crawl a documentation site and extract structured markdown",
                "reason": "the user explicitly requested Firecrawl instead of the catalog listing",
                "tool": "Firecrawl",
            }),
            test_ctx(),
        )
        .await
        .unwrap();

    assert!(
        output
            .output
            .contains("Selected off-catalog product 'firecrawl'")
    );
    assert!(output.output.contains("no provider information"));
    assert!(!output.output.contains("Setup:"));
    let metadata = output.metadata.unwrap();
    assert_eq!(metadata["selected_tool"], "firecrawl");
    assert_eq!(metadata["catalog_tool"], false);
    assert_eq!(metadata["sponsored_discovery"], false);

    let request = server.await.unwrap();
    assert!(request.starts_with("GET /?"), "{request}");
    assert!(request.contains("tool=firecrawl"), "{request}");

    if let Some(prev) = prev_home {
        crate::env::set_var("JCODE_HOME", prev);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}

#[tokio::test]
async fn details_executes_through_public_tool_interface() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let temp = tempfile::tempdir().unwrap();
    crate::env::set_var("JCODE_HOME", temp.path());
    let body = json!({
        "tool": "Stripe",
        "fit": "strong",
        "summary": "Metered billing and webhook reconciliation are supported.",
        "capabilities": ["Usage meters", "Invoice webhooks"],
        "sources": [{
            "title": "Usage billing",
            "provider_url": "https://docs.stripe.com/billing/subscriptions/usage-based",
            "cached_url": "https://jcode.sh/docs/stripe/usage-based"
        }],
        "next_action": "select"
    })
    .to_string();
    let (endpoint, server) = one_shot_server("HTTP/1.1 200 OK", body).await;
    std::fs::write(
        temp.path().join("config.toml"),
        format!("[sponsors]\nenabled = true\nendpoint = \"{endpoint}\"\n"),
    )
    .unwrap();
    crate::config::Config::invalidate_cache();

    let output = DiscoverToolsTool::new().execute(json!({
        "action": "details",
        "category": "payments",
        "query": "confirm metered subscription billing and webhook reconciliation support",
        "reason": "the current SaaS workflow requires usage reporting and reliable invoice state updates",
        "tool": "Stripe",
        "work_relevance": "core_requirement",
        "investigation_goal": "capability_fit",
        "requirements": ["Webhook status updates"],
        "topics": ["capabilities", "limitations"]
    }), test_ctx()).await.unwrap();

    assert_eq!(output.title.as_deref(), Some("stripe details"));
    assert!(output.output.contains("Fit: strong"));
    assert!(output.output.contains("Usage meters"));
    assert!(
        output
            .output
            .contains("https://docs.stripe.com/billing/subscriptions/usage-based")
    );
    assert!(
        output
            .output
            .contains("Jcode snapshot: https://jcode.sh/docs/stripe/usage-based")
    );
    let metadata = output.metadata.unwrap();
    assert_eq!(metadata["integration_details"], true);
    assert_eq!(metadata["work_relevance"], "core_requirement");
    assert_eq!(metadata["investigation_goal"], "capability_fit");
    let request = server.await.unwrap();
    assert!(request.starts_with("POST /details HTTP/1.1"), "{request}");
    assert!(
        request.contains("\"work_relevance\":\"core_requirement\""),
        "{request}"
    );

    if let Some(prev) = prev_home {
        crate::env::set_var("JCODE_HOME", prev);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}

#[tokio::test]
async fn git_category_executes_end_to_end_with_enabled_config_and_local_server() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let temp = tempfile::tempdir().unwrap();
    crate::env::set_var("JCODE_HOME", temp.path());

    let body = json!({"tools": [{"name": "github", "blurb": "repository hosting and collaboration", "url": "https://github.com", "setup": "MCP server: npx github-mcp"}]}).to_string();
    let (endpoint, server) = one_shot_server("HTTP/1.1 200 OK", body).await;
    std::fs::write(
        temp.path().join("config.toml"),
        format!("[sponsors]\nenabled = true\nendpoint = \"{endpoint}\"\n"),
    )
    .unwrap();
    crate::config::Config::invalidate_cache();

    let tool = DiscoverToolsTool::new();
    let output = tool
        .execute(
            json!({
                "category": "git",
                "query": "host and collaborate on git repositories",
                "reason": "task requires remote repository collaboration capabilities not present in the current tools"
            }),
            test_ctx(),
        )
        .await
        .unwrap();

    assert!(output.output.contains("github"));
    assert!(output.output.contains("Jcode integration directory"));
    assert!(
        output
            .output
            .contains("recommendations must be based only on fit")
    );
    // End to end, not just in render_listing: a browse must never hand the
    // agent runnable setup, or it has no reason to call select.
    assert!(
        !output.output.contains("npx github-mcp"),
        "browse leaked setup instructions: {}",
        output.output
    );
    assert!(output.output.contains("action `select`"));
    let title = output.title.unwrap();
    assert_eq!(title, "git", "{title}");
    let meta = output.metadata.unwrap();
    assert_eq!(meta["sponsored_discovery"], true);

    let request = server.await.unwrap();
    assert!(request.contains("category=git"), "{request}");

    // Opted-out config: execute refuses without any network call.
    std::fs::write(
        temp.path().join("config.toml"),
        "[sponsors]\nenabled = false\n",
    )
    .unwrap();
    crate::config::Config::invalidate_cache();
    let err = tool
        .execute(json!({"category": "payments", "reason": "x"}), test_ctx())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("disabled"));

    if let Some(prev) = prev_home {
        crate::env::set_var("JCODE_HOME", prev);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}
