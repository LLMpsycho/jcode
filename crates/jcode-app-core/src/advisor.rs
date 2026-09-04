//! Internal second-model advisor runtime.
//!
//! This first Phase 4 slice owns bounded, redacted post-turn inputs, exact-once
//! cursors, structured note parsing, deduplication, budgets, and soft-interrupt
//! delivery. Risky-tool gating and user controls remain separate follow-ups.

mod evidence;

use crate::config::{AdvisorConfig, AdvisorMode, AdvisorSeverity};
use crate::message::{Message, StreamEvent, redact_secrets};
use crate::protocol::ToolCallSummary;
use crate::provider::Provider;
use futures::StreamExt;
use jcode_agent_runtime::{SoftInterruptMessage, SoftInterruptQueue, SoftInterruptSource};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex};

const MAX_FIELD_BYTES: usize = 4 * 1024;
const MAX_INPUT_BYTES: usize = 32 * 1024;
const MAX_TOOLS: usize = 12;
const MAX_EVIDENCE: usize = 8;
const MAX_PRIVATE_CONTEXT: usize = 8;
const MAX_NOTE_METADATA: usize = 32;
const ADVISOR_REVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const ADVISOR_SYSTEM_PROMPT: &str = "You are Jcode's independent advisor. Review only the bounded evidence from the completed primary turn. You have no tools and must not request actions or additional context. Return exactly one JSON object with severity (nit, concern, or blocker), summary, evidence (an array of concise strings), recommended_action, and blocking. Do not include markdown or hidden reasoning. A blocker is reserved for unsafe actions, data-integrity risks, or an unmet hard acceptance criterion.";

