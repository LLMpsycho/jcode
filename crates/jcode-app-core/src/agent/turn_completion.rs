//! Terminal response recovery and advisor drain before the server emits Done.

use super::*;

pub(super) struct TerminalResponse<'a> {
    pub stop_reason: Option<&'a str>,
    pub saw_message_end: bool,
    pub text: &'a str,
    pub reasoning: &'a str,
    pub prompt_has_recent_tool_result: bool,
}

impl Agent {
    /// True means another provider request is needed. Called only after all
    /// tool results have been persisted, preserving tool-use/result adjacency.
    pub(super) async fn finish_streaming_response(
        &mut self,
        response: TerminalResponse<'_>,
        guardrail_reconsiderations: &mut u32,
        empty_post_tool_continuations: &mut u32,
        incomplete_continuations: &mut u32,
        event_tx: &mpsc::UnboundedSender<ServerEvent>,
    ) -> Result<bool> {
        if response.saw_message_end
            && !self.is_graceful_shutdown()
            && self.maybe_reconsider_fable_guardrail(
                response.stop_reason,
                guardrail_reconsiderations,
            )?
        {
            return Ok(true);
        }
        if response.saw_message_end
            && !self.is_graceful_shutdown()
            && self.maybe_continue_empty_post_tool_response(
                response.text.trim().is_empty(),
                response.prompt_has_recent_tool_result,
                response.stop_reason,
                empty_post_tool_continuations,
            )?
        {
            return Ok(true);
        }
        if self.is_graceful_shutdown() {
            crate::advisor::advisor_manager().cancel_turn(&self.session.id);
            return Ok(false);
        }
        // Recover incomplete protocol responses before treating an answer as
        // terminal. The advisor must not finalize a truncated tool request.
        if self
            .maybe_continue_incomplete_response(response.stop_reason, incomplete_continuations)?
            || self
                .maybe_continue_stranded_tool_use(response.stop_reason, incomplete_continuations)?
        {
            return Ok(true);
        }
        if response.saw_message_end && self.finish_advisor_step(Some(event_tx), false).await {
            return Ok(true);
        }
        match self.handle_streaming_no_tool_calls(response.stop_reason, incomplete_continuations)? {
            NoToolCallOutcome::Break => {
                // A deliberate provider refusal gets a visible status notice.
                // Explicit cancellation must not produce a spurious refusal.
                if response.saw_message_end
                    && !self.is_graceful_shutdown()
                    && let Some(notice) = Self::provider_guardrail_notice(
                        response.stop_reason,
                        response.text.trim().is_empty(),
                        !response.reasoning.trim().is_empty(),
                    )
                {
                    logging::warn(&format!(
                        "{}: turn ended with no visible output (stop_reason={:?}, reasoning_chars={})",
                        Self::empty_turn_log_event(response.stop_reason),
                        response.stop_reason,
                        response.reasoning.len()
                    ));
                    let _ = event_tx.send(ServerEvent::ProviderGuardrail {
                        stop_reason: response.stop_reason.map(str::to_string),
                        message: notice,
                    });
                }
                Ok(false)
            }
            NoToolCallOutcome::ContinueWithoutEvent => Ok(true),
            NoToolCallOutcome::ContinueWithSoftInterrupt { injected, point } => {
                for event in Self::build_soft_interrupt_events(injected, point, None) {
                    let _ = event_tx.send(event);
                }
                Ok(true)
            }
        }
    }
}
