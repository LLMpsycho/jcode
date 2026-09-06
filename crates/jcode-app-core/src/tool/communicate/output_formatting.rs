//! Output formatting.

use super::*;

pub(super) fn format_context_entries(entries: &[ContextEntry]) -> ToolOutput {
    ToolOutput::new(format_comm_context_entries(entries))
}

pub(super) fn format_members(ctx: &ToolContext, members: &[AgentInfo]) -> ToolOutput {
    ToolOutput::new(format_comm_members(&ctx.session_id, members))
}

pub(super) fn format_tool_summary(target: &str, calls: &[ToolCallSummary]) -> ToolOutput {
    ToolOutput::new(format_comm_tool_summary(target, calls))
}

pub(super) fn format_status_snapshot(snapshot: &AgentStatusSnapshot) -> ToolOutput {
    ToolOutput::new(format_comm_status_snapshot(snapshot))
}

pub(super) fn format_context_history(target: &str, messages: &[HistoryMessage]) -> ToolOutput {
    ToolOutput::new(format_comm_context_history(target, messages))
}

pub(super) fn format_channels(channels: &[SwarmChannelInfo]) -> ToolOutput {
    ToolOutput::new(format_comm_channels(channels))
}
