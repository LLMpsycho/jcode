//! Internal second-model advisor runtime.
//!
//! Bounded post-turn review, durable session controls, and capability-based
//! enforcement. Provider context and in-flight reviews are never persisted.

mod evidence;
pub mod investigation;
mod model_selection;
mod persistence;
mod routing;

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
            "Final-review mode: give an independent evidence-referencing verdict in the summary on whether the stated objective and acceptance criteria are satisfied. Identify any missing verification explicitly and do not infer success from implementation alone. Every evidence entry must be a nonempty verbatim excerpt from the supplied objective, diff, diagnostics, verification status, todos, acceptance criteria, or tool results. Use an inconclusive verdict when acceptance cannot be verified."
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
#[serde(deny_unknown_fields)]
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
    model_override: Option<model_selection::AdvisorModelOverride>,
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
    persistence_failed: bool,
    active_review_id: u64,
    pending: Option<PendingReview>,
    enabled_override: Option<bool>,
    model_override: Option<model_selection::AdvisorModelOverride>,
    immunity_until_turn: u64,
    model_selection_id: u64,
    immunity_turns: u64,
    delivery_queue: Option<std::sync::Weak<Mutex<Vec<SoftInterruptMessage>>>>,
    capture: Option<evidence::TurnEvidence>,
    seen_diagnostics: VecDeque<u64>,
    notes: VecDeque<AdvisorNoteMetadata>,
}

#[derive(Default)]
pub struct AdvisorManager {
    sessions: Mutex<HashMap<String, AdvisorRuntime>>,
    next_review_id: AtomicU64,
    store: Option<std::path::PathBuf>,
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

