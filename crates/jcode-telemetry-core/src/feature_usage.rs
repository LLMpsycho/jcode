//! Feature usage.

use super::*;

pub(super) fn increment_tool_category(state: &mut SessionTelemetry, category: ToolCategory) {
    match category {
        ToolCategory::ReadSearch => state.tool_cat_read_search += 1,
        ToolCategory::Write => state.tool_cat_write += 1,
        ToolCategory::Shell => state.tool_cat_shell += 1,
        ToolCategory::Web => state.tool_cat_web += 1,
        ToolCategory::Memory => state.tool_cat_memory += 1,
        ToolCategory::Subagent => state.tool_cat_subagent += 1,
        ToolCategory::Swarm => state.tool_cat_swarm += 1,
        ToolCategory::Email => state.tool_cat_email += 1,
        ToolCategory::SidePanel => state.tool_cat_side_panel += 1,
        ToolCategory::Goal => state.tool_cat_goal += 1,
        ToolCategory::Todo => {
            state.tool_cat_todo += 1;
            state.todo.todo_updates = state.todo.todo_updates.saturating_add(1);
        }
        ToolCategory::Mcp => state.tool_cat_mcp += 1,
        ToolCategory::Other => state.tool_cat_other += 1,
    }
}

pub(super) fn increment_turn_tool_category(state: &mut TurnTelemetry, category: ToolCategory) {
    match category {
        ToolCategory::ReadSearch => state.tool_cat_read_search += 1,
        ToolCategory::Write => state.tool_cat_write += 1,
        ToolCategory::Shell => state.tool_cat_shell += 1,
        ToolCategory::Web => state.tool_cat_web += 1,
        ToolCategory::Memory => state.tool_cat_memory += 1,
        ToolCategory::Subagent => state.tool_cat_subagent += 1,
        ToolCategory::Swarm => state.tool_cat_swarm += 1,
        ToolCategory::Email => state.tool_cat_email += 1,
        ToolCategory::SidePanel => state.tool_cat_side_panel += 1,
        ToolCategory::Goal => state.tool_cat_goal += 1,
        ToolCategory::Todo => state.tool_cat_todo += 1,
        ToolCategory::Mcp => state.tool_cat_mcp += 1,
        ToolCategory::Other => state.tool_cat_other += 1,
    }
}

pub(super) fn mark_command_family_usage(state: &mut SessionTelemetry, command: &str) {
    let family = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('/');
    match family {
        "login" | "auth" => state.command_login_used = true,
        "model" => state.command_model_used = true,
        "usage" => state.command_usage_used = true,
        "resume" | "session" | "back" | "catchup" => state.command_resume_used = true,
        "memory" => state.command_memory_used = true,
        "swarm" | "agents" => state.command_swarm_used = true,
        "goal" | "goals" => state.command_goal_used = true,
        "selfdev" | "dev" => state.command_selfdev_used = true,
        "feedback" => state.command_feedback_used = true,
        _ => state.command_other_used = true,
    }
}

pub(super) fn mark_tool_feature_usage(state: &mut SessionTelemetry, name: &str, input: &Value) {
    let category = classify_tool_category(name);
    increment_tool_category(state, category);
    if let Some(turn) = state.current_turn.as_mut() {
        increment_turn_tool_category(turn, category);
    }

    match name {
        "memory" => {
            state.feature_memory_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_memory_used = true;
            }
        }
        "communicate" => {
            state.feature_swarm_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_swarm_used = true;
            }
        }
        "webfetch" | "websearch" | "codesearch" => {
            state.feature_web_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_web_used = true;
            }
        }
        "gmail" => {
            state.feature_email_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_email_used = true;
            }
        }
        "side_panel" => {
            state.feature_side_panel_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_side_panel_used = true;
            }
        }
        "initiative" => {
            state.feature_goal_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_goal_used = true;
            }
        }
        "todo" | "todowrite" | "todo_write" | "todoread" | "todo_read" => {
            state.feature_todo_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_todo_used = true;
            }
        }
        "selfdev" => {
            state.feature_selfdev_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_selfdev_used = true;
            }
        }
        "bg" | "schedule" => {
            state.feature_background_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_background_used = true;
            }
        }
        "subagent" => {
            state.feature_subagent_used = true;
            if let Some(turn) = state.current_turn.as_mut() {
                turn.feature_subagent_used = true;
            }
        }
        _ => {}
    }

    if matches!(
        name,
        "write" | "edit" | "multiedit" | "patch" | "apply_patch"
    ) {
        state.file_write_calls += 1;
        if let Some(turn) = state.current_turn.as_mut() {
            turn.file_write_calls += 1;
        }
    }

    if name == "mcp" || name.starts_with("mcp__") {
        state.feature_mcp_used = true;
        if let Some(turn) = state.current_turn.as_mut() {
            turn.feature_mcp_used = true;
        }
        if let Some(server) = mcp_server_name(name, input) {
            state.unique_mcp_servers.insert(server);
            if let Some(turn) = state.current_turn.as_mut()
                && let Some(server) = mcp_server_name(name, input)
            {
                turn.unique_mcp_servers.insert(server);
            }
        }
    }

    if looks_like_test_run(name, input) {
        state.tests_run += 1;
        if let Some(turn) = state.current_turn.as_mut() {
            turn.tests_run += 1;
        }
    }
}

pub(super) fn mark_tool_success_side_effects(
    state: &mut SessionTelemetry,
    name: &str,
    input: &Value,
) {
    if looks_like_test_run(name, input) {
        state.tests_passed += 1;
        if state.first_test_pass_ms.is_none() {
            state.first_test_pass_ms = Some(now_ms_since(state.started_at));
        }
        if let Some(turn) = state.current_turn.as_mut() {
            turn.tests_passed += 1;
            if turn.first_test_pass_ms.is_none() {
                turn.first_test_pass_ms = Some(now_ms_since(turn.started_at));
            }
        }
    }

    if state.first_tool_success_ms.is_none() {
        state.first_tool_success_ms = Some(now_ms_since(state.started_at));
    }
    if let Some(turn) = state.current_turn.as_mut()
        && turn.first_tool_success_ms.is_none()
    {
        turn.first_tool_success_ms = Some(now_ms_since(turn.started_at));
    }

    if matches!(
        name,
        "write" | "edit" | "multiedit" | "patch" | "apply_patch"
    ) && state.first_file_edit_ms.is_none()
    {
        state.first_file_edit_ms = Some(now_ms_since(state.started_at));
    }
    if matches!(
        name,
        "write" | "edit" | "multiedit" | "patch" | "apply_patch"
    ) && let Some(turn) = state.current_turn.as_mut()
        && turn.first_file_edit_ms.is_none()
    {
        turn.first_file_edit_ms = Some(now_ms_since(turn.started_at));
    }

    if name == "memory" {
        state.feature_memory_used = true;
        if let Some(turn) = state.current_turn.as_mut() {
            turn.feature_memory_used = true;
        }
    }
}
