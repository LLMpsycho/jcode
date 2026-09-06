use super::*;

fn parse(input: Value) -> Result<TodoInput> {
    parse_todo_input(input)
}

fn goal(group: Option<&str>, state: crate::todo::FeedbackLoopState) -> TodoGoal {
    TodoGoal {
        group: group.map(str::to_string),
        closed_feedback_loop: Some(state),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
        feedback_loop_traceability: Some(crate::todo::FeedbackLoopTraceability::Complete),
        ..Default::default()
    }
}

/// A plan whose intent assessment clears the private gate, so goal-level
/// tests observe only closed feedback loop behavior.
fn aligned_plan() -> TodoPlan {
    TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("understood".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Complete),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Complete],
    }
}

fn todo_in_group(group: Option<&str>, id: &str) -> TodoItem {
    TodoItem {
        content: format!("task {id}"),
        status: "pending".to_string(),
        priority: "medium".to_string(),
        id: id.to_string(),
        group: group.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn todo_telemetry_derives_lifecycle_groups_and_score_summaries() {
    let mut pending = todo_in_group(Some("build"), "pending");
    pending.confidence = Some(crate::todo::ConfidenceState::Plausible);
    let mut removed = todo_in_group(Some("build"), "removed");
    removed.status = "in_progress".to_string();
    removed.confidence = Some(crate::todo::ConfidenceState::Plausible);
    let previous = vec![pending.clone(), removed];

    pending.status = "completed".to_string();
    pending.completion_confidence = Some(crate::todo::ConfidenceState::Validated);
    let mut created = todo_in_group(Some("verify"), "created");
    created.confidence = Some(crate::todo::ConfidenceState::Plausible);
    let current = vec![pending, created];
    let goals = vec![
        TodoGoal {
            group: Some("build".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Strong),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
            feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
            delivery_state: Some(crate::todo::DeliveryState::OutcomeDelivered),
            ..Default::default()
        },
        TodoGoal {
            group: Some("verify".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Strong),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::AcceptanceAligned),
            feedback_loop_coverage: Some(
                crate::todo::FeedbackLoopCoverage::EdgeAndIntegrationPaths,
            ),
            delivery_state: Some(crate::todo::DeliveryState::OutcomeDelivered),
            ..Default::default()
        },
    ];
    let plan = TodoPlan {
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        ..Default::default()
    };

    let update = todo_telemetry_update(&previous, &current, &goals, &plan);
    assert_eq!(update.todos_created, 1);
    assert_eq!(update.todos_completed, 1);
    assert_eq!(update.todos_abandoned, 1);
    assert_eq!(update.current_incomplete, 1);
    assert_eq!(update.list_size, 2);
    assert_eq!(update.groups_completed, 1);
    assert_eq!(update.groups_total, 2);
    assert_eq!(update.confidence.min, Some(80));
    assert_eq!(update.confidence.mean, Some(80.0));
    assert_eq!(update.confidence.count, 2);
    assert_eq!(update.completion_confidence.min, Some(96));
    assert_eq!(update.completion_confidence.count, 1);
    assert_eq!(update.understands_user_intent.min, Some(80));
    assert_eq!(update.closed_feedback_loop.min, Some(88));
    assert_eq!(update.closed_feedback_loop.mean, Some(88.0));
    assert_eq!(update.feedback_loop_relevance.min, Some(75));
    assert_eq!(update.feedback_loop_relevance.count, 2);
    assert_eq!(update.feedback_loop_coverage.min, Some(75));
    assert_eq!(update.feedback_loop_coverage.count, 2);
    assert_eq!(update.end_to_end_ownership.min, Some(98));
    assert_eq!(update.end_to_end_ownership.mean, Some(98.0));
}

#[test]
fn todo_telemetry_regrouping_does_not_create_or_abandon_items() {
    let mut completed = todo_in_group(Some("old"), "a");
    completed.status = "completed".to_string();
    let pending = todo_in_group(Some("old"), "b");
    let previous = vec![completed.clone(), pending.clone()];

    completed.group = Some("done".to_string());
    let mut pending = pending;
    pending.group = Some("remaining".to_string());
    let current = vec![completed, pending];

    let update = todo_telemetry_update(&previous, &current, &[], &TodoPlan::default());
    assert_eq!(update.todos_created, 0);
    assert_eq!(update.todos_completed, 0);
    assert_eq!(update.todos_abandoned, 0);
    assert_eq!(update.groups_completed, 1);
    assert_eq!(update.groups_total, 2);
}

#[test]
fn todo_telemetry_zero_state_is_all_zero_and_has_no_scores() {
    let update = todo_telemetry_update(&[], &[], &[], &TodoPlan::default());
    assert_eq!(update, crate::telemetry::TodoTelemetryUpdate::default());
}

/// Issue #695: after the agent moves to a new task and replaces the todo
/// list, goals from the finished task must not keep showing in the panel.
#[test]
fn prune_orphaned_goals_drops_goals_without_live_todos() {
    let goals = vec![
        goal(Some("old task"), crate::todo::FeedbackLoopState::Weak),
        goal(Some("new task"), crate::todo::FeedbackLoopState::Strong),
    ];
    let todos = vec![todo_in_group(Some("new task"), "1")];

    let pruned = prune_orphaned_goals(goals, &todos);

    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0].group.as_deref(), Some("new task"));
}

