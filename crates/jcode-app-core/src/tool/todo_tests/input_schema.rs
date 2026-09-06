#[test]
fn tool_is_named_todo() {
    assert_eq!(TodoTool::new().name(), "todo");
}
#[test]
fn schema_advertises_intent_and_todos() {
    let schema = TodoTool::new().parameters_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("todo schema should have properties");
    assert_eq!(props.len(), 4);
    assert!(props.contains_key("intent"));
    assert!(props.contains_key("todos"));
    assert!(props.contains_key("plan"));
    assert!(props.contains_key("goals"));

    let item = props["todos"]
        .get("items")
        .and_then(|v| v.as_object())
        .expect("todos should describe item objects");
    let required = item
        .get("required")
        .and_then(|v| v.as_array())
        .expect("todo item should advertise required fields");
    assert!(required.iter().any(|v| v == "confidence"));
    let item_props = item
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("todo item should advertise properties");
    assert!(item_props.contains_key("confidence"));
    assert!(item_props.contains_key("completion_confidence"));
    assert!(!item_props.contains_key("closed_feedback_loop"));
    assert_eq!(
        item_props["confidence"]["description"],
        "Evidence state that this todo can be completed correctly; reassess as evidence accumulates."
    );

    let plan_props = props["plan"]
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("plan should describe properties");
    assert!(plan_props.contains_key("user_intention"));
    assert!(plan_props.contains_key("understands_user_intent"));
    assert!(!plan_props.contains_key("alignment_score"));
    assert!(!plan_props.contains_key("user_intention_alignment"));
    assert_eq!(plan_props.len(), 2);
    let plan_required = props["plan"]["required"]
        .as_array()
        .expect("plan should advertise required fields");
    assert!(plan_required.iter().any(|value| value == "user_intention"));
    assert!(
        plan_required
            .iter()
            .any(|value| value == "understands_user_intent")
    );

    let goal_props = props["goals"]
        .get("items")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("goals should describe item objects");
    assert!(goal_props.contains_key("group"));
    assert!(goal_props.contains_key("closed_feedback_loop"));
    assert!(goal_props.contains_key("feedback_loop"));
    assert!(goal_props.contains_key("feedback_loop_relevance"));
    assert!(goal_props.contains_key("feedback_loop_coverage"));
    assert!(goal_props.contains_key("feedback_loop_traceability"));
    assert!(goal_props.contains_key("delivery_state"));
    assert!(goal_props.contains_key("difficulty"));
    assert!(goal_props.contains_key("autonomy"));
    assert!(goal_props.contains_key("iteration_maturity"));
    assert!(goal_props.contains_key("stopping_evidence"));
    assert!(!goal_props.contains_key("end_to_end_ownership"));
    // Intent lives on the plan, not per goal.
    assert!(!goal_props.contains_key("user_intention"));
    assert!(!goal_props.contains_key("alignment_score"));
    assert!(!goal_props.contains_key("objective"));
    assert_eq!(goal_props.len(), 11);
    assert_eq!(
        goal_props["feedback_loop_relevance"]["enum"],
        json!([
            "indirect",
            "synthetic",
            "representative",
            "acceptance_blocked",
            "acceptance_aligned"
        ])
    );
    let relevance_description = goal_props["feedback_loop_relevance"]["description"]
        .as_str()
        .expect("feedback-loop relevance should explain every state");
    for required_concept in [
        "custom harnesses",
        "real public interfaces",
        "external constraint",
        "Substitute-only validation is never acceptance_aligned",
    ] {
        assert!(relevance_description.contains(required_concept));
    }

    let goal_required = props["goals"]["items"]["required"]
        .as_array()
        .expect("goals should advertise required fields");
    assert!(
        goal_required
            .iter()
            .any(|value| value == "closed_feedback_loop")
    );
    assert!(goal_required.iter().any(|value| value == "feedback_loop"));
    assert!(
        goal_required
            .iter()
            .any(|value| value == "feedback_loop_relevance")
    );
    assert!(
        goal_required
            .iter()
            .any(|value| value == "feedback_loop_coverage")
    );
    assert!(
        goal_required
            .iter()
            .any(|value| value == "feedback_loop_traceability")
    );

    let alignment_description = plan_props["understands_user_intent"]
        .get("description")
        .and_then(Value::as_str)
        .expect("alignment score should describe representation coverage");
    assert!(alignment_description.contains("what the user wants"));
    assert!(alignment_description.contains("when guessing"));
    // The detailed calibration rubric moved out of the always-on schema
    // into deferred turn-finish continuation messages, which are paid only
    // when the completed turn needs another quality pass.
    for required_concept in [
        "requirement inventory",
        "outcomes, deliverables, constraints, prohibited actions",
        "integration paths, edge cases, and necessary follow-through",
        "Do not ask the user",
    ] {
        assert!(
            crate::todo::TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE.contains(required_concept),
            "intent gate message omitted {required_concept}"
        );
    }
    let feedback_description = goal_props["feedback_loop"]
        .get("description")
        .and_then(Value::as_str)
        .expect("feedback loop should describe requirement-to-check coverage");
    // Case-insensitive: the description opens the sentence with
    // "Requirement-to-check", so a case-sensitive match broke when the
    // wording moved to the front of the string (issue #730).
    let feedback_description_lower = feedback_description.to_ascii_lowercase();
    assert!(
        feedback_description_lower.contains("requirement-to-check"),
        "feedback_loop description omitted the requirement-to-check framing: {feedback_description}"
    );
    assert!(
        feedback_description_lower.contains("explicit observation or check"),
        "feedback_loop description omitted per-requirement check coverage: {feedback_description}"
    );
    for required_concept in [
        "reports back on each requirement",
        "run tests, verify, or review count only",
        "non-testable requirements",
    ] {
        assert!(
            crate::todo::TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE.contains(required_concept),
            "feedback gate message omitted {required_concept}"
        );
    }
    assert!(
        !alignment_description
            .to_ascii_lowercase()
            .contains("threshold")
    );

    let ownership_description = goal_props["delivery_state"]
        .get("description")
        .and_then(Value::as_str)
        .expect("delivery state should have a neutral description");
    assert!(ownership_description.contains("toward the user's outcome"));
    assert!(!ownership_description.contains("90"));
    assert!(
        !ownership_description
            .to_ascii_lowercase()
            .contains("threshold")
    );

    let loop_description = goal_props["closed_feedback_loop"]
        .get("description")
        .and_then(Value::as_str)
        .expect("closed feedback loop should describe the assessment neutrally");
    assert!(!loop_description.to_ascii_lowercase().contains("threshold"));

    let model_visible_schema = serde_json::to_string(&schema)
        .expect("todo schema should serialize")
        .to_ascii_lowercase();
    for disclosure in [
        "threshold",
        "quality gate",
        "internal quality check",
        "not jump",
        "test that passes",
        "isn't high enough",
    ] {
        assert!(
            !model_visible_schema.contains(disclosure),
            "model-visible todo schema disclosed calibration wording: {disclosure}"
        );
    }
    for required_guidance in [
        "public interfaces",
        "integration boundaries",
        "edge cases",
        "packaging",
        "likely failure modes",
    ] {
        assert!(
            model_visible_schema.contains(required_guidance),
            "todo schema omitted generic validation guidance: {required_guidance}"
        );
    }
    for domain_hint in [
        "visual quality",
        "screenshot",
        "browser",
        "viewport",
        "console error",
    ] {
        assert!(
            !model_visible_schema.contains(domain_hint),
            "model-visible todo schema biased visual-work feedback: {domain_hint}"
        );
    }
}
#[test]
fn accepts_stringified_todos_array() {
    let input = json!({
        "todos": "[{\"content\":\"a\",\"status\":\"pending\",\"priority\":\"high\",\"id\":\"1\",\"confidence\":90}]"
    });
    let parsed = parse(input).expect("stringified todos array should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].content, "a");
    assert_eq!(
        todos[0].confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
}
#[test]
fn accepts_stringified_todo_items_and_string_confidence() {
    let input = json!({
        "todos": [
            "{\"content\":\"b\",\"status\":\"completed\",\"priority\":\"low\",\"id\":\"2\",\"confidence\":\"85\",\"completion_confidence\":\"95\"}",
            {"content": "c", "status": "pending", "priority": "high", "id": "3", "confidence": "70"}
        ]
    });
    let parsed = parse(input).expect("string-coerced items should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(todos.len(), 2);
    assert_eq!(
        todos[0].confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
    assert_eq!(
        todos[0].completion_confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
    assert_eq!(
        todos[1].confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
}
#[test]
fn normalizes_natural_and_case_varied_todo_statuses() {
    let parsed = parse(json!({
        "todos": [
            {"content": "a", "status": "done", "priority": "high", "id": "1", "confidence": "verified"},
            {"content": "b", "status": " Finished ", "priority": "low", "id": "2", "confidence": "validated"},
            {"content": "c", "status": "Canceled", "priority": "low", "id": "3", "confidence": "plausible"}
        ]
    }))
    .expect("status synonyms should parse");
    let statuses: Vec<_> = parsed
        .todos
        .expect("todos present")
        .into_iter()
        .map(|todo| todo.status)
        .collect();
    assert_eq!(statuses, ["completed", "completed", "cancelled"]);
}
#[test]
fn rejects_unknown_todo_statuses_with_valid_vocabulary() {
    let error = parse(json!({
        "todos": [
            {"content": "a", "status": "blocked", "priority": "high", "id": "1", "confidence": "plausible"}
        ]
    }))
    .err()
    .expect("unknown status should be rejected");
    let message = error.to_string();
    assert!(message.contains("invalid todo status \"blocked\""));
    assert!(message.contains("pending, in_progress, completed, cancelled"));
}
#[test]
fn accepts_float_confidence_and_empty_string_as_none() {
    let input = json!({
        "todos": [
            {"content": "d", "status": "pending", "priority": "high", "id": "4", "confidence": 90.0, "completion_confidence": ""}
        ]
    });
    let parsed = parse(input).expect("float confidence should parse");
    let todos = parsed.todos.expect("todos present");
    assert_eq!(
        todos[0].confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
    assert_eq!(todos[0].completion_confidence, None);
}
#[test]
fn empty_string_todos_means_read() {
    let parsed = parse(json!({"todos": ""})).expect("empty string should parse");
    assert!(parsed.todos.is_none());
}
#[test]
fn native_input_still_parses() {
    let input = json!({
        "todos": [
            {"content": "e", "status": "pending", "priority": "high", "id": "5", "confidence": 80}
        ]
    });
    let parsed = parse(input).expect("native input should parse");
    assert_eq!(
        parsed.todos.expect("todos present")[0].confidence,
        Some(crate::todo::ConfidenceState::Plausible)
    );
}
#[test]
fn accepts_goals_and_plan_including_string_coercion() {
    let input = json!({
        "plan": {"user_intention": "make repository search feel instant", "understands_user_intent": "97"},
        "goals": [
            {"group": "optimize grep", "closed_feedback_loop": "95", "feedback_loop": "run the grep benchmark and compare p50"},
            {"closed_feedback_loop": 20}
        ]
    });
    let parsed = parse(input).expect("goals and plan should parse");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(
        plan.understands_user_intent,
        Some(crate::todo::IntentUnderstanding::Clear)
    );
    assert_eq!(
        plan.user_intention.as_deref(),
        Some("make repository search feel instant")
    );
    let goals = parsed.goals.expect("goals present");
    assert_eq!(
        goals[0].closed_feedback_loop,
        Some(crate::todo::FeedbackLoopState::Strong)
    );
    assert_eq!(
        goals[0].feedback_loop.as_deref(),
        Some("run the grep benchmark and compare p50")
    );
    // Runtime parsing remains backward-compatible with stored or older
    // provider payloads even though the advertised schema requires the field.
    assert_eq!(goals[1].feedback_loop, None);
    assert_eq!(goals[1].group, None);
}
#[test]
fn stringified_plan_object_is_accepted() {
    let parsed = parse(json!({
        "plan": "{\"user_intention\":\"ship it\",\"understands_user_intent\":\"96\"}"
    }))
    .expect("stringified plan should parse");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(plan.user_intention.as_deref(), Some("ship it"));
    assert_eq!(
        plan.understands_user_intent,
        Some(crate::todo::IntentUnderstanding::Clear)
    );
}
#[test]
fn accepts_legacy_plan_alignment_key_but_serializes_the_new_name() {
    let parsed = parse(json!({
        "plan": {"user_intention_alignment": "97"}
    }))
    .expect("legacy alignment key should remain readable");
    let plan = parsed.plan.expect("plan present");
    assert_eq!(
        plan.understands_user_intent,
        Some(crate::todo::IntentUnderstanding::Clear)
    );

    let serialized = serde_json::to_value(plan).expect("plan should serialize");
    assert_eq!(serialized["understands_user_intent"], "clear");
    assert!(serialized.get("user_intention_alignment").is_none());

    let legacy_field: TodoPlanField = serde_json::from_str("\"user_intention_alignment\"")
        .expect("legacy plan-change field should deserialize");
    assert_eq!(legacy_field, TodoPlanField::UnderstandsUserIntent);
    assert_eq!(
        serde_json::to_string(&legacy_field).expect("plan field should serialize"),
        "\"understands_user_intent\""
    );
}