fn advisor_system_prompt(mode: AdvisorMode) -> String {
    let mode_contract = match mode {
        AdvisorMode::Interactive => {
            "Interactive mode: surface one concise, actionable note only when it materially helps the user. Prefer nit or concern unless work is unsafe."
        }
        AdvisorMode::SelfdevGuardian => {
            "Self-development guardian mode: remain strictly read-only. Check evaluator integrity, promotion and release claims, scope drift, safety, rollback readiness, and benchmark validity. Cite supplied evidence for every finding and never perform or propose an unverified mutation as completed."
        }
        AdvisorMode::FinalReview => {
            "Final-review mode: give an independent evidence-referencing verdict on whether the stated objective and acceptance criteria are satisfied. Identify any missing verification explicitly and do not infer success from implementation alone."
        }
    };
    format!("{ADVISOR_SYSTEM_PROMPT}\n\n{mode_contract}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorToolInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    pub result: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorTurnInput {
    pub objective: String,
    pub latest_primary_turn: String,
    pub tools: Vec<AdvisorToolInput>,
    pub diff_summary: String,
    pub diagnostics: String,
    pub verification_status: String,
    pub outstanding_todos: String,
    pub acceptance_criteria: String,
}

impl AdvisorTurnInput {
    pub fn from_completed_turn(
        objective: &str,
        latest_primary_turn: Option<String>,
        tools: Vec<ToolCallSummary>,
        turn_succeeded: bool,
    ) -> Self {
        Self {
            objective: objective.to_string(),
            latest_primary_turn: latest_primary_turn.unwrap_or_default(),
            tools: tools
                .into_iter()
                .take(MAX_TOOLS)
                .map(|tool| AdvisorToolInput {
                    name: tool.tool_name,
                    intent: tool.intent,
                    result: tool.brief_output,
                })
                .collect(),
            verification_status: if turn_succeeded {
                "turn completed"
            } else {
                "turn failed"
            }
            .to_string(),
            ..Self::default()
        }
    }

    pub(crate) fn enrich_from_session(
        self,
        session: &crate::session::Session,
        start_message_index: usize,
    ) -> Self {
        evidence::enrich_completed_turn(self, session, start_message_index)
    }

    fn bounded(mut self, redact: bool) -> Self {
        let clean = |value: String| {
            let value = if redact {
                redact_secrets(&value)
            } else {
                value
            };
            truncate_utf8(value, MAX_FIELD_BYTES)
        };
        self.objective = clean(self.objective);
        self.latest_primary_turn = clean(self.latest_primary_turn);
        self.diff_summary = clean(self.diff_summary);
        self.diagnostics = clean(self.diagnostics);
        self.verification_status = clean(self.verification_status);
        self.outstanding_todos = clean(self.outstanding_todos);
        self.acceptance_criteria = clean(self.acceptance_criteria);
        self.tools.truncate(MAX_TOOLS);
        for tool in &mut self.tools {
            tool.name = truncate_utf8(tool.name.clone(), 256);
            tool.intent = tool.intent.take().map(clean);
            tool.result = clean(std::mem::take(&mut tool.result));
        }

        while serde_json::to_vec(&self).map_or(0, |bytes| bytes.len()) > MAX_INPUT_BYTES {
            if self.tools.pop().is_some() {
                continue;
            }

            let fields = [
                &mut self.latest_primary_turn,
                &mut self.diff_summary,
                &mut self.diagnostics,
                &mut self.verification_status,
                &mut self.outstanding_todos,
                &mut self.acceptance_criteria,
                &mut self.objective,
            ];
            let Some(field) = fields.into_iter().max_by_key(|field| field.len()) else {
                break;
            };
            if field.is_empty() {
                break;
            }
            let shorter_len = field.len().saturating_sub(1024);
            *field = truncate_utf8(std::mem::take(field), shorter_len);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorNote {
    pub severity: AdvisorSeverity,
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub recommended_action: String,
    #[serde(default)]
    pub blocking: bool,
}

impl AdvisorNote {
    fn bounded(mut self, config: &AdvisorConfig) -> Self {
        let clean = |value: String| {
            let value = if config.redact {
                redact_secrets(&value)
            } else {
                value
            };
            truncate_utf8(value, MAX_FIELD_BYTES)
        };
        self.summary = clean(self.summary);
        self.recommended_action = clean(self.recommended_action);
        self.evidence.truncate(MAX_EVIDENCE);
        for evidence in &mut self.evidence {
            *evidence = clean(std::mem::take(evidence));
        }
        self.blocking = self.severity >= config.block_on_severity;
        self
    }

    fn soft_interrupt_text(&self) -> String {
        let mut text = format!(
            "[ADVISOR {:?}] {}\nRecommended action: {}",
            self.severity, self.summary, self.recommended_action
        );
        if !self.evidence.is_empty() {
            text.push_str("\nEvidence:");
            for evidence in &self.evidence {
                text.push_str("\n- ");
                text.push_str(evidence);
            }
        }
        text
    }

    fn dedupe_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.severity.hash(&mut hasher);
        self.summary.hash(&mut hasher);
        self.recommended_action.hash(&mut hasher);
        self.evidence.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdvisorStatus {
    #[default]
    Idle,
    Reviewing,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct AdvisorRuntimeSnapshot {
    pub owner_session_id: String,
    pub turns_observed: u64,
    pub cursor: u64,
    pub status: AdvisorStatus,
    pub private_context_len: usize,
    pub notes_emitted: usize,
    pub last_error: Option<String>,
    pub unresolved_blocking_notes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorNoteDisposition {
    Unresolved,
    Acknowledged,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorNoteMetadata {
    pub id: String,
    pub severity: AdvisorSeverity,
    pub summary: String,
    pub evidence: Vec<String>,
    pub recommended_action: String,
    pub blocking: bool,
    pub disposition: AdvisorNoteDisposition,
}

struct PendingReview {
    provider: Arc<dyn Provider>,
    queue: SoftInterruptQueue,
    input: AdvisorTurnInput,
    config: AdvisorConfig,
}

#[derive(Default)]
struct AdvisorRuntime {
    turns_observed: u64,
    cursor: u64,
    status: AdvisorStatus,
    private_context: Vec<AdvisorTurnInput>,
    notes_emitted: usize,
    last_note_hash: Option<u64>,
    last_error: Option<String>,
    active_review_id: u64,
    pending: Option<PendingReview>,
    enabled_override: Option<bool>,
    notes: VecDeque<AdvisorNoteMetadata>,
}

#[derive(Default)]
pub struct AdvisorManager {
    sessions: Mutex<HashMap<String, AdvisorRuntime>>,
    next_review_id: AtomicU64,
    next_note_id: AtomicU64,
}

impl AdvisorManager {
    pub fn snapshot(&self, owner_session_id: &str) -> Option<AdvisorRuntimeSnapshot> {
        let sessions = self.sessions.lock().ok()?;
        let runtime = sessions.get(owner_session_id)?;
        Some(AdvisorRuntimeSnapshot {
            owner_session_id: owner_session_id.to_string(),
            turns_observed: runtime.turns_observed,
            cursor: runtime.cursor,
            status: runtime.status,
            private_context_len: runtime.private_context.len(),
            notes_emitted: runtime.notes_emitted,
            last_error: runtime.last_error.clone(),
            unresolved_blocking_notes: runtime
                .notes
                .iter()
                .filter(|note| {
                    note.blocking && note.disposition == AdvisorNoteDisposition::Unresolved
                })
                .count(),
        })
    }

    pub fn notes(&self, owner_session_id: &str) -> Vec<AdvisorNoteMetadata> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(owner_session_id)
                    .map(|runtime| runtime.notes.iter().cloned().collect())
            })
            .unwrap_or_default()
    }

    pub fn set_enabled(&self, owner_session_id: &str, enabled: bool) {
        if let Ok(mut sessions) = self.sessions.lock() {
            let runtime = sessions.entry(owner_session_id.to_string()).or_default();
            runtime.enabled_override = Some(enabled);
            if !enabled {
                runtime.pending = None;
                runtime.status = AdvisorStatus::Idle;
                runtime.active_review_id = self
                    .next_review_id
                    .fetch_add(1, AtomicOrdering::Relaxed)
                    .saturating_add(1);
            }
        }
    }

    pub fn is_enabled(&self, owner_session_id: &str, configured_default: bool) -> bool {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(owner_session_id)
                    .and_then(|runtime| runtime.enabled_override)
            })
            .unwrap_or(configured_default)
    }

    pub fn resolve_note(
        &self,
        owner_session_id: &str,
        id: &str,
        disposition: AdvisorNoteDisposition,
    ) -> bool {
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        let Some(note) = sessions
            .get_mut(owner_session_id)
            .and_then(|runtime| runtime.notes.iter_mut().find(|note| note.id == id))
        else {
            return false;
        };
        note.disposition = disposition;
        true
    }

    pub fn blocks_tool_call(
        &self,
        owner_session_id: &str,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<String> {
        if !is_risky_tool_call(tool_name, input) {
            return None;
        }
        let sessions = self.sessions.lock().ok()?;
        let runtime = sessions.get(owner_session_id)?;
        if !runtime
            .enabled_override
            .unwrap_or_else(|| config_for_current_session().enabled)
        {
            return None;
        }
        runtime
            .notes
            .iter()
            .find(|note| note.blocking && note.disposition == AdvisorNoteDisposition::Unresolved)
            .map(|note| format!("advisor blocked future risky tool `{tool_name}` until note {} is acknowledged, dismissed, or advisor is disabled", note.id))
    }

    pub fn remove(&self, owner_session_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(owner_session_id);
        }
    }

    pub fn schedule_turn(
        self: &Arc<Self>,
        owner_session_id: String,
        provider: Arc<dyn Provider>,
        queue: SoftInterruptQueue,
        input: AdvisorTurnInput,
        config: AdvisorConfig,
    ) -> bool {
        if config.max_notes_per_turn == 0 || config.max_reviews_per_session == 0 {
            return false;
        }
        let configured_enabled = config.enabled;
        let cadence = config.review_every_n_turns.max(1) as u64;
        let max_reviews_per_session = config.max_reviews_per_session as u64;

        let input = input.bounded(config.redact);
        let pending = PendingReview {
            provider,
            queue,
            input,
            config,
        };
        let review_id = self
            .next_review_id
            .fetch_add(1, AtomicOrdering::Relaxed)
            .saturating_add(1);
        {
            let Ok(mut sessions) = self.sessions.lock() else {
                return false;
            };
            if !sessions
                .get(&owner_session_id)
                .and_then(|runtime| runtime.enabled_override)
                .unwrap_or(configured_enabled)
            {
                return false;
            }
            let runtime = sessions.entry(owner_session_id.clone()).or_default();
            runtime.turns_observed = runtime.turns_observed.saturating_add(1);
            if (runtime.turns_observed - 1) % cadence != 0
                || runtime.cursor >= max_reviews_per_session
            {
                return false;
            }
            if runtime.status == AdvisorStatus::Reviewing {
                runtime.pending = Some(pending);
                return true;
            }
            let PendingReview {
                provider,
                queue,
                input,
                config,
            } = pending;
            runtime.cursor = runtime.cursor.saturating_add(1);
            runtime.status = AdvisorStatus::Reviewing;
            runtime.notes_emitted = 0;
            runtime.last_error = None;
            runtime.active_review_id = review_id;
            runtime.private_context.push(input.clone());
            if runtime.private_context.len() > MAX_PRIVATE_CONTEXT {
                runtime.private_context.remove(0);
            }
            drop(sessions);
            self.spawn_review(owner_session_id, review_id, provider, queue, input, config);
        }
        true
    }

    fn spawn_review(
        self: &Arc<Self>,
        owner_session_id: String,
        review_id: u64,
        provider: Arc<dyn Provider>,
        queue: SoftInterruptQueue,
        input: AdvisorTurnInput,
        config: AdvisorConfig,
    ) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager
                .run_review(
                    owner_session_id.clone(),
                    review_id,
                    provider,
                    queue,
                    input,
                    config,
                )
                .await;
            manager.start_pending(owner_session_id);
        });
    }

    fn start_pending(self: &Arc<Self>, owner_session_id: String) {
        let next = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return;
            };
            let Some(runtime) = sessions.get_mut(&owner_session_id) else {
                return;
            };
            if runtime.status == AdvisorStatus::Reviewing {
                return;
            }
            let Some(pending) = runtime.pending.take() else {
                return;
            };
            let review_id = self
                .next_review_id
                .fetch_add(1, AtomicOrdering::Relaxed)
                .saturating_add(1);
            runtime.cursor = runtime.cursor.saturating_add(1);
            runtime.status = AdvisorStatus::Reviewing;
            runtime.notes_emitted = 0;
            runtime.last_error = None;
            runtime.active_review_id = review_id;
            runtime.private_context.push(pending.input.clone());
            if runtime.private_context.len() > MAX_PRIVATE_CONTEXT {
                runtime.private_context.remove(0);
            }
            (review_id, pending)
        };
        self.spawn_review(
            owner_session_id,
            next.0,
            next.1.provider,
            next.1.queue,
            next.1.input,
            next.1.config,
        );
    }

    async fn run_review(
        &self,
        owner_session_id: String,
        review_id: u64,
        provider: Arc<dyn Provider>,
        queue: SoftInterruptQueue,
        input: AdvisorTurnInput,
        config: AdvisorConfig,
    ) {
        if tokio::time::timeout(
            ADVISOR_REVIEW_TIMEOUT,
            self.run_review_inner(
                owner_session_id.clone(),
                review_id,
                provider,
                queue,
                input,
                config,
            ),
        )
        .await
        .is_err()
        {
            self.fail(
                &owner_session_id,
                review_id,
                "advisor review timed out".to_string(),
            );
        }
    }

    async fn run_review_inner(
        &self,
        owner_session_id: String,
        review_id: u64,
        provider: Arc<dyn Provider>,
        queue: SoftInterruptQueue,
        input: AdvisorTurnInput,
        config: AdvisorConfig,
    ) {
        if let Some(model) = config.model.as_deref()
            && let Err(error) = provider.set_model(model)
        {
            self.fail(
                &owner_session_id,
                review_id,
                format!("advisor model selection failed: {error}"),
            );
            return;
        }

        let prompt = match serde_json::to_string(&input) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.fail(
                    &owner_session_id,
                    review_id,
                    format!("advisor input serialization failed: {error}"),
                );
                return;
            }
        };
        let system_prompt = advisor_system_prompt(config.mode);
        let mut stream = match provider
            .complete(&[Message::user(&prompt)], &[], &system_prompt, None)
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                self.fail(
                    &owner_session_id,
                    review_id,
                    format!("advisor request failed: {error}"),
                );
                return;
            }
        };

        let mut output = String::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta(text)) => {
                    output.push_str(&text);
                    if output.len() > MAX_INPUT_BYTES {
                        self.fail(
                            &owner_session_id,
                            review_id,
                            "advisor response exceeded limit".to_string(),
                        );
                        return;
                    }
                }
                Ok(StreamEvent::MessageEnd { .. }) => break,
                Ok(_) => {}
                Err(error) => {
                    self.fail(
                        &owner_session_id,
                        review_id,
                        format!("advisor stream failed: {error}"),
                    );
                    return;
                }
            }
        }

        let note: AdvisorNote = match serde_json::from_str(output.trim()) {
            Ok(note) => note,
            Err(error) => {
                self.fail(
                    &owner_session_id,
                    review_id,
                    format!("advisor response was not structured JSON: {error}"),
                );
                return;
            }
        };
        let note = note.bounded(&config);
        let note_hash = note.dedupe_hash();
        let should_deliver = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return;
            };
            let Some(runtime) = sessions.get_mut(&owner_session_id) else {
                return;
            };
            if runtime.active_review_id != review_id || runtime.status != AdvisorStatus::Reviewing {
                return;
            }
            runtime.status = AdvisorStatus::Ready;
            if runtime.last_note_hash == Some(note_hash)
                || runtime.notes_emitted >= config.max_notes_per_turn
            {
                false
            } else {
                runtime.last_note_hash = Some(note_hash);
                runtime.notes_emitted += 1;
                let note_id = self
                    .next_note_id
                    .fetch_add(1, AtomicOrdering::Relaxed)
                    .saturating_add(1);
                runtime.notes.push_back(AdvisorNoteMetadata {
                    id: format!("adv-{note_id:016x}"),
                    severity: note.severity,
                    summary: redact_secrets(&note.summary),
                    evidence: note
                        .evidence
                        .iter()
                        .map(|evidence| redact_secrets(evidence))
                        .collect(),
                    recommended_action: redact_secrets(&note.recommended_action),
                    blocking: note.blocking,
                    disposition: AdvisorNoteDisposition::Unresolved,
                });
                while runtime.notes.len() > MAX_NOTE_METADATA {
                    runtime.notes.pop_front();
                }
                true
            }
        };

        if should_deliver && let Ok(mut pending) = queue.lock() {
            pending.push(SoftInterruptMessage {
                content: note.soft_interrupt_text(),
                images: Vec::new(),
                urgent: note.blocking,
                source: SoftInterruptSource::System,
            });
        }
    }

    fn fail(&self, owner_session_id: &str, review_id: u64, error: String) {
        let error = redact_secrets(&error)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        crate::logging::warn(&format!(
            "ADVISOR_FAILURE session={owner_session_id}: {error}"
        ));
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(runtime) = sessions.get_mut(owner_session_id)
            && runtime.active_review_id == review_id
            && runtime.status == AdvisorStatus::Reviewing
        {
            runtime.status = AdvisorStatus::Failed;
            runtime.last_error = Some(truncate_utf8(error, 1000));
        }
    }
}