#[test]
fn prune_orphaned_goals_keeps_ungrouped_goal_for_flat_list() {
    let goals = vec![goal(None, crate::todo::FeedbackLoopState::Usable)];
    let todos = vec![todo_in_group(None, "1")];

    assert_eq!(prune_orphaned_goals(goals, &todos).len(), 1);
}

#[test]
fn prune_orphaned_goals_keeps_everything_when_todo_list_is_empty() {
    // A goals-only write with no stored todos must not lose assessments.
    let goals = vec![
        goal(Some("a"), crate::todo::FeedbackLoopState::Absent),
        goal(None, crate::todo::FeedbackLoopState::Weak),
    ];
    assert_eq!(prune_orphaned_goals(goals, &[]).len(), 2);
}

#[test]
fn merge_goals_retains_unmentioned_goals() {
    let stored = vec![
        goal(Some("a"), crate::todo::FeedbackLoopState::Weak),
        goal(Some("b"), crate::todo::FeedbackLoopState::Strong),
    ];
    // Rewrite goal 'a', leave 'b' alone.
    let merged = merge_goals(
        &stored,
        Some(vec![goal(
            Some(" a "),
            crate::todo::FeedbackLoopState::Weak,
        )]),
    );
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].group.as_deref(), Some("a"));
    assert_eq!(
        merged[0].closed_feedback_loop,
        Some(crate::todo::FeedbackLoopState::Weak)
    );
    assert_eq!(merged[1].group.as_deref(), Some("b"));
    // No incoming goals: stored goals unchanged.
    assert_eq!(merge_goals(&stored, None).len(), 2);
}

#[test]
fn merge_plan_retains_stored_intent_when_update_omits_fields() {
    let stored = TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("make search feel instant".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Partial],
    };

    let merged = merge_plan(
        &stored,
        Some(TodoPlan {
            user_intention: None,
            understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
            ..Default::default()
        }),
    );
    assert_eq!(
        merged.user_intention.as_deref(),
        Some("make search feel instant")
    );
    assert_eq!(
        merged.understands_user_intent,
        Some(crate::todo::IntentUnderstanding::Partial)
    );

    // An omitted plan leaves the stored assessment untouched.
    assert_eq!(merge_plan(&stored, None), stored);
}

#[test]
fn plan_change_reports_only_updated_intent_fields() {
    let before = aligned_plan();
    let after = TodoPlan {
        user_intention: Some("understood better".to_string()),
        ..before.clone()
    };

    let change = plan_change(&before, &after).expect("intent change should be reported");
    assert_eq!(change.fields, vec![TodoPlanField::UserIntention]);
    assert_eq!(change.before.as_ref(), Some(&before));
    assert_eq!(change.after.as_ref(), Some(&after));
    assert!(plan_change(&before, &before).is_none());
}

