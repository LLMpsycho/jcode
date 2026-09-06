use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, TodoEvent};
use crate::todo::{
    GateObservation, GateObservationKind, SEVERE_INTENT_MISUNDERSTANDING,
    TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE, TodoGoal, TodoGoalChange, TodoGoalField,
    TodoItem, TodoPlan, TodoPlanChange, TodoPlanField, append_gate_observations,
    feedback_loop_passes, intent_understanding_passes, load_goals, load_plan, load_todos,
    save_goals, save_plan, save_todos, update_todo_review_cycle,
};
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct TodoTool;

impl TodoTool {
    pub fn new() -> Self {
        Self
    }
}

/// Fold each incoming todo's confidence into its tool-maintained history.
///
/// The model reports `confidence` while working and `completion_confidence` at
/// completion. Each todo-tool write contributes at most one observation so a
/// single completion update cannot manufacture an apparent intermediate step.
/// The append-only trail lets downstream consumers distinguish an
/// evidence-driven rise (75 -> 85 -> 95 -> 100) from a bulk end-of-task stamp
/// (75 -> 100). Model-supplied `confidence_history` is ignored: the tool owns
/// this field.
fn merge_confidence_history(previous: &[TodoItem], incoming: &mut [TodoItem]) {
    let prior: HashMap<&str, &TodoItem> = previous
        .iter()
        .map(|todo| (todo.id.as_str(), todo))
        .collect();
    for todo in incoming.iter_mut() {
        let previous_todo = prior.get(todo.id.as_str()).copied();
        let mut history = previous_todo
            .map(|prev| prev.confidence_history.clone())
            .unwrap_or_default();
        if history.is_empty()
            && let Some(value) = previous_todo.and_then(|prev| {
                if prev.status == "completed" {
                    prev.completion_confidence.or(prev.confidence)
                } else {
                    prev.confidence
                }
            })
        {
            history.push(value);
        }
        let observation = if todo.status == "completed" {
            todo.completion_confidence.or(todo.confidence)
        } else {
            todo.confidence
        };
        if let Some(value) = observation
            && history.last() != Some(&value)
        {
            history.push(value);
        }
        todo.confidence_history = history;
    }
}

#[derive(Deserialize)]
struct TodoInput {
    todos: Option<Vec<TodoItem>>,
    goals: Option<Vec<TodoGoal>>,
    plan: Option<TodoPlan>,
}

fn parse_todo_input(input: Value) -> Result<TodoInput> {
    let params: TodoInput = serde_json::from_value(normalize_todo_input(input))?;
    if let Some(criteria) = params
        .plan
        .as_ref()
        .and_then(|plan| plan.acceptance_criteria.as_ref())
        && (criteria.len() > 32 || criteria.iter().any(|criterion| criterion.len() > 1024))
    {
        bail!("acceptance_criteria supports at most 32 requirements of 1024 bytes each");
    }
    if let Some(todo) = params.todos.as_ref().and_then(|todos| {
        todos
            .iter()
            .find(|todo| crate::todo::canonical_todo_status(&todo.status).is_none())
    }) {
        bail!(
            "invalid todo status {:?}; expected one of: pending, in_progress, completed, cancelled",
            todo.status
        );
    }
    Ok(params)
}

/// Normalize a goal's group label: trimmed, with empty/whitespace collapsed
/// to `None` (the implicit goal of an ungrouped list).
fn goal_group_key(group: Option<&str>) -> Option<String> {
    group
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string)
}

