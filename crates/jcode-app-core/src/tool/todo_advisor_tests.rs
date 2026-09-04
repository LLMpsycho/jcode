use super::*;

#[test]
fn advisor_acceptance_criteria_retain_clear_and_report_changes() {
    let stored = TodoPlan {
        acceptance_criteria: Some(vec!["No write before approval".into()]),
        ..TodoPlan::default()
    };
    let retained = merge_plan(&stored, Some(TodoPlan::default()));
    assert_eq!(retained.acceptance_criteria, stored.acceptance_criteria);
    let cleared = merge_plan(
        &stored,
        Some(TodoPlan {
            acceptance_criteria: Some(vec![]),
            ..TodoPlan::default()
        }),
    );
    assert_eq!(cleared.acceptance_criteria, Some(vec![]));
    assert_eq!(
        plan_change(&stored, &cleared).expect("change").fields,
        vec![TodoPlanField::AcceptanceCriteria]
    );
    let parsed = parse_todo_input(
        serde_json::json!({"plan":{"acceptance_criteria":["Restart preserves disable"]}}),
    )
    .expect("parse");
    assert_eq!(
        parsed.plan.expect("plan").acceptance_criteria,
        Some(vec!["Restart preserves disable".to_string()])
    );
}

#[test]
fn advisor_acceptance_criteria_are_bounded_and_legacy_plans_still_load() {
    assert!(
        parse_todo_input(serde_json::json!({"plan":{"acceptance_criteria":vec!["x";33]}})).is_err()
    );
    assert!(
        parse_todo_input(serde_json::json!({"plan":{"acceptance_criteria":["x".repeat(1025)]}}))
            .is_err()
    );
    let legacy: TodoPlan =
        serde_json::from_str(r#"{"user_intention":"ship"}"#).expect("legacy plan");
    assert_eq!(legacy.acceptance_criteria, None);
}
