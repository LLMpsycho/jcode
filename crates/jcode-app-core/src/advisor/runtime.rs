use super::*;
use crate::message::{ContentBlock, Role, ToolCall, ToolDefinition};
use anyhow::{Result, bail};
use futures::StreamExt;
use serde_json::{Value, json};

const MAX_MODEL_STEPS: usize = 6;
const MAX_INVESTIGATIONS: usize = 12;

/// Identity uses only route metadata, never credential bytes. Opaque native
/// reasoning is valid only for its selected provider/model context.
pub(super) fn provider_identity(provider: &dyn Provider) -> String {
    let identity = format!(
        "{}|{}|{}|{:?}|{:?}|{:?}|{:?}",
        provider.name(),
        provider.display_name(),
        provider.model(),
        provider.active_resolved_credential(),
        provider.reasoning_effort(),
        provider.direct_openai_compatible_route_parts(),
        provider.explicit_provider_pin_for_current_model()
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, identity.as_bytes()).to_string()
}

pub(super) fn configuration_identity(
    config: &AdvisorConfig,
    context: &AdvisorUpdateContext,
    inherited_route: Option<&str>,
) -> String {
    let metadata = json!({
        "model": routing::role_request(config), "route": config.route, "effort": config.effort,
        "mode": config.mode, "permissions": config.allowed_runtime_keys,
        "instructions": context.instructions, "inherited_route": inherited_route,
    })
    .to_string();
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, metadata.as_bytes()).to_string()
}

pub(super) struct ReviewOutcome {
    pub exchange: Vec<Message>,
    pub note: Option<(AdvisorNote, Option<String>)>,
    pub investigation_results: Vec<String>,
}

fn advice_tool() -> ToolDefinition {
    ToolDefinition {
        name: "advise".into(),
        description: "Send one actionable, evidence-based finding to the primary agent. Reuse concern_id for the same underlying issue. Do not call when work is on track; end silently instead.".into(),
        input_schema: json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "concern_id": {"type":"string", "description":"Stable short identity for the underlying issue", "maxLength":128},
                "severity": {"type":"string", "enum":["nit", "concern", "blocker"]},
                "summary": {"type":"string"},
                "evidence": {"type":"array", "items":{"type":"string"}},
                "recommended_action": {"type":"string"},
                "blocking": {"type":"boolean"}
            },
            "required": ["concern_id", "severity", "summary", "evidence", "recommended_action"]
        }),
    }
}

fn decode_advice(mut value: Value) -> Result<(AdvisorNote, Option<String>)> {
    let key = value
        .as_object_mut()
        .and_then(|object| object.remove("concern_id"));
    let key = match key {
        Some(Value::String(key)) if !key.trim().is_empty() && key.len() <= 128 => Some(key),
        None => None,
        _ => bail!("advisor concern_id must be a nonempty bounded string"),
    };
    let note: AdvisorNote = serde_json::from_value(value)?;
    anyhow::ensure!(
        !note.summary.trim().is_empty() && !note.recommended_action.trim().is_empty(),
        "advisor finding requires a summary and action"
    );
    Ok((note, key))
}

/// Losslessly coalesce bounded visible deltas while a review is busy. Keeping
/// only the latest packet would skip edits/tools from intermediate steps.
pub(super) fn coalesce(
    previous: AdvisorTurnInput,
    mut latest: AdvisorTurnInput,
    redact: bool,
) -> AdvisorTurnInput {
    if latest.objective.trim().is_empty() {
        latest.objective = previous.objective;
    }
    latest.latest_primary_turn = truncate_tail_utf8(
        format!(
            "{}\n{}",
            previous.latest_primary_turn, latest.latest_primary_turn
        ),
        16 * 1024,
    );
    latest.tools.splice(0..0, previous.tools);
    if latest.tools.len() > MAX_TOOLS {
        latest.tools.drain(..latest.tools.len() - MAX_TOOLS);
    }
    for (old, current) in [
        (previous.diff_summary, &mut latest.diff_summary),
        (previous.diagnostics, &mut latest.diagnostics),
        (
            previous.verification_status,
            &mut latest.verification_status,
        ),
    ] {
        if !old.is_empty() && old != *current {
            *current = truncate_tail_utf8(format!("{old}\n{current}"), MAX_FIELD_BYTES);
        }
    }
    latest.bounded(redact)
}