fn todo_telemetry_update(
    previous: &[TodoItem],
    todos: &[TodoItem],
    goals: &[TodoGoal],
    plan: &TodoPlan,
) -> crate::telemetry::TodoTelemetryUpdate {
    let previous_by_id: HashMap<&str, &TodoItem> = previous
        .iter()
        .map(|todo| (todo.id.as_str(), todo))
        .collect();
    let current_by_id: HashMap<&str, &TodoItem> =
        todos.iter().map(|todo| (todo.id.as_str(), todo)).collect();

    let todos_created = current_by_id
        .keys()
        .filter(|id| !previous_by_id.contains_key(**id))
        .count()
        .min(u32::MAX as usize) as u32;
    let todos_completed = current_by_id
        .iter()
        .filter(|(id, todo)| {
            todo.status == "completed"
                && previous_by_id
                    .get(**id)
                    .is_none_or(|previous| previous.status != "completed")
        })
        .count()
        .min(u32::MAX as usize) as u32;
    let todos_abandoned = previous_by_id
        .iter()
        .filter(|(id, todo)| todo.status != "completed" && !current_by_id.contains_key(**id))
        .count()
        .min(u32::MAX as usize) as u32;
    let current_incomplete = current_by_id
        .values()
        .filter(|todo| todo.status != "completed")
        .count()
        .min(u32::MAX as usize) as u32;

    let mut group_completion: HashMap<Option<String>, bool> = HashMap::new();
    for todo in current_by_id.values() {
        let completed = todo.status == "completed";
        group_completion
            .entry(goal_group_key(todo.group.as_deref()))
            .and_modify(|all_completed| *all_completed &= completed)
            .or_insert(completed);
    }

    crate::telemetry::TodoTelemetryUpdate {
        todos_created,
        todos_completed,
        todos_abandoned,
        current_incomplete,
        list_size: todos.len().min(u32::MAX as usize) as u32,
        groups_completed: group_completion
            .values()
            .filter(|completed| **completed)
            .count()
            .min(u32::MAX as usize) as u32,
        groups_total: group_completion.len().min(u32::MAX as usize) as u32,
        confidence: crate::telemetry::TelemetryScoreSummary::from_scores(
            current_by_id
                .values()
                .filter_map(|todo| todo.confidence.map(|state| state.legacy_score())),
        ),
        completion_confidence: crate::telemetry::TelemetryScoreSummary::from_scores(
            current_by_id
                .values()
                .filter_map(|todo| todo.completion_confidence.map(|state| state.legacy_score())),
        ),
        understands_user_intent: crate::telemetry::TelemetryScoreSummary::from_scores(
            plan.understands_user_intent
                .map(|state| state.legacy_score()),
        ),
        closed_feedback_loop: crate::telemetry::TelemetryScoreSummary::from_scores(
            goals
                .iter()
                .filter_map(|goal| goal.closed_feedback_loop.map(|state| state.legacy_score())),
        ),
        feedback_loop_relevance: crate::telemetry::TelemetryScoreSummary::from_scores(
            goals.iter().filter_map(|goal| {
                goal.feedback_loop_relevance
                    .map(|state| state.legacy_score())
            }),
        ),
        feedback_loop_coverage: crate::telemetry::TelemetryScoreSummary::from_scores(
            goals.iter().filter_map(|goal| {
                goal.feedback_loop_coverage
                    .map(|state| state.legacy_score())
            }),
        ),
        end_to_end_ownership: crate::telemetry::TelemetryScoreSummary::from_scores(
            goals
                .iter()
                .filter_map(|goal| goal.delivery_state.map(|state| state.legacy_score())),
        ),
    }
}

fn record_todo_telemetry(
    previous: &[TodoItem],
    todos: &[TodoItem],
    goals: &[TodoGoal],
    plan: &TodoPlan,
) {
    crate::telemetry::record_todo_update(todo_telemetry_update(previous, todos, goals, plan));
}

/// Append `value` to `history` when it is a new observation.
///
/// One todo-tool write contributes at most one entry per score, so a single
/// bulk update cannot manufacture an apparent gradual climb.
fn record_score_observation<T: Copy + PartialEq>(history: &mut Vec<T>, value: Option<T>) {
    if let Some(value) = value
        && history.last() != Some(&value)
    {
        history.push(value);
    }
}