fn open_todo(group: Option<&str>) -> TodoItem {
    TodoItem {
        id: "t1".to_string(),
        content: "work".to_string(),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        group: group.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn ownership_gate_output_preserves_the_saved_todo_card() {
    let todos = vec![open_todo(Some("ship"))];
    let plan = aligned_plan();
    let goals = vec![goal(Some("ship"), crate::todo::FeedbackLoopState::Closed)];
    let output = build_todo_output(
        todos.clone(),
        plan.clone(),
        goals.clone(),
        None,
        None,
        [crate::todo::TODO_OWNERSHIP_CONTINUATION_MESSAGE.to_string()],
    )
    .expect("ownership gate should produce a structured todo result");

    assert_eq!(output.title.as_deref(), Some("1 todos"));
    assert!(output.output.starts_with('['));
    assert!(output.output.contains("\"status\": \"in_progress\""));
    assert!(
        output
            .output
            .contains(crate::todo::TODO_OWNERSHIP_CONTINUATION_MESSAGE)
    );
    assert_eq!(
        output.metadata,
        Some(json!({"todos": todos, "plan": plan, "goals": goals}))
    );
}

fn test_ctx(session_id: &str) -> ToolContext {
    ToolContext {
        session_id: session_id.to_string(),
        message_id: session_id.to_string(),
        tool_call_id: "call".to_string(),
        working_dir: None,
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    }
}

/// Issue #695, the visibly-stale case. The todos panel renders the
/// ungrouped goal unconditionally (not only as a group header), so an
/// ungrouped goal left over from a previous flat todo list is exactly what
/// the reporter saw frozen in the panel.
#[tokio::test]
async fn an_ungrouped_goal_does_not_survive_into_a_grouped_next_task() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "issue-695-ungrouped";
    let tool = TodoTool::new();

    // Task one: a flat (ungrouped) list, so its goal is the ungrouped one.
    tool.execute(
        json!({
            "todos": [{
                "content": "flat task one", "status": "in_progress",
                "priority": "high", "id": "t1", "confidence": 70,
            }],
            "plan": {"user_intention": "do task one", "understands_user_intent": 97},
            "goals": [{"closed_feedback_loop": 97, "feedback_loop": "ran the checks"}],
        }),
        test_ctx(session),
    )
    .await
    .expect("first write");
    let stored = load_goals(session).expect("goals");
    assert_eq!(stored.len(), 1);
    assert!(
        stored[0].group.is_none(),
        "task one goal is the ungrouped one"
    );

    // Task two: a grouped list. The ungrouped goal now describes nothing.
    tool.execute(
        json!({
            "todos": [{
                "content": "task two", "status": "in_progress", "priority": "high",
                "id": "t2", "group": "second task", "confidence": 70,
            }],
            "goals": [{"group": "second task", "closed_feedback_loop": 80,
                       "feedback_loop": "run the new checks"}],
        }),
        test_ctx(session),
    )
    .await
    .expect("second write");

    let goals = load_goals(session).expect("goals");
    assert!(
        !goals.iter().any(|goal| goal.group.is_none()),
        "the stale ungrouped goal must not stay in the panel: {goals:?}"
    );
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].group.as_deref(), Some("second task"));

    if let Some(home) = previous_home {
        crate::env::set_var("JCODE_HOME", home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
}

/// Issue #695, end to end through the real tool: finish task one, then
/// start task two. What the todos panel renders (stored todos + goals) must
/// describe task two only, with no leftovers from task one.
#[tokio::test]
async fn moving_to_a_new_task_replaces_what_the_todos_panel_shows() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "issue-695-new-task";
    let tool = TodoTool::new();

    // Task one, completed. `end_to_end_ownership` clears the completion
    // gate so the write is actually stored.
    tool.execute(
        json!({
            "todos": [{
                "content": "task one",
                "status": "completed",
                "priority": "high",
                "id": "t1",
                "group": "first task",
                "confidence": 90,
                "completion_confidence": 97,
            }],
            "plan": {"user_intention": "do task one", "understands_user_intent": 97},
            "goals": [{
                "group": "first task",
                "closed_feedback_loop": 97,
                "end_to_end_ownership": 97,
                "feedback_loop": "ran the checks",
            }],
        }),
        test_ctx(session),
    )
    .await
    .expect("first task write should succeed");
    assert_eq!(load_goals(session).expect("goals").len(), 1);

    // Task two: a fresh todo list in a new group.
    tool.execute(
        json!({
            "todos": [{
                "content": "task two",
                "status": "in_progress",
                "priority": "high",
                "id": "t2",
                "group": "second task",
                "confidence": 70,
            }],
            "goals": [{
                "group": "second task",
                "closed_feedback_loop": 80,
                "feedback_loop": "run the new checks",
            }],
        }),
        test_ctx(session),
    )
    .await
    .expect("second task write should succeed");

    let todos = load_todos(session).expect("todos");
    assert_eq!(todos.len(), 1, "panel must show only the current task");
    assert_eq!(todos[0].group.as_deref(), Some("second task"));

    let goals = load_goals(session).expect("goals");
    assert_eq!(
        goals.len(),
        1,
        "the finished task's goal must not linger in the panel: {goals:?}"
    );
    assert_eq!(goals[0].group.as_deref(), Some("second task"));

    if let Some(home) = previous_home {
        crate::env::set_var("JCODE_HOME", home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
}

/// End-to-end through the real tool, which is what the model actually sees.
/// A first plan write with honestly-moderate scores must come back clean:
/// this is the exact case that previously returned two nudges and spent the
/// turn re-justifying the plan instead of doing the work.
#[tokio::test]
async fn a_moderate_first_write_returns_no_continuation_and_records_instead() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "gate-deferral-execute";

    let output = TodoTool::new()
        .execute(
            json!({
                "todos": [{
                    "content": "make utf16 transcode faster",
                    "status": "in_progress",
                    "priority": "high",
                    "id": "opt",
                    "group": "speed",
                    "confidence": 70,
                }],
                "plan": {
                    "user_intention": "beat the baseline",
                    "understands_user_intent": 82,
                },
                "goals": [{
                    "group": "speed",
                    "closed_feedback_loop": 80,
                    "feedback_loop": "run ./grade and read the score",
                    "feedback_loop_relevance": "indirect",
                    "feedback_loop_coverage": "narrow",
                }],
            }),
            test_ctx(session),
        )
        .await
        .expect("todo write should succeed");

    assert!(
        !output
            .output
            .contains(TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE),
        "a moderate first write must not be interrupted: {}",
        output.output
    );
    assert!(
        !output
            .output
            .to_ascii_lowercase()
            .contains("not high enough"),
        "no gate text should reach the model mid-turn: {}",
        output.output
    );

    // The points were recorded for the turn-end digest instead.
    let observations = crate::todo::load_gate_observations(session).expect("observations");
    assert_eq!(observations.len(), 5);
    assert!(
        observations
            .iter()
            .any(|observation| { observation.kind == GateObservationKind::FeedbackLoopRelevance })
    );
    assert!(
        observations
            .iter()
            .any(|observation| { observation.kind == GateObservationKind::FeedbackLoopCoverage })
    );
    assert!(
        observations.iter().any(|observation| {
            observation.kind == GateObservationKind::FeedbackLoopTraceability
        })
    );

    // Histories are accumulating, which is what the digest reasons over.
    let plan = load_plan(session).expect("plan");
    assert_eq!(
        plan.understands_user_intent_history,
        vec![crate::todo::IntentUnderstanding::Partial]
    );
    let goals = load_goals(session).expect("goals");
    assert_eq!(
        goals[0].closed_feedback_loop_history,
        vec![crate::todo::FeedbackLoopState::Strong]
    );
    assert_eq!(
        goals[0].feedback_loop_relevance_history,
        vec![crate::todo::FeedbackLoopRelevance::Indirect]
    );
    assert_eq!(
        goals[0].feedback_loop_coverage_history,
        vec![crate::todo::FeedbackLoopCoverage::Narrow]
    );

    // Second write at a higher score: still silent, history grows, and the
    // digest now has the trajectory available.
    let output = TodoTool::new()
        .execute(
            json!({"plan": {"understands_user_intent": 97}}),
            test_ctx(session),
        )
        .await
        .expect("second write should succeed");
    assert!(
        !output
            .output
            .to_ascii_lowercase()
            .contains("not high enough")
    );
    let plan = load_plan(session).expect("plan");
    assert_eq!(
        plan.understands_user_intent_history,
        vec![
            crate::todo::IntentUnderstanding::Partial,
            crate::todo::IntentUnderstanding::Clear
        ]
    );

    // The climb does not erase the point. The turn began without solid
    // understanding, so the work done before it settled still needs a
    // re-check; the wording just reflects that it settled late.
    let observations = crate::todo::load_gate_observations(session).expect("observations");
    let goals = load_goals(session).expect("goals");
    let digest = crate::todo::build_gate_digest(&observations, &plan, &goals)
        .expect("both recorded points should be surfaced");
    assert!(digest.contains("started this work without understanding"));
    assert!(digest.contains("feedback loop"));

    match previous_home {
        Some(value) => crate::env::set_var("JCODE_HOME", value),
        None => crate::env::remove_var("JCODE_HOME"),
    }
}

#[tokio::test]
async fn low_ownership_completion_is_saved_without_mid_write_rejection() {
    let _guard = crate::storage::lock_test_env();
    let previous_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    let session = "ownership-save-before-turn-gate";

    let output = TodoTool::new()
        .execute(
            json!({
                "todos": [{
                    "content": "ship the complete workflow",
                    "status": "completed",
                    "priority": "high",
                    "id": "ship",
                    "group": "release",
                    "confidence": 100,
                    "completion_confidence": 100,
                }],
                "goals": [{
                    "group": "release",
                    "closed_feedback_loop": 100,
                    "feedback_loop": "run the end-to-end release check",
                    "feedback_loop_relevance": "indirect",
                    "feedback_loop_coverage": "narrow",
                    "end_to_end_ownership": 95,
                }],
            }),
            test_ctx(session),
        )
        .await
        .expect("low ownership must not reject the todo write");

    let saved = load_todos(session).expect("completed todo should be persisted");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].status, "completed");
    let saved_goals = load_goals(session).expect("goal should be persisted");
    let saved_goal = &saved_goals[0];
    assert_eq!(
        saved_goal.delivery_state,
        Some(crate::todo::DeliveryState::WorkflowValidated)
    );
    assert_eq!(
        saved_goal.feedback_loop_relevance,
        Some(crate::todo::FeedbackLoopRelevance::Indirect)
    );
    assert_eq!(
        saved_goal.feedback_loop_coverage,
        Some(crate::todo::FeedbackLoopCoverage::Narrow)
    );
    assert!(
        !output
            .output
            .contains(crate::todo::TODO_OWNERSHIP_CONTINUATION_MESSAGE),
        "ownership is enforced after the turn, not by rejecting the write: {}",
        output.output
    );

    match previous_home {
        Some(value) => crate::env::set_var("JCODE_HOME", value),
        None => crate::env::remove_var("JCODE_HOME"),
    }
}