fn truncate_utf8(mut value: String, limit: usize) -> String {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

static ADVISOR_MANAGER: LazyLock<Arc<AdvisorManager>> =
    LazyLock::new(|| Arc::new(AdvisorManager::default()));

pub fn advisor_manager() -> Arc<AdvisorManager> {
    Arc::clone(&ADVISOR_MANAGER)
}

pub fn config_for_current_session() -> AdvisorConfig {
    crate::config::config().advisor.clone()
}

pub fn mode_label(mode: AdvisorMode) -> &'static str {
    match mode {
        AdvisorMode::Interactive => "interactive",
        AdvisorMode::SelfdevGuardian => "selfdev-guardian",
        AdvisorMode::FinalReview => "final-review",
    }
}

fn input_action(input: &serde_json::Value) -> Option<&str> {
    input.get("action").and_then(serde_json::Value::as_str)
}

fn action_is_not(input: &serde_json::Value, safe: &[&str]) -> bool {
    input_action(input).is_none_or(|action| !safe.contains(&action))
}

/// Classify only calls that can write durable state, execute code/processes, or
/// perform an externally visible action. Composite tools such as `batch` are
/// intentionally not listed: every nested call re-enters `Registry::execute`
/// and is classified using its resolved tool name and actual input.
pub fn is_risky_tool_call(name: &str, input: &serde_json::Value) -> bool {
    match name {
        "bash"
        | "write"
        | "edit"
        | "multiedit"
        | "patch"
        | "apply_patch"
        | "anchored_edit"
        | "open"
        | "maintainer_feedback"
        | "todo" => true,
        "browser" => action_is_not(
            input,
            &[
                "status",
                "list_tabs",
                "get_active_tab",
                "list_frames",
                "snapshot",
                "get_content",
                "interactables",
                "wait",
                "screenshot",
            ],
        ),
        "macos_computer_use" => action_is_not(
            input,
            &[
                "screenshot",
                "ocr",
                "ui",
                "find_element",
                "get_value",
                "check_permissions",
                "discover",
            ],
        ),
        "gmail" => action_is_not(
            input,
            &["search", "read", "list", "threads", "thread", "labels"],
        ),
        "schedule" => action_is_not(input, &["list"]),
        "selfdev" => action_is_not(
            input,
            &["status", "find-config", "socket-info", "socket-help"],
        ),
        "bg" => action_is_not(
            input,
            &[
                "list",
                "status",
                "output",
                "tail",
                "watch",
                "delivery",
                "subscribe",
                "wait",
            ],
        ),
        "side_panel" => action_is_not(input, &["status", "load"]),
        "memory" => action_is_not(input, &["recall", "search", "list", "related"]),
        "initiative" => action_is_not(input, &["list", "show"]),
        "skill_manage" => action_is_not(input, &["list", "read"]),
        "integration_tools" => action_is_not(input, &["search", "details"]),
        "lsp" => input.get("apply").and_then(serde_json::Value::as_bool) == Some(true),
        "dap" => action_is_not(
            input,
            &[
                "sessions",
                "threads",
                "stack_trace",
                "scopes",
                "variables",
                "output",
                "step_in_targets",
            ],
        ),
        "swarm" => action_is_not(
            input,
            &[
                "read",
                "list",
                "list_channels",
                "channel_members",
                "status",
                "summary",
                "read_context",
                "plan_status",
                "task_graph",
                "list_models",
            ],
        ),
        "mcp" | "mcp_call" => true,
        _ if name.starts_with("mcp__") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AdvisorProvider {
        calls: Arc<AtomicUsize>,
        response: String,
    }

    struct ModeCaptureProvider {
        systems: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Provider for AdvisorProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            tools: &[crate::message::ToolDefinition],
            _system: &str,
            _resume_session_id: Option<&str>,
        ) -> Result<crate::provider::EventStream> {
            assert!(tools.is_empty());
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::TextDelta(self.response.clone())),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: Some("end_turn".to_string()),
                }),
            ])))
        }

        fn name(&self) -> &str {
            "advisor-test"
        }

        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self {
                calls: Arc::clone(&self.calls),
                response: self.response.clone(),
            })
        }
    }

    #[async_trait]
    impl Provider for ModeCaptureProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            tools: &[crate::message::ToolDefinition],
            system: &str,
            _resume_session_id: Option<&str>,
        ) -> Result<crate::provider::EventStream> {
            assert!(tools.is_empty());
            self.systems
                .lock()
                .expect("systems")
                .push(system.to_string());
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::TextDelta(
                    r#"{"severity":"nit","summary":"ok","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
                )),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: Some("end_turn".to_string()),
                }),
            ])))
        }

        fn name(&self) -> &str {
            "advisor-mode-capture"
        }

        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self {
                systems: Arc::clone(&self.systems),
            })
        }
    }

    struct FailingAdvisorProvider;

    struct BlockingAdvisorProvider {
        release: Arc<tokio::sync::Notify>,
        response: String,
    }

    #[async_trait]
    impl Provider for BlockingAdvisorProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[crate::message::ToolDefinition],
            _system: &str,
            _resume_session_id: Option<&str>,
        ) -> Result<crate::provider::EventStream> {
            self.release.notified().await;
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::TextDelta(self.response.clone())),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: Some("end_turn".to_string()),
                }),
            ])))
        }

        fn name(&self) -> &str {
            "advisor-blocking-test"
        }

        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self {
                release: Arc::clone(&self.release),
                response: self.response.clone(),
            })
        }
    }

    #[async_trait]
    impl Provider for FailingAdvisorProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[crate::message::ToolDefinition],
            _system: &str,
            _resume_session_id: Option<&str>,
        ) -> Result<crate::provider::EventStream> {
            anyhow::bail!(
                "request rejected\nAuthorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789\nOPENAI_API_KEY=sk-test-openai-example"
            )
        }

        fn name(&self) -> &str {
            "advisor-failure-test"
        }

        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Self)
        }
    }

    fn enabled_config() -> AdvisorConfig {
        AdvisorConfig {
            enabled: true,
            ..AdvisorConfig::default()
        }
    }

    #[tokio::test]
    async fn disabled_advisor_has_no_runtime_or_provider_cost() {
        let manager = Arc::new(AdvisorManager::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(Vec::new()));
        let scheduled = manager.schedule_turn(
            "disabled".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::clone(&calls),
                response: String::new(),
            }),
            queue,
            AdvisorTurnInput::default(),
            AdvisorConfig::default(),
        );
        assert!(!scheduled);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(manager.snapshot("disabled").is_none());
    }

    #[tokio::test]
    async fn review_advances_once_redacts_and_delivers_structured_note() {
        let manager = Arc::new(AdvisorManager::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(Vec::new()));
        let scheduled = manager.schedule_turn(
            "session".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::clone(&calls),
                response: r#"{"severity":"concern","summary":"Acceptance drift","evidence":["OPENAI_API_KEY=sk-test-openai-example"],"recommended_action":"Run the public flow","blocking":true}"#.to_string(),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput {
                objective: "OPENAI_API_KEY=sk-test-openai-example".to_string(),
                ..AdvisorTurnInput::default()
            },
            enabled_config(),
        );
        assert!(scheduled);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if manager
                    .snapshot("session")
                    .is_some_and(|s| s.status == AdvisorStatus::Ready)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("advisor should finish");

        let snapshot = manager.snapshot("session").expect("runtime");
        assert_eq!(snapshot.cursor, 1);
        assert_eq!(snapshot.private_context_len, 1);
        assert_eq!(snapshot.notes_emitted, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pending = queue.lock().expect("queue");
        assert_eq!(pending.len(), 1);
        assert!(pending[0].content.contains("Acceptance drift"));
        assert!(pending[0].content.contains("[REDACTED_SECRET]"));
        assert!(
            !pending[0].urgent,
            "concern is below the default blocker threshold"
        );
    }

    async fn wait_for_status(manager: &AdvisorManager, session: &str, expected: AdvisorStatus) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if manager
                    .snapshot(session)
                    .is_some_and(|snapshot| snapshot.status == expected)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("advisor should reach expected status");
    }

    #[tokio::test]
    async fn duplicate_notes_are_delivered_only_once_across_turns() {
        let manager = Arc::new(AdvisorManager::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(Vec::new()));
        let provider: Arc<dyn Provider> = Arc::new(AdvisorProvider {
            calls: Arc::clone(&calls),
            response: r#"{"severity":"nit","summary":"Same note","evidence":[],"recommended_action":"Keep going","blocking":false}"#.to_string(),
        });

        for expected_cursor in 1..=2 {
            assert!(manager.schedule_turn(
                "dedupe".to_string(),
                Arc::clone(&provider),
                Arc::clone(&queue),
                AdvisorTurnInput::default(),
                enabled_config(),
            ));
            wait_for_status(&manager, "dedupe", AdvisorStatus::Ready).await;
            assert_eq!(
                manager.snapshot("dedupe").expect("runtime").cursor,
                expected_cursor
            );
        }

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(queue.lock().expect("queue").len(), 1);
    }

    #[tokio::test]
    async fn review_finishing_while_another_turn_completes_runs_latest_pending_review() {
        let manager = Arc::new(AdvisorManager::default());
        let queue = Arc::new(Mutex::new(Vec::new()));
        let release = Arc::new(tokio::sync::Notify::new());
        assert!(manager.schedule_turn(
            "coalesce".to_string(),
            Arc::new(BlockingAdvisorProvider {
                release: Arc::clone(&release),
                response: r#"{"severity":"nit","summary":"first","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput {
                objective: "first".to_string(),
                ..AdvisorTurnInput::default()
            },
            enabled_config(),
        ));
        assert!(manager.schedule_turn(
            "coalesce".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: r#"{"severity":"concern","summary":"latest","evidence":[],"recommended_action":"review latest","blocking":false}"#.to_string(),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput {
                objective: "latest".to_string(),
                ..AdvisorTurnInput::default()
            },
            enabled_config(),
        ));

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if manager.snapshot("coalesce").is_some_and(|snapshot| {
                    snapshot.cursor == 2 && snapshot.status == AdvisorStatus::Ready
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pending review should run after the active review");

        let pending = queue.lock().expect("queue");
        assert_eq!(pending.len(), 2);
        assert!(pending[1].content.contains("latest"));
    }

    #[tokio::test]
    async fn malformed_advisor_response_fails_without_interrupting_primary_session() {
        let manager = Arc::new(AdvisorManager::default());
        let queue = Arc::new(Mutex::new(Vec::new()));
        assert!(manager.schedule_turn(
            "failure".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: "not-json".to_string(),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));

        wait_for_status(&manager, "failure", AdvisorStatus::Failed).await;
        let snapshot = manager.snapshot("failure").expect("runtime");
        assert_eq!(snapshot.cursor, 1);
        assert!(snapshot.last_error.is_some());
        assert!(queue.lock().expect("queue").is_empty());
    }

    #[tokio::test]
    async fn provider_failure_is_redacted_before_runtime_storage() {
        let manager = Arc::new(AdvisorManager::default());
        assert!(manager.schedule_turn(
            "redacted-failure".to_string(),
            Arc::new(FailingAdvisorProvider),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));

        wait_for_status(&manager, "redacted-failure", AdvisorStatus::Failed).await;
        let error = manager
            .snapshot("redacted-failure")
            .and_then(|snapshot| snapshot.last_error)
            .expect("failure should be stored");
        assert!(error.contains("[REDACTED_SECRET]"));
        assert!(!error.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(!error.contains("sk-test-openai-example"));
        assert!(!error.contains('\n'));
    }

    #[tokio::test]
    async fn stale_review_completion_cannot_publish_into_recreated_session() {
        let manager = AdvisorManager::default();
        manager.sessions.lock().expect("sessions").insert(
            "recreated".to_string(),
            AdvisorRuntime {
                cursor: 1,
                status: AdvisorStatus::Reviewing,
                active_review_id: 2,
                ..AdvisorRuntime::default()
            },
        );
        let queue = Arc::new(Mutex::new(Vec::new()));

        manager
            .run_review(
                "recreated".to_string(),
                1,
                Arc::new(AdvisorProvider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    response: r#"{"severity":"blocker","summary":"Stale","evidence":[],"recommended_action":"Do not publish","blocking":true}"#.to_string(),
                }),
                Arc::clone(&queue),
                AdvisorTurnInput::default(),
                enabled_config(),
            )
            .await;

        let snapshot = manager.snapshot("recreated").expect("runtime");
        assert_eq!(snapshot.status, AdvisorStatus::Reviewing);
        assert_eq!(snapshot.notes_emitted, 0);
        assert!(queue.lock().expect("queue").is_empty());
    }

    #[tokio::test]
    async fn oversized_response_fails_with_bounded_error_and_no_note() {
        let manager = Arc::new(AdvisorManager::default());
        let queue = Arc::new(Mutex::new(Vec::new()));
        assert!(manager.schedule_turn(
            "oversized".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: "x".repeat(MAX_INPUT_BYTES + 1),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));

        wait_for_status(&manager, "oversized", AdvisorStatus::Failed).await;
        let snapshot = manager.snapshot("oversized").expect("runtime");
        assert!(snapshot.last_error.expect("bounded error").len() <= 1000);
        assert!(queue.lock().expect("queue").is_empty());
    }

    #[tokio::test]
    async fn private_context_retains_only_the_bounded_recent_window() {
        let manager = Arc::new(AdvisorManager::default());
        let queue = Arc::new(Mutex::new(Vec::new()));

        for index in 0..MAX_PRIVATE_CONTEXT + 2 {
            assert!(manager.schedule_turn(
                "context".to_string(),
                Arc::new(AdvisorProvider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    response: format!(
                        r#"{{"severity":"nit","summary":"note-{index}","evidence":[],"recommended_action":"continue","blocking":false}}"#
                    ),
                }),
                Arc::clone(&queue),
                AdvisorTurnInput {
                    objective: format!("turn-{index}"),
                    ..AdvisorTurnInput::default()
                },
                enabled_config(),
            ));
            wait_for_status(&manager, "context", AdvisorStatus::Ready).await;
        }

        let snapshot = manager.snapshot("context").expect("runtime");
        assert_eq!(snapshot.cursor as usize, MAX_PRIVATE_CONTEXT + 2);
        assert_eq!(snapshot.private_context_len, MAX_PRIVATE_CONTEXT);
    }

    #[tokio::test]
    async fn review_cadence_and_session_budget_bound_provider_calls() {
        let manager = Arc::new(AdvisorManager::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn Provider> = Arc::new(AdvisorProvider {
            calls: Arc::clone(&calls),
            response: r#"{"severity":"nit","summary":"ok","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
        });
        let config = AdvisorConfig {
            enabled: true,
            review_every_n_turns: 2,
            max_reviews_per_session: 2,
            ..AdvisorConfig::default()
        };

        assert!(manager.schedule_turn(
            "budgeted".to_string(),
            Arc::clone(&provider),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config.clone(),
        ));
        wait_for_status(&manager, "budgeted", AdvisorStatus::Ready).await;
        assert!(!manager.schedule_turn(
            "budgeted".to_string(),
            Arc::clone(&provider),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config.clone(),
        ));
        assert!(manager.schedule_turn(
            "budgeted".to_string(),
            Arc::clone(&provider),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config.clone(),
        ));
        wait_for_status(&manager, "budgeted", AdvisorStatus::Ready).await;
        assert!(!manager.schedule_turn(
            "budgeted".to_string(),
            Arc::clone(&provider),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config.clone(),
        ));
        assert!(!manager.schedule_turn(
            "budgeted".to_string(),
            provider,
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config,
        ));

        let snapshot = manager.snapshot("budgeted").expect("runtime");
        assert_eq!(snapshot.turns_observed, 5);
        assert_eq!(snapshot.cursor, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn zero_session_budget_has_no_runtime_or_provider_cost() {
        let manager = Arc::new(AdvisorManager::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let config = AdvisorConfig {
            enabled: true,
            max_reviews_per_session: 0,
            ..AdvisorConfig::default()
        };
        assert!(!manager.schedule_turn(
            "zero-budget".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::clone(&calls),
                response: String::new(),
            }),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            config,
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(manager.snapshot("zero-budget").is_none());
    }

    #[test]
    fn advisor_modes_have_distinct_toolless_evidence_contracts() {
        let interactive = advisor_system_prompt(AdvisorMode::Interactive);
        let guardian = advisor_system_prompt(AdvisorMode::SelfdevGuardian);
        let final_review = advisor_system_prompt(AdvisorMode::FinalReview);

        for prompt in [&interactive, &guardian, &final_review] {
            assert!(prompt.contains("You have no tools"));
            assert!(prompt.contains("Return exactly one JSON object"));
            assert!(prompt.contains("Do not include markdown or hidden reasoning"));
        }
        assert!(interactive.contains("materially helps the user"));
        assert!(guardian.contains("strictly read-only"));
        assert!(guardian.contains("benchmark validity"));
        assert!(final_review.contains("independent evidence-referencing verdict"));
        assert!(final_review.contains("do not infer success from implementation alone"));
        assert_ne!(interactive, guardian);
        assert_ne!(guardian, final_review);
    }

    #[tokio::test]
    async fn configured_mode_contract_reaches_the_forked_provider() {
        let manager = Arc::new(AdvisorManager::default());
        let systems = Arc::new(Mutex::new(Vec::new()));
        for (index, mode) in [
            AdvisorMode::Interactive,
            AdvisorMode::SelfdevGuardian,
            AdvisorMode::FinalReview,
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = format!("mode-{index}");
            assert!(manager.schedule_turn(
                session_id.clone(),
                Arc::new(ModeCaptureProvider {
                    systems: Arc::clone(&systems),
                }),
                Arc::new(Mutex::new(Vec::new())),
                AdvisorTurnInput::default(),
                AdvisorConfig {
                    enabled: true,
                    mode,
                    ..AdvisorConfig::default()
                },
            ));
            wait_for_status(&manager, &session_id, AdvisorStatus::Ready).await;
        }

        let systems = systems.lock().expect("systems");
        assert_eq!(systems.len(), 3);
        assert!(systems[0].contains("Interactive mode"));
        assert!(systems[1].contains("Self-development guardian mode"));
        assert!(systems[2].contains("Final-review mode"));
    }

    #[test]
    fn turn_input_is_bounded_and_redacted() {
        let input = AdvisorTurnInput {
            objective: format!(
                "OPENAI_API_KEY=sk-test-openai-example {}",
                "x".repeat(MAX_FIELD_BYTES * 2)
            ),
            tools: (0..MAX_TOOLS + 5)
                .map(|index| AdvisorToolInput {
                    name: format!("tool-{index}"),
                    intent: Some(format!(
                        "OPENAI_API_KEY=sk-test-openai-example {}",
                        "i".repeat(MAX_FIELD_BYTES * 2)
                    )),
                    result: "y".repeat(MAX_FIELD_BYTES * 2),
                })
                .collect(),
            ..AdvisorTurnInput::default()
        }
        .bounded(true);
        let encoded = serde_json::to_vec(&input).expect("serialize");
        assert!(encoded.len() <= MAX_INPUT_BYTES);
        assert!(input.objective.contains("[REDACTED_SECRET]"));
        assert!(input.tools.len() <= MAX_TOOLS);
        assert!(
            input.tools[0]
                .intent
                .as_deref()
                .is_some_and(|intent| intent.contains("[REDACTED_SECRET]"))
        );
    }

    #[test]
    fn escaped_fields_cannot_exceed_total_input_budget() {
        let escaped = "\0".repeat(MAX_FIELD_BYTES);
        let input = AdvisorTurnInput {
            objective: escaped.clone(),
            latest_primary_turn: escaped.clone(),
            diff_summary: escaped.clone(),
            diagnostics: escaped.clone(),
            verification_status: escaped.clone(),
            outstanding_todos: escaped.clone(),
            acceptance_criteria: escaped,
            ..AdvisorTurnInput::default()
        }
        .bounded(false);

        assert!(serde_json::to_vec(&input).expect("serialize").len() <= MAX_INPUT_BYTES);
    }

    #[tokio::test]
    async fn unresolved_blocker_gates_only_future_risky_tools_until_handled_or_disabled() {
        let manager = Arc::new(AdvisorManager::default());
        manager.set_enabled("gating", true);
        assert!(manager.schedule_turn(
            "gating".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: r#"{"severity":"blocker","summary":"unsafe publication","evidence":["bounded"],"recommended_action":"verify first","blocking":false}"#.to_string(),
            }),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));
        wait_for_status(&manager, "gating", AdvisorStatus::Ready).await;

        let notes = manager.notes("gating");
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].blocking,
            "severity, not model boolean, controls gating"
        );
        let empty = serde_json::json!({});
        assert!(manager.blocks_tool_call("gating", "read", &empty).is_none());
        assert!(manager.blocks_tool_call("gating", "bash", &empty).is_some());

        assert!(
            manager.resolve_note("gating", &notes[0].id, AdvisorNoteDisposition::Acknowledged,)
        );
        assert!(manager.blocks_tool_call("gating", "bash", &empty).is_none());

        manager.resolve_note("gating", &notes[0].id, AdvisorNoteDisposition::Unresolved);
        manager.set_enabled("gating", false);
        assert!(
            manager
                .blocks_tool_call("gating", "write", &empty)
                .is_none()
        );
    }

    #[test]
    fn risky_tool_classifier_distinguishes_read_only_and_mutating_actions() {
        let cases = [
            ("browser", serde_json::json!({"action":"snapshot"}), false),
            ("browser", serde_json::json!({"action":"click"}), true),
            ("gmail", serde_json::json!({"action":"read"}), false),
            ("gmail", serde_json::json!({"action":"send"}), true),
            ("schedule", serde_json::json!({"action":"list"}), false),
            ("schedule", serde_json::json!({"action":"cancel"}), true),
            ("memory", serde_json::json!({"action":"recall"}), false),
            ("memory", serde_json::json!({"action":"remember"}), true),
            (
                "lsp",
                serde_json::json!({"action":"rename","apply":false}),
                false,
            ),
            (
                "lsp",
                serde_json::json!({"action":"rename","apply":true}),
                true,
            ),
            ("dap", serde_json::json!({"action":"threads"}), false),
            ("dap", serde_json::json!({"action":"evaluate"}), true),
            ("swarm", serde_json::json!({"action":"list"}), false),
            ("swarm", serde_json::json!({"action":"spawn"}), true),
            ("mcp__server__read", serde_json::json!({}), true),
        ];

        for (name, input, expected) in cases {
            assert_eq!(
                is_risky_tool_call(name, &input),
                expected,
                "unexpected risk classification for {name} with {input}"
            );
        }
    }

    #[tokio::test]
    async fn enable_override_activates_globally_disabled_advisor() {
        let manager = Arc::new(AdvisorManager::default());
        manager.set_enabled("enabled-override", true);
        assert!(manager.is_enabled("enabled-override", false));

        assert!(manager.schedule_turn(
            "enabled-override".to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: r#"{"severity":"nit","summary":"ok","evidence":[],"recommended_action":"continue","blocking":false}"#.to_string(),
            }),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput::default(),
            AdvisorConfig::default(),
        ));
        wait_for_status(&manager, "enabled-override", AdvisorStatus::Ready).await;
    }

    #[tokio::test]
    async fn disabling_fences_an_in_flight_review_and_discards_its_note() {
        let manager = Arc::new(AdvisorManager::default());
        let release = Arc::new(tokio::sync::Notify::new());
        let queue = Arc::new(Mutex::new(Vec::new()));
        assert!(manager.schedule_turn(
            "disable-in-flight".to_string(),
            Arc::new(BlockingAdvisorProvider {
                release: Arc::clone(&release),
                response: r#"{"severity":"blocker","summary":"late","evidence":[],"recommended_action":"stop","blocking":true}"#.to_string(),
            }),
            Arc::clone(&queue),
            AdvisorTurnInput::default(),
            enabled_config(),
        ));
        wait_for_status(&manager, "disable-in-flight", AdvisorStatus::Reviewing).await;

        manager.set_enabled("disable-in-flight", false);
        release.notify_waiters();
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        let snapshot = manager.snapshot("disable-in-flight").expect("snapshot");
        assert_eq!(snapshot.status, AdvisorStatus::Idle);
        assert!(!manager.is_enabled("disable-in-flight", true));
        assert!(manager.notes("disable-in-flight").is_empty());
        assert!(queue.lock().expect("queue").is_empty());
    }
}