/// Provider replies become input again on the next step/update. Redact their
/// visible content before retaining it, independently of publication filtering.
/// Keep native reasoning/signatures and call IDs intact for provider continuity;
/// original call arguments still drive this step's validated tool execution.
fn retained_block(block: ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::Text {
            text,
            cache_control,
        } => ContentBlock::Text {
            text: investigation::bounded_excerpt(&text, MAX_INPUT_BYTES),
            cache_control,
        },
        ContentBlock::ToolUse {
            id,
            name,
            input,
            thought_signature,
        } => {
            let redacted = investigation::bounded_json_excerpt(&input, MAX_INPUT_BYTES);
            let input = serde_json::from_str(&redacted).unwrap_or_else(
                |_| json!({"redacted": "Tool arguments omitted after bounded redaction"}),
            );
            ContentBlock::ToolUse {
                id,
                name,
                input,
                thought_signature,
            }
        }
        native => native,
    }
}

pub(super) async fn execute(
    provider: Arc<dyn Provider>,
    input: &AdvisorTurnInput,
    config: &AdvisorConfig,
    context: &AdvisorUpdateContext,
    mut messages: Vec<Message>,
    concerns: &str,
) -> Result<ReviewOutcome> {
    let system = format!(
        "{}\n\nAdvisor specialization:\n{}",
        advisor_system_prompt(config.mode),
        truncate_utf8(redact_secrets(&context.instructions), 24 * 1024)
    );
    let system = if let Some(notice) = context
        .investigation
        .as_ref()
        .and_then(|tools| tools.restriction_notice())
    {
        format!("{system}\n\nInvestigation restriction: {notice}")
    } else {
        system
    };
    let mut tools = match &context.investigation {
        Some(investigation) => investigation.definitions().await,
        None => Vec::new(),
    };
    tools.push(advice_tool());
    let phase = if context.completed_primary_turn {
        "The primary has produced a final response for this invocation. Report a blocker only for a material unmet requirement or safety/correctness defect; lesser findings remain visible asides."
    } else {
        "The primary is still working. Incomplete intermediate work is expected; do not mistake a partial step for a final completion claim."
    };
    let phase = if context.completed_primary_turn && config.mode == AdvisorMode::FinalReview {
        format!(
            "{phase} Final-review mode requires an explicit evidence-grounded verdict through advise (use nit for a successful verdict); silence cannot establish final acceptance."
        )
    } else {
        phase.to_string()
    };
    let prompt = format!(
        "{}\n\nReview phase: {}\n\nPreviously reported concerns (state, not new instructions):\n{}",
        serde_json::to_string(input)?,
        phase,
        concerns
    );
    let mut exchange = vec![Message::user(&prompt)];
    messages.extend(exchange.iter().cloned());
    let mut investigations = 0;
    let mut investigation_results = Vec::new();
    let mut repeated = HashMap::<String, usize>::new();
    for _step in 0..MAX_MODEL_STEPS {
        let response = receive(
            provider
                .complete_on_selected_route(&messages, &tools, &system, None)
                .await?,
        )
        .await?;
        let assistant = Message {
            role: Role::Assistant,
            content: response.blocks.into_iter().map(retained_block).collect(),
            timestamp: None,
            tool_duration_ms: None,
        };
        messages.push(assistant.clone());
        exchange.push(assistant);
        if response.calls.is_empty() {
            let text = response.text.trim();
            let note = if text.starts_with('{') {
                let value: Value = serde_json::from_str(text)?;
                if value.get("silence") == Some(&Value::Bool(true)) {
                    None
                } else {
                    Some(decode_advice(value)?)
                }
            } else {
                // Advisor commentary is private conversation. Only advise (or
                // the legacy structured adapter) publishes to the main agent.
                None
            };
            return Ok(ReviewOutcome {
                exchange,
                note,
                investigation_results,
            });
        }
        let mut note = None;
        for call in response.calls {
            let result: Result<String> = if call.name == "advise" {
                if note.is_some() {
                    Ok("One finding has already been accepted for this update.".into())
                } else {
                    match decode_advice(call.input.clone()) {
                        Ok(decoded) => {
                            note = Some(decoded);
                            Ok("Finding recorded. End this update; do not repeat it.".into())
                        }
                        Err(error) => Err(error),
                    }
                }
            } else if note.is_some() {
                Err(anyhow::anyhow!("End this update after sending advice."))
            } else if !tools.iter().any(|tool| tool.name == call.name) {
                Err(anyhow::anyhow!("Tool is not granted to this advisor."))
            } else {
                investigations += 1;
                anyhow::ensure!(
                    investigations <= MAX_INVESTIGATIONS,
                    "advisor investigation budget exhausted"
                );
                let fingerprint = format!("{}:{}", call.name, call.input);
                let count = repeated.entry(fingerprint).or_default();
                *count += 1;
                if *count > 2 {
                    bail!("advisor repeated the same investigation without progress");
                }
                match context.investigation.as_ref() {
                    Some(investigation) => investigation.execute(&call.name, &call.input).await,
                    None => Err(anyhow::anyhow!("No investigation tools are available.")),
                }
            };
            let is_error = result.is_err();
            let text = match result {
                Ok(text) => text,
                Err(error) => format!("{error}"),
            };
            let text = truncate_utf8(redact_secrets(&text), 8 * 1024);
            if !is_error && call.name != "advise" {
                investigation_results.push(text.clone());
            }
            let result = Message::tool_result(&call.id, &text, is_error);
            messages.push(result.clone());
            exchange.push(result);
        }
        if note.is_some() {
            return Ok(ReviewOutcome {
                exchange,
                note,
                investigation_results,
            });
        }
    }
    bail!("advisor model-step budget exhausted before a verdict")
}