#[test]
fn goal_changes_include_only_updated_quality_fields() {
    let before = TodoGoal {
        group: Some("search".to_string()),
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Strong),
        feedback_loop: Some("Run one benchmark".to_string()),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Indirect),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::Narrow),
        delivery_state: None,
        ..Default::default()
    };
    let after = TodoGoal {
        closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Closed),
        feedback_loop: Some("Run five benchmarks and compare p50".to_string()),
        feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
        feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
        ..before.clone()
    };

    let changes = goal_changes(&[before.clone()], &[after.clone()]);

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].before.as_ref(), Some(&before));
    assert_eq!(changes[0].after.as_ref(), Some(&after));
    assert_eq!(
        changes[0].fields,
        vec![
            TodoGoalField::ClosedFeedbackLoop,
            TodoGoalField::FeedbackLoop,
            TodoGoalField::FeedbackLoopRelevance,
            TodoGoalField::FeedbackLoopCoverage,
        ]
    );
}

/// The core behavior change: a low score records an observation for the
/// turn-end digest instead of interrupting the write, and repeated writes
/// do not re-interrupt.
#[test]
fn low_open_goal_records_an_observation_without_interrupting() {
    let todos = vec![open_todo(Some("design"))];
    let plan = aligned_plan();
    let goals = vec![
        goal(Some("design"), crate::todo::FeedbackLoopState::Strong),
        goal(Some("perf"), crate::todo::FeedbackLoopState::Closed),
    ];
    let (observations, nudges) = record_reframe_observations(&plan, &goals, &todos, &[]);

    assert!(
        nudges.is_empty(),
        "a low closed feedback loop score must not interrupt the write"
    );
    assert_eq!(
        observations,
        vec![GateObservation {
            kind: GateObservationKind::ClosedFeedbackLoop,
            group: Some("design".to_string()),
            state: Some("strong".to_string()),
        }]
    );
    // A subsequent write still records, still does not interrupt.
    let (again, nudges) = record_reframe_observations(&plan, &goals, &todos, &[]);
    assert_eq!(again, observations);
    assert!(nudges.is_empty());
}