    pub fn set_enabled(&self, owner_session_id: &str, enabled: bool) -> anyhow::Result<()> {
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("advisor state unavailable"))?;
            let runtime = sessions.entry(owner_session_id.to_string()).or_default();
            runtime.model_selection_id = self
                .next_review_id
                .fetch_add(1, AtomicOrdering::Relaxed)
                .saturating_add(1);
            runtime.enabled_override = Some(enabled);
            if !enabled {
                clear_queued_notes(runtime);
                runtime.capture = None;
                runtime.pending = None;
                runtime.status = AdvisorStatus::Idle;
                runtime.active_review_id = self
                    .next_review_id
                    .fetch_add(1, AtomicOrdering::Relaxed)
                    .saturating_add(1);
            }
            self.persist(owner_session_id, runtime)?;
        }
        Ok(())
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
    ) -> anyhow::Result<bool> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("advisor state unavailable"))?;
        let Some(runtime) = sessions.get_mut(owner_session_id) else {
            return Ok(false);
        };
        let Some(note) = runtime.notes.iter_mut().find(|note| note.id == id) else {
            return Ok(false);
        };
        let newly_handled = note.disposition == AdvisorNoteDisposition::Unresolved
            && disposition != AdvisorNoteDisposition::Unresolved;
        note.disposition = disposition;
        if newly_handled {
            runtime.immunity_until_turn = runtime
                .turns_observed
                .saturating_add(runtime.immunity_turns);
            runtime.pending = None;
            runtime.active_review_id = 0;
            runtime.status = AdvisorStatus::Idle;
            clear_queued_notes(runtime);
        }
        self.persist(owner_session_id, runtime)?;
        Ok(true)
    }

    pub fn blocks_tool_call(
        &self,
        owner_session_id: &str,
        tool_name: &str,
        capability: crate::tool::ToolCapability,
    ) -> Option<String> {
        if !capability.requires_advisor_clearance() {
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
        if runtime.persistence_failed {
            return Some("advisor state could not be restored or saved; inspect status or explicitly disable advisor".to_string());
        }
        runtime
            .notes
            .iter()
            .find(|note| note.blocking && note.disposition == AdvisorNoteDisposition::Unresolved)
            .map(|note| format!("advisor blocked future risky tool `{tool_name}` until note {} is acknowledged, dismissed, or advisor is disabled", note.id))
    }

    pub fn remove(&self, owner_session_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(runtime) = sessions.remove(owner_session_id) {
                clear_queued_notes(&runtime);
            }
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
        let immunity_turns = config.handled_note_immunity_turns.min(100) as u64;
        let cadence = config.review_every_n_turns.max(1) as u64;
        let max_reviews_per_session = config.max_reviews_per_session as u64;

        let input = input.bounded(config.redact);
        let mut pending = PendingReview {
            provider,
            queue,
            input,
            config,
            model_override: None,
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
            pending.model_override = runtime.model_override.clone();
            if runtime.persistence_failed {
                return false;
            }
            runtime.turns_observed = runtime.turns_observed.saturating_add(1);
            runtime.immunity_turns = immunity_turns;
            runtime.delivery_queue = Some(Arc::downgrade(&pending.queue));
            if runtime.turns_observed <= runtime.immunity_until_turn
                || (runtime.turns_observed - 1) % cadence != 0
                || runtime.cursor >= max_reviews_per_session
            {
                let _ = self.persist(&owner_session_id, runtime);
                return false;
            }
            if runtime.status == AdvisorStatus::Reviewing {
                runtime.pending = Some(pending);
                return true;
            }
            runtime.cursor = runtime.cursor.saturating_add(1);
            runtime.status = AdvisorStatus::Reviewing;
            runtime.notes_emitted = 0;
            runtime.last_error = None;
            runtime.active_review_id = review_id;
            runtime.private_context.push(pending.input.clone());
            if runtime.private_context.len() > MAX_PRIVATE_CONTEXT {
                runtime.private_context.remove(0);
            }
            if self.persist(&owner_session_id, runtime).is_err() {
                runtime.status = AdvisorStatus::Failed;
                return false;
            }
            drop(sessions);
            self.spawn_review(owner_session_id, review_id, pending);
        }
        true
    }

    fn spawn_review(
        self: &Arc<Self>,
        owner_session_id: String,
        review_id: u64,
        pending: PendingReview,
    ) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager
                .run_review(owner_session_id.clone(), review_id, pending)
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
            if runtime.persistence_failed || runtime.status == AdvisorStatus::Reviewing {
                return;
            }
            let Some(pending) = runtime.pending.take() else {
                return;
            };
            if runtime.cursor >= pending.config.max_reviews_per_session as u64 {
                return;
            }
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
            if self.persist(&owner_session_id, runtime).is_err() {
                runtime.status = AdvisorStatus::Failed;
                return;
            }
            (review_id, pending)
        };
        self.spawn_review(owner_session_id, next.0, next.1);
    }

    async fn run_review(&self, owner_session_id: String, review_id: u64, pending: PendingReview) {
        if tokio::time::timeout(
            ADVISOR_REVIEW_TIMEOUT,
            self.run_review_inner(owner_session_id.clone(), review_id, pending),
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
        pending: PendingReview,
    ) {
        let PendingReview {
            provider,
            queue,
            input,
            config,
            model_override,
        } = pending;
        if !self.sessions.lock().ok().is_some_and(|sessions| {
            sessions
                .get(&owner_session_id)
                .is_some_and(|runtime| runtime.active_review_id == review_id)
        }) {
            return;
        }
        if let Err(error) =
            routing::apply_override(provider.as_ref(), &config, model_override.as_ref())
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
            .complete_on_selected_route(&[Message::user(&prompt)], &[], &system_prompt, None)
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
        let mut completed = false;
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
                Ok(StreamEvent::MessageEnd { .. }) => {
                    completed = true;
                    break;
                }
                Ok(StreamEvent::Error { message, .. }) => {
                    self.fail(&owner_session_id, review_id, message);
                    return;
                }
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

        if !completed {
            self.fail(
                &owner_session_id,
                review_id,
                "advisor stream ended without completion".into(),
            );
            return;
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
        if config.mode == AdvisorMode::FinalReview && !evidence::grounded(&input, &note) {
            self.fail(
                &owner_session_id,
                review_id,
                "final review did not cite supplied evidence".into(),
            );
            return;
        }
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
                || (runtime.notes.len() == MAX_NOTE_METADATA
                    && runtime.notes.iter().all(|note| {
                        note.blocking && note.disposition == AdvisorNoteDisposition::Unresolved
                    }))
            {
                false
            } else {
                runtime.last_note_hash = Some(note_hash);
                runtime.notes_emitted += 1;
                let note_id = uuid::Uuid::new_v4().simple();
                runtime.notes.push_back(AdvisorNoteMetadata {
                    id: format!("adv-{note_id}"),
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
                    let removable = runtime.notes.iter().position(|note| {
                        !note.blocking || note.disposition != AdvisorNoteDisposition::Unresolved
                    });
                    if let Some(index) = removable {
                        runtime.notes.remove(index);
                    } else {
                        runtime.notes.pop_back();
                    }
                }
                self.persist(&owner_session_id, runtime).is_ok()
            }
        };

        if should_deliver
            && let Ok(sessions) = self.sessions.lock()
            && sessions.get(&owner_session_id).is_some_and(|runtime| {
                runtime.active_review_id == review_id && runtime.status == AdvisorStatus::Ready
            })
            && let Ok(mut pending) = queue.lock()
        {
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
        let error = truncate_utf8(error, 1000);
        crate::logging::warn(&format!("ADVISOR_FAILURE: {error}"));
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

fn clear_queued_notes(runtime: &AdvisorRuntime) {
    if let Some(queue) = runtime
        .delivery_queue
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        && let Ok(mut pending) = queue.lock()
    {
        pending.retain(|message| !message.content.starts_with("[ADVISOR "));
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

static ADVISOR_MANAGER: LazyLock<Arc<AdvisorManager>> = LazyLock::new(|| {
    #[cfg(test)]
    let manager = AdvisorManager::default();
    #[cfg(not(test))]
    let manager = AdvisorManager::persistent(
        crate::storage::jcode_dir()
            .unwrap_or_else(|_| crate::storage::durable_state_dir())
            .join("state/advisor"),
    );
    Arc::new(manager)
});

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

#[cfg(test)]
mod tests;