/// Merge incoming goal assessments with the stored ones.
///
/// Incoming goals win per group key; stored goals for groups the write does
/// not mention are retained (a todo update should not silently discard goal
/// assessments). Score histories are tool-maintained: whatever the model sends
/// for them is discarded in favor of the stored trail plus this write's
/// observation.
fn merge_goals(stored: &[TodoGoal], incoming: Option<Vec<TodoGoal>>) -> Vec<TodoGoal> {
    let Some(incoming) = incoming else {
        return stored.to_vec();
    };
    let mut merged: Vec<TodoGoal> = Vec::new();
    for mut goal in incoming {
        goal.group = goal_group_key(goal.group.as_deref());
        let previous = stored
            .iter()
            .find(|prev| goal_group_key(prev.group.as_deref()) == goal.group);
        goal.closed_feedback_loop_history = previous
            .map(|prev| prev.closed_feedback_loop_history.clone())
            .unwrap_or_default();
        goal.feedback_loop_relevance_history = previous
            .map(|prev| prev.feedback_loop_relevance_history.clone())
            .unwrap_or_default();
        goal.feedback_loop_coverage_history = previous
            .map(|prev| prev.feedback_loop_coverage_history.clone())
            .unwrap_or_default();
        goal.feedback_loop_traceability_history = previous
            .map(|prev| prev.feedback_loop_traceability_history.clone())
            .unwrap_or_default();
        goal.delivery_state_history = previous
            .map(|prev| prev.delivery_state_history.clone())
            .unwrap_or_default();
        // Field-level merge, matching `merge_plan`: a write that revises one
        // assessment must not silently erase the others. Without this the
        // turn-end digest would read a stale `None` and re-raise a point the
        // agent had already resolved.
        if let Some(prev) = previous {
            if goal.closed_feedback_loop.is_none() {
                goal.closed_feedback_loop = prev.closed_feedback_loop;
            }
            if goal.delivery_state.is_none() {
                goal.delivery_state = prev.delivery_state;
            }
            if goal.feedback_loop_relevance.is_none() {
                goal.feedback_loop_relevance = prev.feedback_loop_relevance;
            }
            if goal.feedback_loop_coverage.is_none() {
                goal.feedback_loop_coverage = prev.feedback_loop_coverage;
            }
            if goal.feedback_loop_traceability.is_none() {
                goal.feedback_loop_traceability = prev.feedback_loop_traceability;
            }
            if goal.difficulty.is_none() {
                goal.difficulty = prev.difficulty;
            }
            if goal.autonomy.is_none() {
                goal.autonomy = prev.autonomy;
            }
            if goal.iteration_maturity.is_none() {
                goal.iteration_maturity = prev.iteration_maturity;
            }
            if goal.feedback_loop.is_none() {
                goal.feedback_loop = prev.feedback_loop.clone();
            }
            if goal.stopping_evidence.is_none() {
                goal.stopping_evidence = prev.stopping_evidence.clone();
            }
        }
        record_score_observation(
            &mut goal.closed_feedback_loop_history,
            goal.closed_feedback_loop,
        );
        record_score_observation(
            &mut goal.feedback_loop_relevance_history,
            goal.feedback_loop_relevance,
        );
        record_score_observation(
            &mut goal.feedback_loop_coverage_history,
            goal.feedback_loop_coverage,
        );
        record_score_observation(
            &mut goal.feedback_loop_traceability_history,
            goal.feedback_loop_traceability,
        );
        record_score_observation(&mut goal.delivery_state_history, goal.delivery_state);
        if let Some(slot) = merged
            .iter_mut()
            .find(|existing| existing.group == goal.group)
        {
            *slot = goal;
        } else {
            merged.push(goal);
        }
    }
    for prev in stored {
        let key = goal_group_key(prev.group.as_deref());
        if !merged.iter().any(|goal| goal.group == key) {
            merged.push(prev.clone());
        }
    }
    merged
}

/// Drop retained goals whose todo group no longer exists in `todos`.
///
/// `merge_goals` deliberately keeps goals a write does not mention, so a
/// goals-only or partial update cannot erase assessments. But when the agent
/// moves on to a new task and replaces the todo list wholesale, goals from the
/// finished task have no todos left to describe and were shown indefinitely in
/// the todos panel (issue #695). Goals whose group still has a todo, and the
/// ungrouped goal for a flat list, are always kept.
fn prune_orphaned_goals(goals: Vec<TodoGoal>, todos: &[TodoItem]) -> Vec<TodoGoal> {
    if todos.is_empty() {
        return goals;
    }
    let live_groups: std::collections::HashSet<Option<String>> = todos
        .iter()
        .map(|todo| goal_group_key(todo.group.as_deref()))
        .collect();
    goals
        .into_iter()
        .filter(|goal| live_groups.contains(&goal_group_key(goal.group.as_deref())))
        .collect()
}