#[test]
fn low_intent_is_plan_level_and_independent_of_goals() {
    let todos = vec![open_todo(Some("coverage"))];
    let plan = TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("partially understood".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Partial],
    };
    let (observations, nudges) = record_reframe_observations(
        &plan,
        &[goal(
            Some("coverage"),
            crate::todo::FeedbackLoopState::Closed,
        )],
        &todos,
        &[],
    );

    assert_eq!(
        observations,
        vec![GateObservation {
            kind: GateObservationKind::IntentUnderstanding,
            group: None,
            state: Some("partial".to_string()),
        }]
    );
    // 95 is below threshold but nowhere near severe, so exploration is
    // given the chance to resolve it rather than being interrupted.
    assert!(nudges.is_empty());
}

/// The single retained immediate nudge: the agent's first plan write says it
/// does not understand the task at all, and a whole turn of wrong work
/// cannot be undone at turn end.
#[test]
fn severely_low_first_intent_still_nudges_immediately() {
    let todos = vec![open_todo(None)];
    let plan = TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("guessing".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Uncertain),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Uncertain],
    };
    let (_, nudges) = record_reframe_observations(&plan, &[], &todos, &[]);
    assert_eq!(nudges, vec![TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE]);
    assert!(!nudges[0].contains("40"));
    assert!(!nudges[0].to_ascii_lowercase().contains("threshold"));

    // Once the plan has a history, the same severe score is deferred to the
    // digest rather than nudged again on every write.
    let later = TodoPlan {
        understands_user_intent_history: vec![
            crate::todo::IntentUnderstanding::Uncertain,
            crate::todo::IntentUnderstanding::Uncertain,
        ],
        ..plan
    };
    let (_, nudges) = record_reframe_observations(&later, &[], &todos, &[]);
    assert!(nudges.is_empty());
}

