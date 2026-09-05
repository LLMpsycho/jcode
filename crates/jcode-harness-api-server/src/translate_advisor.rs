//! Advisor replies are independent of the primary model's stream lifecycle.

use super::{BridgeState, Outbound, SimpleKind};
use jcode_harness_api::{AdvisorControlResult, AdvisorRequest, ApiEvent, ErrorCode, ServerFrame};
use serde_json::{Value, json};

impl BridgeState {
    pub(super) fn advisor_request_to_legacy(
        &mut self,
        frame: &Value,
        api_id: u64,
    ) -> Vec<Outbound> {
        let Some(session_id) = frame["session_id"].as_str().filter(|id| !id.is_empty()) else {
            return Self::error_reply(
                api_id,
                ErrorCode::InvalidRequest,
                "advisor controls require a session_id",
            );
        };
        let Ok(request) = serde_json::from_value::<AdvisorRequest>(frame["request"].clone()) else {
            return Self::error_reply(api_id, ErrorCode::InvalidRequest, "invalid advisor control");
        };
        let control = match &request {
            AdvisorRequest::ForAdvisor { request, .. } => request.as_ref(),
            request => request,
        };
        if matches!(control, AdvisorRequest::ForAdvisor { .. }) {
            return Self::error_reply(
                api_id,
                ErrorCode::InvalidRequest,
                "advisor target wrappers may not be nested",
            );
        }
        let selection = match control {
            AdvisorRequest::SelectModel { selection, .. } => Some(selection),
            AdvisorRequest::ModelOptions { selection } => selection.as_ref(),
            _ => None,
        };
        // Keep malformed enum tags away from the legacy decoder: it closes
        // the entire connection on an unparseable request. The identities
        // here are wire tags, never auth/provider-name classification.
        if selection.is_some_and(|selection| !known_runtime_kind(&selection.runtime_key.kind)) {
            return Self::error_reply(
                api_id,
                ErrorCode::InvalidRequest,
                "unsupported advisor runtime identity; use a selection from model_options",
            );
        }
        let id = self.legacy_id();
        self.pending_simple.push((
            id,
            api_id,
            SimpleKind::Advisor {
                session_id: session_id.to_string(),
            },
        ));
        vec![Outbound::Legacy(json!({
            "type": "advisor", "id": id, "request": request,
        }))]
    }

    pub(super) fn advisor_result_to_api(&mut self, frame: &Value) -> Vec<ServerFrame> {
        let Some(id) = frame["id"].as_u64() else {
            return Vec::new();
        };
        let Some(index) = self.pending_simple.iter().position(|(legacy_id, _, kind)| {
            *legacy_id == id && matches!(kind, SimpleKind::Advisor { .. })
        }) else {
            return Vec::new();
        };
        let (_, api_id, SimpleKind::Advisor { session_id }) = self.pending_simple.remove(index)
        else {
            return Vec::new();
        };
        let event = match serde_json::from_value::<AdvisorControlResult>(frame["result"].clone()) {
            Ok(result) => ApiEvent::AdvisorResult { session_id, result },
            Err(_) => ApiEvent::Error {
                code: ErrorCode::Internal,
                message: "daemon returned an invalid advisor result".into(),
            },
        };
        vec![ServerFrame::reply(api_id, event)]
    }
}

fn known_runtime_kind(kind: &str) -> bool {
    matches!(
        kind,
        "jcode-subscription"
            | "claude-o-auth"
            | "anthropic-api-key"
            | "open-a-i-o-auth"
            | "open-a-i-api-key"
            | "open-router"
            | "open-ai-compatible"
            | "copilot"
            | "gemini"
            | "cursor"
            | "bedrock"
            | "antigravity"
            | "code-assist-o-auth"
            | "remote-catalog"
            | "current"
    )
}

#[cfg(test)]
#[path = "translate_advisor_tests.rs"]
mod tests;