fn changed_goal_fields(before: Option<&TodoGoal>, after: Option<&TodoGoal>) -> Vec<TodoGoalField> {
    let mut fields = Vec::new();
    if before.and_then(|goal| goal.closed_feedback_loop)
        != after.and_then(|goal| goal.closed_feedback_loop)
    {
        fields.push(TodoGoalField::ClosedFeedbackLoop);
    }
    if before.and_then(|goal| goal.feedback_loop.as_ref())
        != after.and_then(|goal| goal.feedback_loop.as_ref())
    {
        fields.push(TodoGoalField::FeedbackLoop);
    }
    if before.and_then(|goal| goal.feedback_loop_relevance)
        != after.and_then(|goal| goal.feedback_loop_relevance)
    {
        fields.push(TodoGoalField::FeedbackLoopRelevance);
    }
    if before.and_then(|goal| goal.feedback_loop_coverage)
        != after.and_then(|goal| goal.feedback_loop_coverage)
    {
        fields.push(TodoGoalField::FeedbackLoopCoverage);
    }
    if before.and_then(|goal| goal.feedback_loop_traceability)
        != after.and_then(|goal| goal.feedback_loop_traceability)
    {
        fields.push(TodoGoalField::FeedbackLoopTraceability);
    }
    if before.and_then(|goal| goal.delivery_state) != after.and_then(|goal| goal.delivery_state) {
        fields.push(TodoGoalField::DeliveryState);
    }
    if before.and_then(|goal| goal.autonomy) != after.and_then(|goal| goal.autonomy) {
        fields.push(TodoGoalField::Autonomy);
    }
    if before.and_then(|goal| goal.iteration_maturity)
        != after.and_then(|goal| goal.iteration_maturity)
    {
        fields.push(TodoGoalField::IterationMaturity);
    }
    if before.and_then(|goal| goal.stopping_evidence.as_ref())
        != after.and_then(|goal| goal.stopping_evidence.as_ref())
    {
        fields.push(TodoGoalField::StoppingEvidence);
    }
    fields
}

/// Merge the incoming plan-level intent assessment with the stored one.
///
/// User intention describes why the user asked for the work and should remain
/// stable while the agent revises its steps or scores, so an omitted intention
/// inherits the stored value. Sending an empty string clears it. The intent
/// score's history is tool-maintained, so a model-supplied trail is discarded.
fn merge_plan(stored: &TodoPlan, incoming: Option<TodoPlan>) -> TodoPlan {
    let Some(mut plan) = incoming else {
        return stored.clone();
    };
    if plan.acceptance_criteria.is_none() {
        plan.acceptance_criteria = stored.acceptance_criteria.clone();
    }
    if plan.user_intention.is_none() {
        plan.user_intention = stored.user_intention.clone();
    }
    if plan.understands_user_intent.is_none() {
        plan.understands_user_intent = stored.understands_user_intent;
    }
    plan.understands_user_intent_history = stored.understands_user_intent_history.clone();
    record_score_observation(
        &mut plan.understands_user_intent_history,
        plan.understands_user_intent,
    );
    plan
}

fn plan_change(before: &TodoPlan, after: &TodoPlan) -> Option<TodoPlanChange> {
    let mut fields = Vec::new();
    if before.acceptance_criteria != after.acceptance_criteria {
        fields.push(TodoPlanField::AcceptanceCriteria);
    }
    if before.user_intention != after.user_intention {
        fields.push(TodoPlanField::UserIntention);
    }
    if before.understands_user_intent != after.understands_user_intent {
        fields.push(TodoPlanField::UnderstandsUserIntent);
    }
    (!fields.is_empty()).then(|| TodoPlanChange {
        before: Some(before.clone()),
        after: Some(after.clone()),
        fields,
    })
}

fn goal_changes(before: &[TodoGoal], after: &[TodoGoal]) -> Vec<TodoGoalChange> {
    let mut changes = Vec::new();
    for current in after {
        let key = goal_group_key(current.group.as_deref());
        let previous = before
            .iter()
            .find(|goal| goal_group_key(goal.group.as_deref()) == key);
        let fields = changed_goal_fields(previous, Some(current));
        if !fields.is_empty() {
            changes.push(TodoGoalChange {
                before: previous.cloned(),
                after: Some(current.clone()),
                fields,
            });
        }
    }
    for previous in before {
        let key = goal_group_key(previous.group.as_deref());
        if after
            .iter()
            .any(|goal| goal_group_key(goal.group.as_deref()) == key)
        {
            continue;
        }
        let fields = changed_goal_fields(Some(previous), None);
        if !fields.is_empty() {
            changes.push(TodoGoalChange {
                before: Some(previous.clone()),
                after: None,
                fields,
            });
        }
    }
    changes
}

