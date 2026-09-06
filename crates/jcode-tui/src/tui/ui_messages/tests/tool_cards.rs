#[test]
fn render_system_message_uses_scheduled_task_card() {
    let msg = DisplayMessage::system(
        "[Scheduled task]\nA scheduled task for this session is now due.\n\nTask: Follow up on the scheduler test\nWorking directory: /home/jeremy/jcode\nRelevant files: src/tui/ui_messages.rs\nBranch: master\n\nBackground: Verify the scheduled task card styling\nSuccess criteria: The due task renders clearly\nScheduled by session: session_test",
    );

    let lines = render_system_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains(width_stable_system_title(
        "⏰ scheduled task due",
        "scheduled task due"
    )));
    assert!(plain.contains("This scheduled task is now active in this session."));
    assert!(plain.contains("Follow up on the scheduler test"));
    assert!(plain.contains("Verify the scheduled task card styling"));
    assert!(!plain.contains("[Scheduled task]"));
    assert!(!plain.contains("A scheduled task for this session is now due."));
}
#[test]
fn render_tool_message_uses_scheduled_card() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Scheduled task 'Follow up on the scheduler test' for in 1m (id: sched_abc123)\nWorking directory: /home/jeremy/jcode\nRelevant files: src/tui/ui_messages.rs\nTarget: resume session session_test".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("scheduled: Follow up on the scheduler test".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_schedule_card".to_string(),
            name: "schedule".to_string(),
            input: serde_json::json!({
                "task": "Follow up on the scheduler test",
                "wake_in_minutes": 1,
                "target": "resume"
            }),
            intent: None, thought_signature: None, }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains(width_stable_system_title("⏰ scheduled", "scheduled")));
    assert!(plain.contains("Will run in 1m."));
    assert!(plain.contains("Follow up on the scheduler test"));
    assert!(plain.contains("session session_test"));
    assert!(plain.contains("sched_abc123"));
    assert!(!plain.contains("✓ schedule"));
}
#[test]
fn render_tool_message_prefers_subagent_title_with_model() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "done".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("Verify subagent model (general · gpt-5.4)".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_1".to_string(),
            name: "subagent".to_string(),
            input: serde_json::json!({
                "description": "Verify subagent model",
                "subagent_type": "general"
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 80, crate::config::DiffDisplayMode::Off);
    let rendered: String = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(rendered.contains("subagent Verify subagent model (general · gpt-5.4)"));
}
#[test]
fn render_tool_message_shows_intent_and_technical_preview_on_one_line() {
    crate::tui::ui::tools_ui::tests_tool_call_details_override::set(true);
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "ok".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_intent".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargo test -p jcode render_background_task --lib",
                "intent": "Verify compact progress card"
            }),
            intent: Some("Verify compact progress card".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered = extract_line_text(&lines[0]);

    assert!(rendered.contains("bash · Verify compact progress card · $ cargo test"));
    assert_eq!(lines.len(), 1, "Bash output is hidden by default");
    crate::tui::ui::tools_ui::tests_tool_call_details_override::set(false);
}
/// Default (tool_call_details off): a row with an intent renders only the
/// intent; the dimmed technical preview is dropped and no fallback command
/// line is added.
#[test]
fn render_tool_message_hides_technical_preview_by_default() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "ok".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_intent".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargo test -p jcode render_background_task --lib",
                "intent": "Verify compact progress card"
            }),
            intent: Some("Verify compact progress card".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered = extract_line_text(&lines[0]);

    assert!(
        rendered.contains("bash · Verify compact progress card"),
        "rendered={rendered}"
    );
    assert!(
        !rendered.contains("cargo test"),
        "technical detail should be hidden by default: {rendered}"
    );
    assert_eq!(lines.len(), 1, "Bash output is hidden by default");
}
/// Even with details off, a failed tool row keeps its error summary so
/// failures stay diagnosable.
#[test]
fn render_tool_message_keeps_error_summary_when_details_hidden() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "Error: command not found: cargoo".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_intent_err".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargoo test",
                "intent": "Run the test suite"
            }),
            intent: Some("Run the test suite".to_string()),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered = extract_line_text(&lines[0]);

    assert!(
        rendered.contains("Run the test suite ·"),
        "error summary should still render after the intent: {rendered}"
    );
}
#[test]
fn render_tool_message_shows_token_badge() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "x".repeat(7_600),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_2".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({"file_path": "src/main.rs"}),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let badge_span = lines[0]
        .spans
        .iter()
        .find(|span| span.content.contains("1.9k tok"))
        .expect("missing token badge");

    assert_eq!(badge_span.style.fg, Some(rgb(118, 118, 118)));
}
#[test]
fn render_tool_message_hides_bash_output() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "<class 'zip'>\n[('p', 'b'), ('a', 'a'), ('l', 'l'), ('e', 'e')]".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_bash_output".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "python3 -c \"s='pale'; t='bale'; print(type(zip(s,t))); print(list(zip(s,t)))\""
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off);
    let rendered = lines.iter().map(extract_line_text).collect::<Vec<_>>();

    assert!(!rendered.iter().any(|line| line.contains("<class 'zip'>")));
    assert!(!rendered.iter().any(|line| line.contains("[('p', 'b')")));
}
#[test]
fn render_tool_message_shows_bash_output_when_enabled() {
    crate::tui::ui::tools_ui::tests_show_bash_output_override::set(true);
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "one\ntwo\nthree\nfour".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_bash_output_enabled".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "printf output"}),
            intent: Some("Print output".to_string()),
            thought_signature: None,
        }),
    };

    let rendered = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>();

    assert_eq!(rendered.len(), 4);
    assert!(!rendered.iter().any(|line| line.trim() == "one"));
    assert!(rendered.iter().any(|line| line.trim() == "two"));
    assert!(rendered.iter().any(|line| line.trim() == "four"));
    crate::tui::ui::tools_ui::tests_show_bash_output_override::set(false);
}
#[test]
fn render_tool_message_shows_gmail_draft_card() {
    let msg = gmail_draft_message(
        "Draft created successfully.\nDraft ID: draft_123\nTo: bob@example.com\nSubject: Project update",
        serde_json::json!({
            "action": "draft",
            "to": "bob@example.com",
            "subject": "Project update",
            "body": "Hi Bob,\n\nThe release is ready for review."
        }),
    );

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("Gmail draft created · draft_123"), "{plain}");
    assert!(
        !plain.contains('✉'),
        "draft card should not show an icon: {plain}"
    );
    assert!(plain.contains("To: bob@example.com"), "{plain}");
    assert!(plain.contains("Subject: Project update"), "{plain}");
    assert!(
        plain.contains("The release is ready for review."),
        "{plain}"
    );
    assert!(
        !plain.contains("\"body\""),
        "must not leak raw JSON: {plain}"
    );
}
#[test]
fn render_gmail_draft_card_marks_failures_and_empty_fields() {
    let msg = gmail_draft_message(
        "Error: Gmail draft creation failed",
        serde_json::json!({ "action": "draft", "body": "" }),
    );

    let lines = render_tool_message(&msg, 80, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("Gmail draft failed"), "{plain}");
    assert!(plain.contains("(recipient missing)"), "{plain}");
    assert!(plain.contains("(no subject)"), "{plain}");
    assert!(plain.contains("(empty body)"), "{plain}");
}
#[test]
fn render_gmail_draft_card_wraps_attachments_and_shows_complete_long_body() {
    let body = (1..=30)
        .map(|index| format!("body line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let msg = gmail_draft_message(
        "Draft created successfully.\nDraft ID: draft_long",
        serde_json::json!({
            "action": "draft",
            "to": "a-very-long-recipient-address@example.com",
            "subject": "A subject that should wrap cleanly in a narrow transcript",
            "body": body,
            "attachments": [
                "/tmp/a-very-long-quarterly-report-filename.pdf",
                "/tmp/notes.txt"
            ]
        }),
    );

    let lines = render_tool_message(&msg, 48, crate::config::DiffDisplayMode::Off);
    let rendered = lines.iter().map(extract_line_text).collect::<Vec<_>>();
    let plain = rendered.join("\n");
    let compact = without_whitespace(&plain.replace('│', ""));

    assert!(
        compact.contains("To:a-very-long-recipient-address@example.com"),
        "{plain}"
    );
    assert!(
        compact.contains("Subject:Asubjectthatshouldwrapcleanlyinanarrowtranscript"),
        "{plain}"
    );
    assert!(
        compact
            .contains("Attachments:/tmp/a-very-long-quarterly-report-filename.pdf,/tmp/notes.txt"),
        "{plain}"
    );
    assert!(plain.contains("body line 18"), "{plain}");
    assert!(plain.contains("body line 19"), "{plain}");
    assert!(plain.contains("body line 30"), "{plain}");
    assert!(
        !plain.contains("more lines"),
        "body must not be truncated: {plain}"
    );
    assert!(
        lines.iter().all(|line| line.width() <= 47),
        "draft card exceeded row width: {rendered:?}"
    );
}
#[test]
fn render_gmail_draft_card_preserves_html_like_body_text() {
    let msg = gmail_draft_message(
        "Draft created successfully.\nDraft ID: draft_html",
        serde_json::json!({
            "action": "draft",
            "to": "web@example.com",
            "subject": "HTML-ish content",
            "body": "<p>Hello <strong>team</strong></p>"
        }),
    );

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        plain.contains("<p>Hello <strong>team</strong></p>"),
        "{plain}"
    );
}
#[test]
fn render_batch_tool_message_shows_nested_gmail_draft_card() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] gmail ---\nDraft created successfully.\nDraft ID: nested_123\nTo: nested@example.com\nSubject: Nested\n\nCompleted: 1 succeeded, 0 failed".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_batch_gmail".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [{
                    "tool": "gmail",
                    "parameters": {
                        "action": "draft",
                        "to": "nested@example.com",
                        "subject": "Nested",
                        "body": "Created inside a batch"
                    }
                }]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        plain.contains("Gmail draft created · nested_123"),
        "{plain}"
    );
    assert!(plain.contains("Created inside a batch"), "{plain}");
}
#[test]
fn render_batch_tool_message_shows_flat_and_nested_subcall_intents() {
    crate::tui::ui::tools_ui::tests_tool_call_details_override::set(true);
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] read ---\nflat output\n\n--- [2] read ---\nnested output\n\nCompleted: 2 succeeded, 0 failed".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_batch_intents".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [
                    {
                        "tool": "read",
                        "intent": "Inspect flat batch input",
                        "file_path": "src/flat.rs"
                    },
                    {
                        "tool": "read",
                        "parameters": {
                            "intent": "Inspect nested batch input",
                            "file_path": "src/nested.rs"
                        }
                    }
                ]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 120, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        plain.contains("read · Inspect flat batch input ·"),
        "{plain}"
    );
    assert!(
        plain.contains("read · Inspect nested batch input ·"),
        "{plain}"
    );
    assert!(plain.contains("flat.rs"), "{plain}");
    assert!(plain.contains("nested.rs"), "{plain}");
    crate::tui::ui::tools_ui::tests_tool_call_details_override::set(false);
}
#[test]
fn render_tool_message_shows_discovery_browse_results_and_rationale() {
    let msg = discovery_message(
        "Discoverable tools in 'payments' (Jcode tool directory; recommendations must be based only on fit; details: https://jcode.sh/discovery-tools):\n\n- agentcard: prepaid virtual Visa cards for AI agents (https://agentcard.sh/?via=jcode-discovery)\n\nSearch request ID: `11111111-2222-4333-8444-555555555555`",
        serde_json::json!({
            "action": "search",
            "category": "payments",
            "query": "manage Stripe sandbox products and recurring prices",
            "reason": "the task needs test-mode catalog administration through scoped agent access"
        }),
    );
    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("agentcard"), "{plain}");
    assert!(
        !plain.contains("1 integration"),
        "single-result browse shows only the entry name: {plain}"
    );
    assert!(
        !plain.contains("why:"),
        "browse results stay to a single line without rationale: {plain}"
    );
    assert!(
        !plain.contains("prepaid virtual Visa cards"),
        "browse results must not render descriptions: {plain}"
    );
    assert!(
        !plain.contains("agentcard.sh"),
        "browse results must not render URLs: {plain}"
    );
    assert!(
        !plain.contains("Listings are vetted"),
        "discovery results must not render the disclosure notice: {plain}"
    );
    assert!(!plain.contains("sponsored result"), "{plain}");
    assert!(
        lines.len() <= 8,
        "compact discovery details used {} lines: {plain}",
        lines.len()
    );
    assert!(
        !plain.contains("\n\n"),
        "compact details contain a blank row: {plain}"
    );
    assert!(
        !plain
            .chars()
            .any(|ch| matches!(ch, '╭' | '╮' | '╰' | '╯' | '│')),
        "discovery details must remain borderless: {plain}"
    );
    assert!(
        !plain.contains("11111111-2222"),
        "request IDs stay model-only: {plain}"
    );
}
#[test]
fn batched_discovery_renders_without_disclosure_notice() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "--- [1] integration_tools ---\nAvailable integrations in 'payments' (Jcode tool directory; recommendations must be based only on fit; details: https://jcode.sh/discovery-tools):\n\n- agentcard: prepaid virtual Visa cards for AI agents (https://agentcard.sh/?via=jcode-discovery)\n\nSearch request ID: `11111111-2222-4333-8444-555555555555`\n\nCompleted: 1 succeeded, 0 failed".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_batch_discovery".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "tool_calls": [{
                    "tool": "integration_tools",
                    "parameters": {
                        "action": "search",
                        "category": "payments",
                        "query": "issue a capped virtual card",
                        "reason": "the task requires a payment instrument with a hard limit"
                    }
                }]
            }),
            intent: None,
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("agentcard"), "{plain}");
    assert!(
        !plain.contains("1 integration"),
        "single-result browse shows only the entry name: {plain}"
    );
    assert!(
        !plain.contains("Listings are vetted"),
        "batched discovery must not render the disclosure notice: {plain}"
    );
    assert!(
        !plain
            .chars()
            .any(|ch| matches!(ch, '╭' | '╮' | '╰' | '╯' | '│')),
        "batched discovery details must remain borderless: {plain}"
    );
}
#[test]
fn render_tool_message_shows_selected_discovery_setup() {
    let msg = discovery_message(
        "Selected 'agentcard' from 'payments' (Jcode tool directory; selection must be based only on fit; details: https://jcode.sh/discovery-tools):\n\nagentcard: prepaid virtual Visa cards for AI agents (https://agentcard.sh/?via=jcode-discovery)\n\nSetup: Run `npx -y agentcard-mcp@1.2.3`, then connect the resulting MCP server.\n\nConsequential actions (signups, spending) must note the partnership in the confirmation shown to the user.",
        serde_json::json!({
            "action": "select",
            "category": "payments",
            "tool": "agentcard",
            "query": "create a capped virtual card for an online purchase",
            "reason": "selected because capped cards fit the purchase constraints better than alternatives"
        }),
    );
    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("selected agentcard"), "{plain}");
    assert!(!plain.contains("sponsored"), "{plain}");
    assert!(
        plain.contains("details: prepaid virtual Visa cards"),
        "{plain}"
    );
    assert!(plain.contains("https://agentcard.sh"), "{plain}");
    assert!(plain.contains("setup:"), "{plain}");
    assert!(plain.contains("agentcard-mcp@1.2.3"), "{plain}");
    assert!(
        !plain.contains("Listings are vetted"),
        "discovery results must not render the disclosure notice: {plain}"
    );
}
#[test]
fn render_tool_message_does_not_duplicate_selected_when_tool_is_missing() {
    let msg = discovery_message(
        "Selection recorded.",
        serde_json::json!({
            "action": "select",
            "category": "web-search",
            "query": "find current public estimates"
        }),
    );
    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("selected tool"), "{plain}");
    assert!(!plain.contains("selected selected tool"), "{plain}");
}
#[test]
fn render_tool_message_marks_off_catalog_selection_without_fake_details() {
    let msg = discovery_message(
        "Selected off-catalog product 'firecrawl' for 'web-data'.\n\nSelection recorded as demand data. Jcode does not list or partner with this product, so no provider information, recommendation, or setup instructions are provided.",
        serde_json::json!({
            "action": "select",
            "category": "web-data",
            "tool": "firecrawl",
            "query": "crawl a documentation site and extract structured markdown",
            "reason": "the user explicitly requested Firecrawl instead of the catalog listing"
        }),
    );
    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("selected off-catalog firecrawl"), "{plain}");
    assert!(
        plain.contains("why: the user explicitly requested"),
        "{plain}"
    );
    assert!(!plain.contains("details:"), "{plain}");
    assert!(!plain.contains("setup:"), "{plain}");
}
#[test]
fn render_tool_message_shows_catalog_suggestion_receipt_and_trust_line() {
    let msg = discovery_message(
        "Catalog suggestion submitted.\n\nSuggestion ID: aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\nCategory: payments\nKind: known_product\nCapability: manage Stripe sandbox products\nCatalog gap: no matching catalog entry\nProduct: Stripe sandbox MCP\nPublic URL: https://example.com/stripe-mcp\n\nStatus: received for Jcode maintainer review. Suggestions are not sent to partners. This does not mean Jcode has partnered with the tool or that it is approved or available.",
        serde_json::json!({
            "action": "suggest",
            "category": "payments",
            "query": "manage Stripe sandbox products and recurring prices through scoped agent access",
            "reason": "the listed payment tool only provides cards and cannot administer Stripe test data",
            "suggestion_kind": "known_product",
            "product_name": "Stripe sandbox MCP",
            "product_url": "https://example.com/stripe-mcp",
            "gap_evidence": "Agentcard handles cards rather than Stripe test-mode objects.",
            "requirements": [
                "Scoped authentication without exposing a secret key",
                "Create recurring prices in test mode"
            ],
            "prior_request_id": "11111111-2222-4333-8444-555555555555"
        }),
    );
    let lines = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("suggestion sent"), "{plain}");
    assert!(
        plain.contains("Known product · Stripe sandbox MCP"),
        "{plain}"
    );
    assert!(plain.contains("gap: the listed payment tool"), "{plain}");
    assert!(plain.contains("needs:"), "{plain}");
    assert!(plain.contains("Jcode maintainers only"), "{plain}");
    assert!(plain.contains("not approval or availability"), "{plain}");
    assert!(
        !plain.contains("11111111-2222"),
        "prior request ID must stay hidden: {plain}"
    );
}
#[test]
fn discovery_cards_wrap_within_narrow_transcript_width() {
    let msg = discovery_message(
        "Catalog suggestion submitted.\n\nStatus: received for Jcode maintainer review.",
        serde_json::json!({
            "action": "suggest",
            "category": "cloud-infrastructure",
            "query": "a deliberately long capability description that must wrap cleanly in a narrow terminal",
            "reason": "the current catalog entries do not satisfy several detailed infrastructure constraints",
            "suggestion_kind": "capability_gap",
            "requirements": ["A long requirement that also needs reliable narrow-width wrapping"]
        }),
    );
    let lines = render_tool_message(&msg, 48, crate::config::DiffDisplayMode::Off);
    assert!(
        lines.iter().all(|line| line.width() <= 47),
        "discovery card exceeded width: {:?}",
        lines.iter().map(extract_line_text).collect::<Vec<_>>()
    );
}