/// Work that was already complete before this write is grandfathered: the
/// turn cannot go back and improve a loop over work it did not do.
#[test]
fn work_already_closed_before_this_write_records_nothing() {
    let mut done = open_todo(None);
    done.status = "completed".to_string();
    let already = vec![done.clone()];
    let (observations, nudges) = record_reframe_observations(
        &TodoPlan::default(),
        &[goal(None, crate::todo::FeedbackLoopState::Absent)],
        &already,
        &already,
    );
    assert!(observations.is_empty());
    assert!(nudges.is_empty());
}

/// A group created and finished in one write must still be observed. This is
/// where a weak feedback loop hides best: declare it done in one step and no
/// "still open" check ever sees it.
#[test]
fn a_group_closed_by_this_write_is_still_observed() {
    let mut done = open_todo(Some("one shot"));
    done.status = "completed".to_string();
    let (observations, nudges) = record_reframe_observations(
        &aligned_plan(),
        &[goal(Some("one shot"), crate::todo::FeedbackLoopState::Weak)],
        &[done],
        &[],
    );
    assert!(nudges.is_empty());
    assert_eq!(
        observations,
        vec![GateObservation {
            kind: GateObservationKind::ClosedFeedbackLoop,
            group: Some("one shot".to_string()),
            state: Some("weak".to_string()),
        }]
    );
}

#[test]
fn both_weak_links_are_recorded_independently() {
    let todos = vec![open_todo(Some("coverage"))];
    let plan = TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("partially understood".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Partial],
    };
    let (observations, _) = record_reframe_observations(
        &plan,
        &[goal(
            Some("coverage"),
            crate::todo::FeedbackLoopState::Strong,
        )],
        &todos,
        &[],
    );
    assert_eq!(
        observations
            .iter()
            .map(|observation| observation.kind)
            .collect::<Vec<_>>(),
        vec![
            GateObservationKind::IntentUnderstanding,
            GateObservationKind::ClosedFeedbackLoop,
        ]
    );
}

#[test]
fn missing_quality_scores_still_record_observations() {
    let todos = vec![open_todo(Some("coverage"))];
    let mut goal = goal(Some("coverage"), crate::todo::FeedbackLoopState::Closed);
    goal.closed_feedback_loop = None;

    let (observations, _) = record_reframe_observations(&TodoPlan::default(), &[goal], &todos, &[]);
    assert_eq!(
        observations
            .iter()
            .map(|observation| observation.kind)
            .collect::<Vec<_>>(),
        vec![
            GateObservationKind::IntentUnderstanding,
            GateObservationKind::ClosedFeedbackLoop,
        ]
    );
}

/// Groups already complete before this write are grandfathered, so a
/// long-lived session does not re-flag work from previous turns.
#[test]
fn observations_skip_goals_closed_in_an_earlier_write() {
    let mut done = open_todo(Some("legacy"));
    done.status = "completed".to_string();
    let already = vec![done];
    let goals = vec![goal(Some("legacy"), crate::todo::FeedbackLoopState::Absent)];
    let (observations, _) =
        record_reframe_observations(&aligned_plan(), &goals, &already, &already);
    assert!(observations.is_empty());
}

#[test]
fn observations_cover_the_ungrouped_implicit_goal() {
    let todos = vec![open_todo(None)];
    let goals = vec![goal(None, crate::todo::FeedbackLoopState::Absent)];
    let (observations, _) = record_reframe_observations(&aligned_plan(), &goals, &todos, &[]);
    assert_eq!(
        observations,
        vec![GateObservation {
            kind: GateObservationKind::ClosedFeedbackLoop,
            group: None,
            state: Some("absent".to_string()),
        }]
    );
}