/// Record the points this write would previously have interrupted on, and
/// return the rare continuation that is still worth sending immediately.
///
/// Previously both checks emitted a continuation on every applicable write for
/// as long as the score stayed low. That punished the common healthy case:
/// understanding of a request starts low and rises as the agent explores, so an
/// agent already resolving the ambiguity was repeatedly told to stop and go
/// resolve the ambiguity. On long iterative turns the same text reattached to
/// every todo call, spending reasoning on re-justifying the plan instead of on
/// the work.
///
/// So the checks are deferred: observations accumulate and are replayed once at
/// turn end by `build_gate_digest`. Deferred, not forgiven. A score that climbs
/// late is still raised, because the work done while it was low was never
/// governed by the better loop that arrived afterwards. The one exception is a
/// first plan write that scores severely low, where the agent is admitting it
/// does not know the task at all and a whole turn of wrong work cannot be undone
/// at turn end.
fn record_reframe_observations(
    plan: &TodoPlan,
    goals: &[TodoGoal],
    todos: &[TodoItem],
    previous: &[TodoItem],
) -> (Vec<GateObservation>, Vec<String>) {
    let mut observations = Vec::new();
    let mut immediate = Vec::new();
    let any_open = todos
        .iter()
        .any(|todo| todo.status != "completed" && todo.status != "cancelled");
    if any_open && !intent_understanding_passes(plan.understands_user_intent) {
        observations.push(GateObservation {
            kind: GateObservationKind::IntentUnderstanding,
            group: None,
            state: plan
                .understands_user_intent
                .map(|state| state.as_str().to_string()),
        });
        // Only on the first observation of the plan, so a persistently low
        // assessment is reported once at turn end rather than on every write.
        let first_assessment = plan.understands_user_intent_history.len() <= 1;
        if first_assessment
            && plan
                .understands_user_intent
                .is_some_and(|state| state <= SEVERE_INTENT_MISUNDERSTANDING)
        {
            immediate.push(TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE.to_string());
        }
    }
    let closed_now = crate::todo::groups_closed_by_update(previous, todos);
    for goal in goals {
        let group_open = todos.iter().any(|todo| {
            goal_group_key(todo.group.as_deref()) == goal.group
                && todo.status != "completed"
                && todo.status != "cancelled"
        });
        // A group this write closes counts too: a goal created and finished in
        // one step is otherwise never observed, and one-step completions are
        // where a weak feedback loop hides best.
        if !group_open && !closed_now.contains(&goal.group) {
            continue;
        }
        if !feedback_loop_passes(goal.closed_feedback_loop) {
            observations.push(GateObservation {
                kind: GateObservationKind::ClosedFeedbackLoop,
                group: goal.group.clone(),
                state: goal
                    .closed_feedback_loop
                    .map(|state| state.as_str().to_string()),
            });
        }
        if !crate::todo::feedback_loop_relevance_passes(goal) {
            observations.push(GateObservation {
                kind: GateObservationKind::FeedbackLoopRelevance,
                group: goal.group.clone(),
                state: goal
                    .feedback_loop_relevance
                    .map(|state| state.as_str().to_string()),
            });
        }
        if !crate::todo::feedback_loop_coverage_passes(goal) {
            observations.push(GateObservation {
                kind: GateObservationKind::FeedbackLoopCoverage,
                group: goal.group.clone(),
                state: goal
                    .feedback_loop_coverage
                    .map(|state| state.as_str().to_string()),
            });
        }
        if !crate::todo::feedback_loop_traceability_passes(goal) {
            observations.push(GateObservation {
                kind: GateObservationKind::FeedbackLoopTraceability,
                group: goal.group.clone(),
                state: goal
                    .feedback_loop_traceability
                    .map(|state| state.as_str().to_string()),
            });
        }
    }
    (observations, immediate)
}

fn build_todo_output(
    todos: Vec<TodoItem>,
    plan: TodoPlan,
    goals: Vec<TodoGoal>,
    plan_change: Option<TodoPlanChange>,
    goal_changes: Option<Vec<TodoGoalChange>>,
    continuations: impl IntoIterator<Item = String>,
) -> Result<ToolOutput> {
    let remaining = todos
        .iter()
        .filter(|todo| todo.status != "completed")
        .count();
    let mut text = serde_json::to_string_pretty(&todos)?;
    if plan != TodoPlan::default() {
        text.push_str("\n\nPlan:\n");
        text.push_str(&serde_json::to_string_pretty(&plan)?);
    }
    if !goals.is_empty() {
        text.push_str("\n\nGoals:\n");
        text.push_str(&serde_json::to_string_pretty(&goals)?);
    }
    if let Some(plan_change) = plan_change.as_ref() {
        text.push_str("\n\nPlan updates:\n");
        text.push_str(&serde_json::to_string_pretty(plan_change)?);
    }
    if let Some(goal_changes) = goal_changes.as_ref().filter(|changes| !changes.is_empty()) {
        text.push_str("\n\nGoal updates:\n");
        text.push_str(&serde_json::to_string_pretty(goal_changes)?);
    }
    for continuation in continuations {
        text.push_str("\n\n");
        text.push_str(&continuation);
    }
    let mut metadata = json!({"todos": todos, "plan": plan, "goals": goals});
    if let Some(plan_change) = plan_change {
        metadata["plan_update"] = serde_json::to_value(plan_change)?;
    }
    if let Some(goal_changes) = goal_changes.filter(|changes| !changes.is_empty()) {
        metadata["goal_updates"] = serde_json::to_value(goal_changes)?;
    }
    Ok(ToolOutput::new(text)
        .with_title(format!("{} todos", remaining))
        .with_metadata(metadata))
}

