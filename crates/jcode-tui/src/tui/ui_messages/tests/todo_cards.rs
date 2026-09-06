#[test]
fn render_todos_message_shows_goal_scores_without_verbose_feedback() {
    let todos = vec![crate::todo::TodoItem {
        id: "1".to_string(),
        content: "Render the card".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("todo rendering".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(85)),
        completion_confidence: None,
        confidence_history: vec![
            crate::todo::ConfidenceState::from_legacy_score(80),
            crate::todo::ConfidenceState::from_legacy_score(85),
        ],
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("todo rendering".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(95)),
        feedback_loop: Some("Inspect a debug frame".to_string()),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
        delivery_state: Some(crate::todo::DeliveryState::from_legacy_score(90)),
        ..Default::default()
    }];
    let plan = crate::todo::TodoPlan {
        user_intention: Some("Keep the agent aligned with the user's request".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(98)),
        ..Default::default()
    };
    let msg = DisplayMessage::todos(
        serde_json::json!({ "todos": todos, "plan": plan, "goals": goals }).to_string(),
    );

    let plain = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    for assessment in ["Closed feedback loop strong", "Delivery workflow_validated"] {
        assert!(plain.contains(assessment), "{plain}");
    }
    assert!(!plain.contains("Relevance representative"), "{plain}");
    assert!(!plain.contains("Coverage main_paths"), "{plain}");
    // Only the plan-level assessment renders above the groups.
    assert!(
        plain.contains("Intent clear: Keep the agent aligned"),
        "{plain}"
    );
    assert!(!plain.contains("Feedback ·"), "{plain}");
    assert!(!plain.contains("Inspect a debug frame"), "{plain}");
    assert!(plain.contains("● Render the card · plausible"), "{plain}");
    assert!(!plain.contains("(high)"), "{plain}");
}
#[test]
fn render_todos_message_shows_user_intention_when_understanding_is_unclear() {
    let long_text = "This deliberately long assessment detail should not consume several rows in a narrow terminal window";
    let todos = vec![crate::todo::TodoItem {
        id: "1".to_string(),
        content: "Keep the task visible".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("responsive card".to_string()),
        confidence: None,
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let plan = crate::todo::TodoPlan {
        user_intention: Some(long_text.to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        ..Default::default()
    };
    let goals = vec![crate::todo::TodoGoal {
        group: Some("responsive card".to_string()),
        feedback_loop: Some(long_text.to_string()),
        ..Default::default()
    }];
    let msg = DisplayMessage::todos(
        serde_json::json!({ "todos": todos, "plan": plan, "goals": goals }).to_string(),
    );

    let narrow = render_todos_message(&msg, 60, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>();
    assert_eq!(
        narrow
            .iter()
            .filter(|line| line.contains("Intent partial:"))
            .count(),
        1
    );
    assert_eq!(
        narrow
            .iter()
            .filter(|line| line.contains("Feedback"))
            .count(),
        0,
        "verbose goal feedback should stay out of inline cards: {}",
        narrow.join("\n")
    );
    assert!(
        narrow
            .iter()
            .any(|line| line.contains("Keep the task visible")),
        "{}",
        narrow.join("\n")
    );

    let wide = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>();
    assert!(
        wide.iter()
            .any(|line| line.contains("Intent partial: This deliberately long assessment detail")),
        "wide={wide:?}"
    );
    assert!(
        wide.iter().any(|line| line.contains('…')),
        "wide intent should remain on one ellipsized line: {wide:?}"
    );
    assert!(
        !narrow
            .iter()
            .any(|line| line.contains("narrow terminal window")),
        "narrow={narrow:?}"
    );
    assert!(
        narrow.iter().any(|line| line.contains('…')),
        "narrow intent should be ellipsized: {narrow:?}"
    );
}
#[test]
fn render_todos_message_uses_readable_semantic_colors() {
    let todos = vec![crate::todo::TodoItem {
        id: "1".to_string(),
        content: "Tune the palette".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("todo rendering".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(85)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("todo rendering".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(95)),
        feedback_loop: None,
        ..Default::default()
    }];
    let plan = crate::todo::TodoPlan {
        user_intention: Some("Readable metadata".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(98)),
        ..Default::default()
    };
    let msg = DisplayMessage::todos(
        serde_json::json!({ "todos": todos, "plan": plan, "goals": goals }).to_string(),
    );
    let lines = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let color_for = |text: &str| {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == text)
            .and_then(|span| span.style.fg)
    };

    assert_eq!(color_for("todo rendering"), Some(todo_group_color()));
    assert_eq!(color_for("clear"), Some(todo_score_color()));
    assert_eq!(color_for("Readable metadata"), Some(todo_meta_color()));
    assert_eq!(color_for("● "), Some(asap_color()));
    assert_eq!(color_for(" (high)"), None);
    assert_eq!(color_for(" · plausible"), Some(todo_confidence_color()));
    assert_eq!(color_for("strong"), Some(todo_warning_color()));
    assert_eq!(color_for("missing"), Some(todo_failure_color()));
    assert_ne!(todo_meta_color(), dim_color());
}
#[test]
fn render_todos_message_color_codes_every_intent_state() {
    let cases = [
        (
            crate::todo::IntentUnderstanding::Uncertain,
            todo_failure_color(),
        ),
        (
            crate::todo::IntentUnderstanding::Partial,
            todo_warning_color(),
        ),
        (crate::todo::IntentUnderstanding::Clear, todo_score_color()),
        (
            crate::todo::IntentUnderstanding::Complete,
            todo_score_color(),
        ),
    ];

    for (state, expected_color) in cases {
        let state_text = state.as_str().to_string();
        let msg = DisplayMessage::todos(
            serde_json::json!({
                "todos": [],
                "plan": {
                    "user_intention": "Keep intent visible",
                    "understands_user_intent": state,
                },
                "goals": [],
            })
            .to_string(),
        );
        let lines = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off);
        let rendered_color = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == state_text)
            .and_then(|span| span.style.fg);

        assert_eq!(
            rendered_color,
            Some(expected_color),
            "intent state {state_text} should keep its semantic color in the todo renderer"
        );
    }
}
#[test]
fn render_todos_message_collapses_passing_quality_gates() {
    let todos = vec![crate::todo::TodoItem {
        id: "1".to_string(),
        content: "Verify the result".to_string(),
        status: "completed".to_string(),
        priority: "high".to_string(),
        group: Some("quality".to_string()),
        confidence: None,
        completion_confidence: Some(crate::todo::ConfidenceState::Validated),
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("quality".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Closed),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::AcceptanceAligned),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::EdgeAndIntegrationPaths),
        feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
        delivery_state: Some(crate::todo::DeliveryState::OutcomeDelivered),
        ..Default::default()
    }];
    let msg =
        DisplayMessage::todos(serde_json::json!({ "todos": todos, "goals": goals }).to_string());
    let lines = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("✓ All quality gates passing"), "{plain}");
    assert!(plain.contains("Delivery outcome_delivered"), "{plain}");
    assert!(!plain.contains("Closed feedback loop closed"), "{plain}");
    assert!(!plain.contains("Relevance acceptance_aligned"), "{plain}");
    assert!(
        !plain.contains("Coverage edge_and_integration_paths"),
        "{plain}"
    );
    let passing = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.as_ref() == "✓ All quality gates passing")
        .and_then(|span| span.style.fg);
    assert_eq!(passing, Some(todo_score_color()));
}
#[test]
fn render_todos_message_wraps_goal_scores_at_narrow_widths() {
    let todos = vec![crate::todo::TodoItem {
        id: "1".to_string(),
        content: "Render the card".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("todo rendering".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(85)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("todo rendering".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(95)),
        feedback_loop: None,
        delivery_state: Some(crate::todo::DeliveryState::from_legacy_score(90)),
        ..Default::default()
    }];
    let msg =
        DisplayMessage::todos(serde_json::json!({ "todos": todos, "goals": goals }).to_string());

    let lines = render_todos_message(&msg, 40, crate::config::DiffDisplayMode::Off);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("Closed feedback loop strong"), "{plain}");
    assert!(plain.contains("Delivery workflow_validated"), "{plain}");
    assert!(
        lines.iter().all(|line| line.width() <= 38),
        "card exceeded its 38-column content budget: {plain}"
    );
}
#[test]
fn render_todos_message_empty_list_shows_placeholder() {
    let msg = DisplayMessage::todos("[]");
    let plain = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!plain.contains("Todos"), "{plain}");
    assert!(plain.contains("No tasks yet"), "{plain}");
}
#[test]
fn render_todos_message_bad_payload_falls_back_to_system() {
    let msg = DisplayMessage::todos("not json");
    let lines = render_todos_message(&msg, 100, crate::config::DiffDisplayMode::Off);
    assert!(!lines.is_empty());
}
#[test]
fn render_todo_tool_result_uses_borderless_card_with_goal_scores() {
    let todos = vec![crate::todo::TodoItem {
        id: "render".to_string(),
        content: "Render the todo result".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("todo rendering".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(92)),
        completion_confidence: None,
        confidence_history: vec![
            crate::todo::ConfidenceState::from_legacy_score(85),
            crate::todo::ConfidenceState::from_legacy_score(92),
        ],
        blocked_by: Vec::new(),
        assigned_to: None,
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("todo rendering".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(95)),
        feedback_loop: Some("Inspect the rendered frame".to_string()),
        delivery_state: Some(crate::todo::DeliveryState::from_legacy_score(92)),
        ..Default::default()
    }];
    let content = format!(
        "[todo] [tool timing: start=2026-07-13T19:51:50.261Z finish=2026-07-13T19:51:50.265Z duration=4ms] {}\n\nGoals:\n{}\n\n{}",
        serde_json::to_string_pretty(&todos).unwrap(),
        serde_json::to_string_pretty(&goals).unwrap(),
        crate::todo::TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE
    );
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content,
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("1 todos".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_todo".to_string(),
            name: "todo".to_string(),
            input: serde_json::json!({ "todos": todos, "goals": goals }),
            intent: Some("Track todo card work".to_string()),
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!plain.contains("Todos"), "{plain}");
    assert!(plain.contains("todo rendering  ●"), "{plain}");
    assert!(plain.contains("Closed feedback loop strong"), "{plain}");
    assert!(plain.contains("Relevance missing"), "{plain}");
    assert!(plain.contains("Coverage missing"), "{plain}");
    assert!(plain.contains("Delivery workflow_validated"), "{plain}");
    assert!(
        plain.contains("● Render the todo result · plausible"),
        "{plain}"
    );
    assert!(!plain.contains("(high)"), "{plain}");
    assert!(
        !plain.contains('╭'),
        "todo tool result should be borderless:\n{plain}"
    );
    assert!(
        !plain.contains("todo 1 items"),
        "generic tool row leaked:\n{plain}"
    );
}
#[test]
fn render_todo_quality_gate_retry_shows_only_changed_goal_fields() {
    let todos = vec![crate::todo::TodoItem {
        id: "render".to_string(),
        content: "Render the entire unchanged todo plan".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("todo rendering".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(92)),
        ..Default::default()
    }];
    let before = crate::todo::TodoGoal {
        group: Some("todo rendering".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(90)),
        feedback_loop: Some("Inspect one frame".to_string()),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Indirect),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::Narrow),
        ..Default::default()
    };
    let after = crate::todo::TodoGoal {
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(98)),
        feedback_loop: Some(
            "Render before and after fixtures and assert unchanged fields are absent".to_string(),
        ),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
        ..before.clone()
    };
    let updates = vec![crate::todo::TodoGoalChange {
        before: Some(before),
        after: Some(after.clone()),
        fields: vec![
            crate::todo::TodoGoalField::ClosedFeedbackLoop,
            crate::todo::TodoGoalField::FeedbackLoop,
            crate::todo::TodoGoalField::FeedbackLoopRelevance,
            crate::todo::TodoGoalField::FeedbackLoopCoverage,
        ],
    }];
    let content = format!(
        "{}\n\nGoals:\n{}\n\nGoal updates:\n{}\n\n{}",
        serde_json::to_string_pretty(&todos).unwrap(),
        serde_json::to_string_pretty(&vec![after]).unwrap(),
        serde_json::to_string_pretty(&updates).unwrap(),
        crate::todo::TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE,
    );
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content,
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("1 todos".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_todo_update".to_string(),
            name: "todo".to_string(),
            input: serde_json::Value::Null,
            intent: Some("Refine the todo feedback loop".to_string()),
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("todo rendering  updated"), "{plain}");
    assert!(
        plain.contains("Closed feedback loop strong → closed"),
        "{plain}"
    );
    assert!(
        plain.contains("Feedback-loop relevance indirect → representative"),
        "{plain}"
    );
    assert!(
        plain.contains("Feedback-loop coverage narrow → main_paths"),
        "{plain}"
    );
    assert!(!plain.contains("Feedback ·"), "{plain}");
    assert!(
        !plain.contains("assert unchanged fields are absent"),
        "{plain}"
    );
    assert!(
        !plain.contains("Render the entire unchanged todo plan"),
        "{plain}"
    );
    assert!(!plain.contains("Alignment score"), "{plain}");
    assert!(!plain.contains("Keep the todo card concise"), "{plain}");
    assert!(!plain.contains("See current work at a glance"), "{plain}");
}
#[test]
fn render_goal_update_size_is_bounded_when_narrative_evidence_is_long() {
    let long_text = "verbose evidence ".repeat(600);
    let goal = crate::todo::TodoGoal {
        group: Some("compact assessment".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Strong),
        delivery_state: Some(crate::todo::DeliveryState::Integrated),
        autonomy: Some(crate::todo::Autonomy::NecessaryFollowthrough),
        iteration_maturity: Some(crate::todo::IterationMaturity::OutcomeReached),
        feedback_loop: Some(long_text.clone()),
        stopping_evidence: Some(long_text),
        ..Default::default()
    };
    let update = crate::todo::TodoGoalChange {
        before: None,
        after: Some(goal),
        fields: vec![
            crate::todo::TodoGoalField::ClosedFeedbackLoop,
            crate::todo::TodoGoalField::DeliveryState,
            crate::todo::TodoGoalField::Autonomy,
            crate::todo::TodoGoalField::IterationMaturity,
            crate::todo::TodoGoalField::FeedbackLoop,
            crate::todo::TodoGoalField::StoppingEvidence,
        ],
    };

    let lines = render_todo_goal_updates(&[update], 95);
    let plain = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(lines.len(), 5, "narrative text must not add rows:\n{plain}");
    assert!(!plain.contains("Feedback"), "{plain}");
    assert!(!plain.contains("Stopping evidence"), "{plain}");
    assert!(!plain.contains("verbose evidence"), "{plain}");
}
#[test]
fn render_todo_plan_update_card_shows_only_changed_intent_fields() {
    let todos = vec![crate::todo::TodoItem {
        id: "render".to_string(),
        content: "Render the entire unchanged todo plan".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(92)),
        ..Default::default()
    }];
    let before = crate::todo::TodoPlan {
        user_intention: Some("Ship the plan-level intent gate".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(80)),
        ..Default::default()
    };
    let after = crate::todo::TodoPlan {
        understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(97)),
        ..before.clone()
    };
    let update = crate::todo::TodoPlanChange {
        before: Some(before),
        after: Some(after.clone()),
        fields: vec![crate::todo::TodoPlanField::UnderstandsUserIntent],
    };
    let content = format!(
        "{}\n\nPlan:\n{}\n\nPlan updates:\n{}",
        serde_json::to_string_pretty(&todos).unwrap(),
        serde_json::to_string_pretty(&after).unwrap(),
        serde_json::to_string_pretty(&update).unwrap(),
    );
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content,
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("1 todos".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_plan_update".to_string(),
            name: "todo".to_string(),
            input: serde_json::Value::Null,
            intent: Some("Reassess the user's intent".to_string()),
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("Plan  updated"), "{plain}");
    assert!(
        plain.contains("Understands user intent partial → clear"),
        "{plain}"
    );
    // Unchanged fields and the full plan stay out of the refinement card.
    assert!(!plain.contains("User intention"), "{plain}");
    assert!(
        !plain.contains("Render the entire unchanged todo plan"),
        "{plain}"
    );
}
#[test]
fn parse_todo_tool_output_accepts_timestamp_only_header() {
    let todos = vec![crate::todo::TodoItem {
        id: "timed".to_string(),
        content: "Render the restored todo".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        ..Default::default()
    }];
    let content = format!(
        "[2026-07-13T19:51:50.261Z] [todo] {}",
        serde_json::to_string(&todos).unwrap()
    );

    let parsed = parse_todo_tool_output(&content).expect("timestamped todo payload");
    assert_eq!(parsed.todos.len(), 1);
    assert_eq!(parsed.todos[0].id, todos[0].id);
    assert_eq!(parsed.todos[0].content, todos[0].content);
    assert!(parsed.goals.is_empty());
    assert!(parsed.goal_updates.is_empty());
}
#[test]
fn unbiased_visual_prompt_retry_renders_complete_feedback_change() {
    const PROMPT: &str = "can you make a pelican riding a bike animation in html and vanillia js ";
    const INITIAL_FEEDBACK: &str = "Open the page in a browser, inspect runtime errors, and verify animation state changes over time.";
    const REVISED_FEEDBACK: &str = "Serve the files locally, load them in a real browser at desktop and mobile viewport sizes, assert zero console/page errors, sample wheel and scenery transforms at two timestamps to prove motion, and exercise pause plus speed controls to confirm state changes.";
    const REVISED_OBJECTIVE: &str = "Deliver a responsive standalone animation whose pelican visibly pedals a moving bicycle through a layered seaside scene at 60fps where supported, with working pause/resume and three-speed controls, accessible labels, no external runtime dependencies, and zero browser console errors.";

    // Keep the eval input neutral. The visual verification strategy must come
    // from the model's todo refinement, not from criteria planted in the prompt.
    for biased_term in [
        "feedback loop",
        "browser",
        "console",
        "viewport",
        "screenshot",
        "visual quality",
    ] {
        assert!(!PROMPT.to_ascii_lowercase().contains(biased_term));
    }

    let todos = vec![crate::todo::TodoItem {
        id: "implement".to_string(),
        content: "Implement the illustrated pelican bicycle scene and responsive styling"
            .to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("pelican-bike-animation".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(90)),
        ..Default::default()
    }];
    let render = |goal: crate::todo::TodoGoal,
                  intention: &str,
                  continuation: Option<&str>,
                  tool_data: Option<crate::message::ToolCall>| {
        let plan = crate::todo::TodoPlan {
            user_intention: Some(intention.to_string()),
            understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(96)),
            ..Default::default()
        };
        let mut content = format!(
            "[todo] [tool timing: start=2026-07-13T19:51:50.261Z finish=2026-07-13T19:51:50.265Z duration=4ms] {}\n\nPlan:\n{}\n\nGoals:\n{}",
            serde_json::to_string_pretty(&todos).unwrap(),
            serde_json::to_string_pretty(&plan).unwrap(),
            serde_json::to_string_pretty(&vec![goal]).unwrap()
        );
        if let Some(continuation) = continuation {
            content.push_str("\n\n");
            content.push_str(continuation);
        }
        let msg = DisplayMessage {
            role: "tool".to_string(),
            content,
            tool_calls: Vec::new(),
            duration_secs: None,
            title: Some("1 todos".to_string()),
            tool_data,
        };
        render_tool_message(&msg, 72, crate::config::DiffDisplayMode::Off)
            .iter()
            .map(extract_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    };

    let initial = render(
        crate::todo::TodoGoal {
            group: Some("pelican-bike-animation".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(90)),
            feedback_loop: Some(INITIAL_FEEDBACK.to_string()),
            ..Default::default()
        },
        "Make a pelican riding a bike animation that clearly works in a browser",
        Some(crate::todo::TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE),
        Some(crate::message::ToolCall {
            id: "call_initial_todo".to_string(),
            name: "todo".to_string(),
            input: serde_json::Value::Null,
            intent: Some("Track implementation and browser verification".to_string()),
            thought_signature: None,
        }),
    );
    assert!(initial.contains("pelican-bike-animation"), "{initial}");
    assert!(initial.contains("Closed feedback loop strong"), "{initial}");
    assert!(!without_whitespace(&initial).contains(&without_whitespace(INITIAL_FEEDBACK)));

    // Simulate a restored/mirrored result whose ToolCall association was lost.
    // The structured result must still render as the same complete todo card.
    let revised = render(
        crate::todo::TodoGoal {
            group: Some("pelican-bike-animation".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(98)),
            feedback_loop: Some(REVISED_FEEDBACK.to_string()),
            ..Default::default()
        },
        REVISED_OBJECTIVE,
        None,
        None,
    );
    let compact_revised = without_whitespace(&revised);
    assert!(revised.contains("pelican-bike-animation"), "{revised}");
    assert!(
        revised.contains("Relevance missing · Coverage missing · Traceability missing"),
        "{revised}"
    );
    assert!(!revised.contains("Closed feedback loop closed"));
    assert!(!compact_revised.contains(&without_whitespace(REVISED_FEEDBACK)));
    assert!(revised.contains("● Implement"), "{revised}");
}
#[test]
fn visually_appealing_prompt_batched_retry_renders_compact_todo_card() {
    // This fixture is only the first todo retry emitted after the
    // closed feedback loop continuation. The eval stops here and deliberately does
    // not depend on the model implementing or completing the visual task.
    const PROMPT: &str =
        "make the most visually appealing pelican on a bike animation with html and vanillia js";
    const FEEDBACK: &str = "At each iteration, render at 1440x900 and 390x844, capture screenshots, and score five checks: scene fills viewport without clipping, focal subject is centered, at least six distinct motion layers run smoothly, controls respond, and no console errors occur. Refine until all checks pass.";
    const OBJECTIVE: &str = "Deliver a single-page vanilla HTML/CSS/JS animation whose pelican cyclist remains legible and visually balanced at desktop and mobile sizes, includes six or more coordinated motion layers, supports interactive speed controls, and runs with zero console errors.";

    assert!(!PROMPT.contains("1440x900"));
    assert!(!PROMPT.contains("screenshot"));
    assert!(!PROMPT.contains("console"));
    assert!(!PROMPT.contains("feedback"));

    let todos = vec![crate::todo::TodoItem {
        id: "inspect".to_string(),
        content: "Inspect the starter project and determine the page structure".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("pelican-bike".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(95)),
        ..Default::default()
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("pelican-bike".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(98)),
        feedback_loop: Some(FEEDBACK.to_string()),
        feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
        ..Default::default()
    }];
    let plan = crate::todo::TodoPlan {
        user_intention: Some(OBJECTIVE.to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::from_legacy_score(97)),
        ..Default::default()
    };
    let todo_output = format!(
        "{}\n\nPlan:\n{}\n\nGoals:\n{}",
        serde_json::to_string_pretty(&todos).unwrap(),
        serde_json::to_string_pretty(&plan).unwrap(),
        serde_json::to_string_pretty(&goals).unwrap()
    );
    let content = format!(
        "--- [1] todo ---\n{todo_output}\n\n--- [2] ls ---\n./\n\n0 files, 0 directories\n\nCompleted: 2 succeeded, 0 failed"
    );
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content,
        tool_calls: Vec::new(),
        duration_secs: None,
        title: None,
        tool_data: Some(crate::message::ToolCall {
            id: "call_batch".to_string(),
            name: "batch".to_string(),
            input: serde_json::json!({
                "intent": "Inspect starter files and strengthen measurable visual goals",
                "tool_calls": [
                    {
                        "tool": "todo",
                        "intent": "Make the visual outcome objectively verifiable",
                        "todos": todos,
                        "goals": goals
                    },
                    { "tool": "ls", "path": "." }
                ]
            }),
            intent: Some(
                "Inspect starter files and strengthen measurable visual goals".to_string(),
            ),
            thought_signature: None,
        }),
    };

    let lines = render_tool_message(&msg, 84, crate::config::DiffDisplayMode::Off);
    assert!(
        lines.iter().all(|line| line.width() <= 84),
        "batched todo card must respect the available width: {lines:?}"
    );
    let rendered = lines
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    let compact = without_whitespace(&rendered);

    assert!(rendered.contains("✓ todo"), "{rendered}");
    assert!(rendered.contains("pelican-bike"), "{rendered}");
    let intent_lines: Vec<_> = rendered
        .lines()
        .filter(|line| line.contains("Intent clear:"))
        .collect();
    assert_eq!(intent_lines.len(), 1, "{rendered}");
    assert!(
        intent_lines[0].contains("Deliver a single-page vanilla HTML/CSS/JS animation")
            && intent_lines[0].ends_with('…'),
        "batched todo intent should use the same single-line summary as standalone cards:\n{rendered}"
    );
    let parsed_todo = tools_ui::parse_batch_sub_outputs_by_index(&msg.content)
        .get(&1)
        .and_then(|result| parse_todo_tool_output(&result.content))
        .expect("batched todo payload should remain available in full");
    assert_eq!(parsed_todo.plan.user_intention.as_deref(), Some(OBJECTIVE));
    // Compact transcript cards show the goal's quality assessments rather than
    // repeating its potentially long feedback-loop prose. The full prose remains
    // available in the serialized todo payload and the todos side panel.
    assert!(rendered.contains("Relevance missing · Coverage missing"));
    assert!(!compact.contains(&without_whitespace(FEEDBACK)));
    let goal_details = rendered
        .split_once("pelican-bike")
        .map(|(_, details)| details)
        .and_then(|details| details.split("● Inspect").next())
        .expect("todo item should follow the batched goal details");
    assert!(
        !goal_details.contains('…'),
        "batched todo goal details must not truncate:\n{rendered}"
    );
}
#[test]
fn render_ownership_gated_todo_result_keeps_the_full_card() {
    let todos = vec![crate::todo::TodoItem {
        id: "ship".to_string(),
        content: "Deliver the complete workflow".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: Some("ship outcome".to_string()),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(95)),
        ..Default::default()
    }];
    let goals = vec![crate::todo::TodoGoal {
        group: Some("ship outcome".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(100)),
        feedback_loop: Some("Run the complete workflow".to_string()),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
        feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
        delivery_state: Some(crate::todo::DeliveryState::from_legacy_score(80)),
        ..Default::default()
    }];
    let content = format!(
        "{}\n\nGoals:\n{}\n\n{}",
        serde_json::to_string_pretty(&todos).unwrap(),
        serde_json::to_string_pretty(&goals).unwrap(),
        crate::todo::TODO_OWNERSHIP_CONTINUATION_MESSAGE
    );
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content,
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("1 todos".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_todo_ownership".to_string(),
            name: "todo".to_string(),
            input: serde_json::json!({ "todos": todos, "goals": goals }),
            intent: Some("Complete the full user outcome".to_string()),
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("ship outcome  ●"), "{plain}");
    assert!(plain.contains("Deliver the complete workflow"), "{plain}");
    assert!(
        plain.contains("✓ All quality gates passing · Delivery workflow_validated"),
        "{plain}"
    );
    assert!(!plain.contains("todo 1 items"), "{plain}");
}
#[test]
fn render_empty_todo_tool_result_collapses_to_compact_line() {
    let msg = DisplayMessage {
        role: "tool".to_string(),
        content: "[todo] []".to_string(),
        tool_calls: Vec::new(),
        duration_secs: None,
        title: Some("0 todos".to_string()),
        tool_data: Some(crate::message::ToolCall {
            id: "call_todo_empty".to_string(),
            name: "todo".to_string(),
            input: serde_json::json!({}),
            intent: Some("Read the todo list".to_string()),
            thought_signature: None,
        }),
    };

    let plain = render_tool_message(&msg, 100, crate::config::DiffDisplayMode::Off)
        .iter()
        .map(extract_line_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!plain.contains("No tasks yet"), "{plain}");
    assert!(plain.contains("no tasks"), "{plain}");
}