/// Tool-owned histories are the substrate the turn-end digest reasons over,
/// so a model-supplied trail must not be able to fabricate a climb.
#[test]
fn plan_and_goal_score_histories_are_tool_maintained() {
    let stored = TodoPlan {
        acceptance_criteria: None,
        user_intention: Some("ship it".to_string()),
        understands_user_intent: Some(crate::todo::IntentUnderstanding::Partial),
        understands_user_intent_history: vec![crate::todo::IntentUnderstanding::Partial],
    };
    let merged = merge_plan(
        &stored,
        Some(TodoPlan {
            understands_user_intent: Some(crate::todo::IntentUnderstanding::Clear),
            // Forged trail: discarded in favor of the stored one.
            understands_user_intent_history: vec![
                crate::todo::IntentUnderstanding::Uncertain,
                crate::todo::IntentUnderstanding::Uncertain,
                crate::todo::IntentUnderstanding::Uncertain,
            ],
            ..Default::default()
        }),
    );
    assert_eq!(
        merged.understands_user_intent_history,
        vec![
            crate::todo::IntentUnderstanding::Partial,
            crate::todo::IntentUnderstanding::Clear
        ]
    );
    assert_eq!(merged.user_intention.as_deref(), Some("ship it"));

    // Re-sending the same state does not manufacture an extra step.
    let merged = merge_plan(
        &merged,
        Some(TodoPlan {
            understands_user_intent: Some(crate::todo::IntentUnderstanding::Clear),
            ..Default::default()
        }),
    );
    assert_eq!(
        merged.understands_user_intent_history,
        vec![
            crate::todo::IntentUnderstanding::Partial,
            crate::todo::IntentUnderstanding::Clear
        ]
    );

    let stored_goals = merge_goals(
        &[],
        Some(vec![TodoGoal {
            group: Some("perf".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Usable),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Indirect),
            feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::Narrow),
            ..Default::default()
        }]),
    );
    assert_eq!(
        stored_goals[0].closed_feedback_loop_history,
        vec![crate::todo::FeedbackLoopState::Usable]
    );
    let merged_goals = merge_goals(
        &stored_goals,
        Some(vec![TodoGoal {
            group: Some("perf".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Strong),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::AcceptanceAligned),
            feedback_loop_relevance_history: vec![
                crate::todo::FeedbackLoopRelevance::AcceptanceAligned,
            ],
            feedback_loop_coverage: Some(
                crate::todo::FeedbackLoopCoverage::EdgeAndIntegrationPaths,
            ),
            feedback_loop_coverage_history: vec![
                crate::todo::FeedbackLoopCoverage::EdgeAndIntegrationPaths,
            ],
            ..Default::default()
        }]),
    );
    assert_eq!(
        merged_goals[0].closed_feedback_loop_history,
        vec![
            crate::todo::FeedbackLoopState::Usable,
            crate::todo::FeedbackLoopState::Strong
        ]
    );
    assert_eq!(
        merged_goals[0].feedback_loop_relevance_history,
        vec![
            crate::todo::FeedbackLoopRelevance::Indirect,
            crate::todo::FeedbackLoopRelevance::AcceptanceAligned,
        ]
    );
    assert_eq!(
        merged_goals[0].feedback_loop_coverage_history,
        vec![
            crate::todo::FeedbackLoopCoverage::Narrow,
            crate::todo::FeedbackLoopCoverage::EdgeAndIntegrationPaths,
        ]
    );
}

/// A write that revises one assessment must not erase the others, or the
/// digest would read a stale `None` and re-raise a resolved point.
#[test]
fn omitted_goal_fields_inherit_the_stored_assessment() {
    let stored = merge_goals(
        &[],
        Some(vec![TodoGoal {
            group: Some("perf".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::Closed),
            feedback_loop: Some("cargo bench".to_string()),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
            feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
            delivery_state: Some(crate::todo::DeliveryState::OutcomeDelivered),
            ..Default::default()
        }]),
    );
    let merged = merge_goals(
        &stored,
        Some(vec![TodoGoal {
            group: Some("perf".to_string()),
            ..Default::default()
        }]),
    );
    assert_eq!(
        merged[0].closed_feedback_loop,
        Some(crate::todo::FeedbackLoopState::Closed)
    );
    assert_eq!(merged[0].feedback_loop.as_deref(), Some("cargo bench"));
    assert_eq!(
        merged[0].feedback_loop_relevance,
        Some(crate::todo::FeedbackLoopRelevance::Representative)
    );
    assert_eq!(
        merged[0].feedback_loop_coverage,
        Some(crate::todo::FeedbackLoopCoverage::MainPaths)
    );
    assert_eq!(
        merged[0].delivery_state,
        Some(crate::todo::DeliveryState::OutcomeDelivered)
    );
}