/// Leniently normalize raw todo-tool arguments before strict deserialization.
///
/// Some providers (notably Claude tool calling) intermittently emit tool
/// arguments as JSON *strings* instead of native types: the whole `todos`
/// array as one stringified JSON blob, individual items as stringified
/// objects, or numeric fields like `confidence` as `"90"`. Strict
/// `serde_json::from_value` rejects these with `invalid type: string ...`,
/// failing the entire call (issue #357; same provider quirk as #106).
fn normalize_todo_input(mut input: Value) -> Value {
    let Some(obj) = input.as_object_mut() else {
        return input;
    };
    if let Some(plan) = obj.get_mut("plan") {
        if let Value::String(raw) = plan {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                *plan = Value::Null;
            } else if let Ok(parsed @ (Value::Object(_) | Value::Null)) =
                serde_json::from_str::<Value>(trimmed)
            {
                *plan = parsed;
            }
        }
        if let Some(fields) = plan.as_object_mut() {
            for key in [
                "alignment_score",
                "user_intention_alignment",
                "understands_user_intent",
            ] {
                if let Some(value) = fields.get_mut(key) {
                    coerce_empty_string_to_null(value);
                }
            }
        }
    }
    for key in ["todos", "goals"] {
        let Some(entries) = obj.get_mut(key) else {
            continue;
        };

        // Whole array sent as a stringified JSON blob.
        if let Value::String(raw) = entries {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                *entries = Value::Null;
            } else if let Ok(parsed @ (Value::Array(_) | Value::Null)) =
                serde_json::from_str::<Value>(trimmed)
            {
                *entries = parsed;
            }
        }

        if let Value::Array(items) = entries {
            for item in items.iter_mut() {
                // Individual item sent as a stringified JSON object.
                if let Value::String(raw) = item
                    && let Ok(parsed @ Value::Object(_)) = serde_json::from_str::<Value>(raw.trim())
                {
                    *item = parsed;
                }
                let Some(fields) = item.as_object_mut() else {
                    continue;
                };
                if key == "todos"
                    && let Some(Value::String(status)) = fields.get_mut("status")
                    && let Some(canonical) = crate::todo::canonical_todo_status(status)
                {
                    *status = canonical.to_string();
                }
                for key in [
                    "confidence",
                    "completion_confidence",
                    "alignment_score",
                    "user_intention_alignment",
                    "closed_feedback_loop",
                    // Pre-rename aliases; some prompts and replayed transcripts
                    // still carry the old keys.
                    "hill_climbability",
                    "end_to_end_ownership",
                    "delivery_state",
                    "feedback_loop_relevance",
                    "feedback_loop_coverage",
                    "feedback_loop_traceability",
                    "difficulty",
                    "autonomy",
                ] {
                    if let Some(value) = fields.get_mut(key) {
                        coerce_empty_string_to_null(value);
                    }
                }
            }
        }
    }
    input
}