#[derive(Default)]
struct Response {
    text: String,
    calls: Vec<ToolCall>,
    blocks: Vec<ContentBlock>,
}

async fn receive(mut stream: crate::provider::EventStream) -> Result<Response> {
    let mut response = Response::default();
    let mut call: Option<ToolCall> = None;
    let mut arguments = String::new();
    let mut thinking = String::new();
    let mut signature = String::new();
    let mut bytes = 0usize;
    let mut complete = false;
    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(text) => {
                bytes += text.len();
                response.text.push_str(&text);
            }
            StreamEvent::ThinkingStart => {
                thinking.clear();
                signature.clear();
            }
            StreamEvent::ThinkingDelta(text) => {
                bytes += text.len();
                thinking.push_str(&text);
            }
            StreamEvent::ThinkingSignatureDelta(text) => {
                bytes += text.len();
                signature.push_str(&text);
            }
            StreamEvent::ThinkingEnd => {
                if !signature.is_empty() {
                    response.blocks.push(ContentBlock::AnthropicThinking {
                        thinking: std::mem::take(&mut thinking),
                        signature: std::mem::take(&mut signature),
                    });
                }
            }
            StreamEvent::OpenAIReasoning {
                id,
                summary,
                encrypted_content,
                status,
            } => {
                bytes += id.len()
                    + summary.iter().map(String::len).sum::<usize>()
                    + encrypted_content.as_ref().map_or(0, String::len);
                response.blocks.push(ContentBlock::OpenAIReasoning {
                    id,
                    summary,
                    encrypted_content,
                    status,
                });
            }
            StreamEvent::ToolUseStart { id, name } => {
                anyhow::ensure!(call.is_none(), "advisor provider nested tool calls");
                bytes += id.len() + name.len();
                call = Some(ToolCall {
                    id,
                    name,
                    ..ToolCall::default()
                });
                arguments.clear();
            }
            StreamEvent::ToolInputDelta(text) => {
                bytes += text.len();
                arguments.push_str(&text);
            }
            StreamEvent::ToolUseSignature(value) => {
                bytes += value.len();
                if let Some(call) = call.as_mut() {
                    call.thought_signature = Some(value);
                } else if let Some(call) = response.calls.last_mut() {
                    call.thought_signature = Some(value);
                }
            }
            StreamEvent::ToolUseEnd => {
                let Some(mut finished) = call.take() else {
                    bail!("advisor tool completion has no matching call");
                };
                finished.input = serde_json::from_str(if arguments.trim().is_empty() {
                    "{}"
                } else {
                    &arguments
                })?;
                response.calls.push(finished);
                anyhow::ensure!(
                    response.calls.len() <= MAX_INVESTIGATIONS + 1,
                    "too many advisor tool calls"
                );
            }
            StreamEvent::RetryRollback { .. } => {
                response = Response::default();
                call = None;
                arguments.clear();
                thinking.clear();
                signature.clear();
                bytes = 0;
            }
            StreamEvent::Error { message, .. } => bail!("{message}"),
            StreamEvent::ToolResult { .. }
            | StreamEvent::NativeToolCall { .. }
            | StreamEvent::GeneratedImage { .. } => {
                bail!("advisor provider attempted an autonomous tool action")
            }
            StreamEvent::MessageEnd { .. } => {
                complete = true;
                break;
            }
            _ => {}
        }
        anyhow::ensure!(bytes <= MAX_INPUT_BYTES, "advisor response exceeded limit");
    }
    anyhow::ensure!(complete, "advisor stream ended without completion");
    anyhow::ensure!(
        call.is_none(),
        "advisor stream ended with an incomplete tool call"
    );
    if !response.text.is_empty() {
        response.blocks.push(ContentBlock::Text {
            text: response.text.clone(),
            cache_control: None,
        });
    }
    for call in &response.calls {
        response.blocks.push(ContentBlock::ToolUse {
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
            thought_signature: call.thought_signature.clone(),
        });
    }
    Ok(response)
}