#[test]
fn garbage_string_still_errors() {
    assert!(parse(json!({"todos": "not json at all"})).is_err());
}

/// Sessions and model calls written before the rename carry
/// `hill_climbability`. Those must keep loading, or resuming an old session
/// silently drops its goal assessments and re-raises resolved gate points.
#[test]
fn pre_rename_hill_climbability_keys_still_load() {
    let goal: crate::todo::TodoGoal = serde_json::from_value(json!({
        "group": "optimize grep",
        "hill_climbability": 91,
        "hill_climbability_history": [70, 91],
        "feedback_loop": "cargo bench grep"
    }))
    .expect("the pre-rename key must still deserialize");
    assert_eq!(
        goal.closed_feedback_loop,
        Some(crate::todo::FeedbackLoopState::Strong)
    );
    assert_eq!(
        goal.closed_feedback_loop_history,
        vec![
            crate::todo::FeedbackLoopState::Usable,
            crate::todo::FeedbackLoopState::Strong
        ]
    );

    let goals = parse(json!({
        "goals": [{"group": "optimize grep", "hill_climbability": "88", "feedback_loop": "bench"}]
    }))
    .expect("a pre-rename tool call must still parse")
    .goals
    .expect("goals should be present");
    assert_eq!(
        goals[0].closed_feedback_loop,
        Some(crate::todo::FeedbackLoopState::Strong)
    );
}

use crate::todo::ConfidenceState as CS;

fn history_todo(id: &str, confidence: Option<CS>, history: Vec<CS>) -> TodoItem {
    TodoItem {
        id: id.to_string(),
        content: format!("todo {id}"),
        status: "in_progress".to_string(),
        priority: "high".to_string(),
        confidence,
        confidence_history: history,
        ..Default::default()
    }
}

#[test]
fn confidence_history_appends_changes_and_skips_repeats() {
    let previous = vec![history_todo("1", Some(CS::Plausible), vec![CS::Plausible])];
    // Same confidence again: no new entry.
    let mut incoming = vec![history_todo("1", Some(CS::Plausible), Vec::new())];
    merge_confidence_history(&previous, &mut incoming);
    assert_eq!(incoming[0].confidence_history, vec![CS::Plausible]);
    // Raised confidence: appended.
    let mut incoming = vec![history_todo("1", Some(CS::Validated), Vec::new())];
    merge_confidence_history(&previous, &mut incoming);
    assert_eq!(
        incoming[0].confidence_history,
        vec![CS::Plausible, CS::Validated]
    );
}

#[test]
fn confidence_history_records_completion_confidence() {
    let previous = vec![history_todo("1", Some(CS::Plausible), vec![CS::Plausible])];
    let mut done = history_todo("1", Some(CS::Verified), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(CS::Verified);
    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);
    // 75 (planning) -> 100 (final bulk stamp): the spike stays visible.
    assert_eq!(
        incoming[0].confidence_history,
        vec![CS::Plausible, CS::Verified]
    );
}

#[test]
fn completion_write_contributes_only_one_final_confidence_observation() {
    let previous = vec![history_todo("1", Some(CS::Plausible), vec![CS::Plausible])];
    let mut done = history_todo("1", Some(CS::Plausible), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(CS::Verified);

    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);

    assert_eq!(
        incoming[0].confidence_history,
        vec![CS::Plausible, CS::Verified]
    );
}

#[test]
fn confidence_history_seeds_legacy_todos_before_completion() {
    let previous = vec![history_todo("1", Some(CS::Plausible), Vec::new())];
    let mut done = history_todo("1", Some(CS::Plausible), Vec::new());
    done.status = "completed".to_string();
    done.completion_confidence = Some(CS::Verified);

    let mut incoming = vec![done];
    merge_confidence_history(&previous, &mut incoming);

    assert_eq!(
        incoming[0].confidence_history,
        vec![CS::Plausible, CS::Verified]
    );
}

#[test]
fn confidence_history_ignores_model_supplied_history_for_new_todos() {
    let mut incoming = vec![history_todo(
        "9",
        Some(CS::Plausible),
        vec![CS::Speculative, CS::Verified],
    )];
    merge_confidence_history(&[], &mut incoming);
    assert_eq!(incoming[0].confidence_history, vec![CS::Plausible]);
}

include!("todo_tests/input_schema.rs");