/// Coerce an empty or whitespace-only string to `null` so an omitted-but-sent
/// assessment reads as absent. Numeric legacy scores (ints, floats, numeric
/// strings) are handled by the semantic-state deserializers themselves, so no
/// numeric coercion is needed here anymore.
fn coerce_empty_string_to_null(value: &mut Value) {
    if let Value::String(raw) = value
        && raw.trim().is_empty()
    {
        *value = Value::Null;
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn capability(&self, input: &serde_json::Value) -> crate::tool::ToolCapability {
        use crate::tool::ToolCapability;
        if ["todos", "goals", "plan"]
            .iter()
            .any(|key| input.get(key).is_some_and(|v| !v.is_null()))
        {
            ToolCapability::ChangeState
        } else {
            ToolCapability::ReadOnly
        }
    }

    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        // SECURITY/EVAL: This is model-visible calibration text. Keep it
        // deliberately handwritten. Never generate it from gate constants or
        // interpolate private thresholds, because that would teach the model
        // how to target the evaluator instead of reporting an honest assessment.
        "Read or update structured todo items and optional goal-level assessments."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "todos": {
                    "type": "array",
                    "description": "Todo list to save.",
                    "items": {
                        "type": "object",
                        "required": ["content", "status", "priority", "id", "confidence"],
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Task."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "Status. Use completed when the task is done."
                            },
                            "priority": {
                                "type": "string",
                                "description": "Priority."
                            },
                            "id": {
                                "type": "string",
                                "description": "ID."
                            },
                            "group": {
                                "type": "string",
                                "description": "Optional group label; one group per coherent goal, new direction = new group. Omit for flat list."
                            },
                            "confidence": {
                                "type": "string",
                                "enum": ["speculative", "plausible", "validated", "verified"],
                                "description": "Evidence state that this todo can be completed correctly; reassess as evidence accumulates."
                            },
                            "completion_confidence": {
                                "type": "string",
                                "enum": ["speculative", "plausible", "validated", "verified"],
                                "description": "Evidence state behind this todo's completion. Use only for completed items."
                            },
                        }
                    }
                },
                "plan": {
                    "type": "object",
                    "description": "Plan-level understanding of the request. Send on first write and whenever understanding changes.",
                    "required": ["user_intention", "understands_user_intent"],
                    "properties": {
                        "acceptance_criteria": {
                            "type": "array", "maxItems": 32,
                            "items": {"type": "string", "maxLength": 1024},
                            "description": "Literal user acceptance requirements, separate from plan steps. Omit to retain; an empty array clears them. These are not verification results."
                        },
                        "user_intention": {
                            "type": "string",
                            "description": "What the user actually wants: underlying reason and desired end state. Omit later to retain."
                        },
                        "understands_user_intent": {
                            "type": "string",
                            "enum": ["uncertain", "partial", "clear", "complete"],
                            "description": "How well you understand what the user wants. Report uncertain or partial when guessing."
                        }
                    }
                },
                "goals": {
                    "type": "array",
                    "description": "Goal-level assessments, one per todo group (null = ungrouped). Omitted groups are retained.",
                    "items": {
                        "type": "object",
                        "required": ["closed_feedback_loop", "feedback_loop", "feedback_loop_relevance", "feedback_loop_coverage", "feedback_loop_traceability"],
                        "properties": {
                            "group": {
                                "type": "string",
                                "description": "Group label this goal describes. Omit or null for the ungrouped list."
                            },
                            "closed_feedback_loop": {
                                "type": "string",
                                "enum": ["absent", "weak", "usable", "strong", "closed"],
                                "description": "How much of this goal's correctness the feedback_loop can verify on its own."
                            },
                            "feedback_loop": {
                                "type": "string",
                                "description": "Requirement-to-check process: an explicit observation or check for each requirement of this goal."
                            },
                            "feedback_loop_relevance": {
                                "type": "string",
                                "enum": ["indirect", "synthetic", "representative", "acceptance_blocked", "acceptance_aligned"],
                                "description": "How directly checks represent observable acceptance behavior. indirect = inspection or an internal proxy; synthetic = custom harnesses, stubs, mocks, copied sources, or synthetic fixtures; representative = real public interfaces but not the complete acceptance workflow; acceptance_blocked = the real acceptance workflow was attempted but an external constraint prevented a result; acceptance_aligned = the real project build, integration test, or end-user workflow passed. Substitute-only validation is never acceptance_aligned."
                            },
                            "feedback_loop_coverage": {
                                "type": "string",
                                "enum": ["narrow", "main_paths", "edge_and_integration_paths"],
                                "description": "How broadly the checks exercise main workflows, integration boundaries, edge cases, packaging, and likely failure modes."
                            },
                            "feedback_loop_traceability": {
                                "type": "string",
                                "enum": ["unmapped", "partial", "complete"],
                                "description": "How completely requirements map to evidence. unmapped = requirements are not tied to checks; partial = only some explicit requirements or changed public outputs have concrete checks and observed results; complete = every explicit requirement and changed public output has a concrete check and observed result. Aggregate test counts alone do not establish complete traceability."
                            },
                            "delivery_state": {
                                "type": "string",
                                "enum": ["change_made", "integrated", "workflow_validated", "outcome_delivered"],
                                "description": "Completion-time: how far the result actually traveled toward the user's outcome."
                            },
                            "difficulty": {
                                "type": "string",
                                "enum": ["trivial", "routine", "involved", "complex", "hard", "expert", "research", "open_ended"],
                                "description": "Honest intrinsic difficulty of this goal. Descriptive only."
                            },
                            "autonomy": {
                                "type": "string",
                                "enum": ["requested_only", "necessary_followthrough", "proactive", "stewardship"],
                                "description": "How far beyond the literal request the work went. Assess from what was completed."
                            },
                            "iteration_maturity": {
                                "type": "string",
                                "enum": ["not_started", "exploring", "improving", "plateau_unproven", "outcome_reached", "constraints_exhausted", "plateau_confirmed", "budget_exhausted"],
                                "description": "How far the feedback loop was actually exercised, and any evidence-based reason to stop iterating."
                            },
                            "stopping_evidence": {
                                "type": "string",
                                "description": "Evidence for the reported iteration_maturity: attempts, observations, or a real budget limit."
                            }
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params = parse_todo_input(input)?;
        let is_write = params.todos.is_some() || params.goals.is_some() || params.plan.is_some();
        let operation = if is_write { "write" } else { "read" };
        let result = if is_write {
            // Goals/plan-only writes keep the stored todo list.
            let previous = load_todos(&ctx.session_id).unwrap_or_default();
            let mut todos = params.todos.unwrap_or_else(|| previous.clone());
            merge_confidence_history(&previous, &mut todos);
            (|| {
                let stored_goals = load_goals(&ctx.session_id).unwrap_or_default();
                let stored_plan = load_plan(&ctx.session_id).unwrap_or_default();
                let goals = prune_orphaned_goals(merge_goals(&stored_goals, params.goals), &todos);
                let plan = merge_plan(&stored_plan, params.plan);
                let (observations, nudges) =
                    record_reframe_observations(&plan, &goals, &todos, &previous);
                for observation in &observations {
                    let kind = match observation.kind {
                        GateObservationKind::IntentUnderstanding => {
                            crate::telemetry::TodoGateKind::IntentUnderstanding
                        }
                        GateObservationKind::ClosedFeedbackLoop => {
                            crate::telemetry::TodoGateKind::ClosedFeedbackLoop
                        }
                        GateObservationKind::FeedbackLoopRelevance => {
                            crate::telemetry::TodoGateKind::FeedbackLoopRelevance
                        }
                        GateObservationKind::FeedbackLoopCoverage => {
                            crate::telemetry::TodoGateKind::FeedbackLoopCoverage
                        }
                        GateObservationKind::FeedbackLoopTraceability => {
                            crate::telemetry::TodoGateKind::FeedbackLoopTraceability
                        }
                    };
                    crate::telemetry::record_todo_gate(kind);
                }
                // Best-effort: a failure to persist the observation log must not
                // fail the todo write itself. The cost is a missing reminder.
                if let Err(err) = append_gate_observations(&ctx.session_id, &observations) {
                    crate::logging::warn(&format!(
                        "[tool:todo] failed to record gate observations session_id={} error={}",
                        ctx.session_id, err
                    ));
                }
                // Assessment-only writes, especially quality-gate retries,
                // should render the fields that changed instead of repeating an
                // otherwise identical todo plan.
                let assessment_only = todos == previous;
                let concise_goal_changes = (assessment_only && !stored_goals.is_empty())
                    .then(|| goal_changes(&stored_goals, &goals));
                let concise_plan_change = assessment_only
                    .then(|| plan_change(&stored_plan, &plan))
                    .flatten();
                save_todos(&ctx.session_id, &todos)?;
                save_goals(&ctx.session_id, &goals)?;
                save_plan(&ctx.session_id, &plan)?;
                if let Err(err) = update_todo_review_cycle(&ctx.session_id, &previous, &todos) {
                    crate::logging::warn(&format!(
                        "[tool:todo] failed to update review cycle session_id={} error={}",
                        ctx.session_id, err
                    ));
                }
                record_todo_telemetry(&previous, &todos, &goals, &plan);

                Bus::global().publish(BusEvent::TodoUpdated(TodoEvent {
                    session_id: ctx.session_id.clone(),
                    todos: todos.clone(),
                }));

                build_todo_output(
                    todos,
                    plan,
                    goals,
                    concise_plan_change,
                    concise_goal_changes,
                    nudges,
                )
            })()
        } else {
            (|| {
                let todos = load_todos(&ctx.session_id)?;
                let goals = load_goals(&ctx.session_id).unwrap_or_default();
                let plan = load_plan(&ctx.session_id).unwrap_or_default();
                record_todo_telemetry(&todos, &todos, &goals, &plan);
                build_todo_output(todos, plan, goals, None, None, Vec::new())
            })()
        };
        result.map_err(|err| {
            crate::logging::warn(&format!(
                "[tool:todo] operation failed operation={} session_id={} error={}",
                operation, ctx.session_id, err
            ));
            err
        })
    }
}

#[cfg(test)]
#[path = "todo_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "todo_advisor_tests.rs"]
mod advisor_tests;
