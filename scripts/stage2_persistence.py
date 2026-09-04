from pathlib import Path

ROOT = Path.cwd()


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"expected text not found in {path}: {old[:180]!r}")
    target.write_text(text.replace(old, new, 1))


def replace_between(path: str, start: str, end: str, replacement: str) -> None:
    target = ROOT / path
    text = target.read_text()
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"start marker not found in {path}: {start!r}")
    end_index = text.find(end, start_index)
    if end_index < 0:
        raise SystemExit(f"end marker not found in {path}: {end!r}")
    target.write_text(text[:start_index] + replacement + text[end_index:])


persistence = r'''//! Durable, minimal advisor state.
//!
//! The persisted contract deliberately excludes primary-turn evidence, private
//! advisor prompts, provider output, pending work, and provider credentials.
//! Only redacted note metadata, dispositions, enablement, cursors, and stable
//! dedupe bookkeeping survive a process restart.

use super::{
    AdvisorNote, AdvisorNoteDisposition, AdvisorNoteMetadata, AdvisorRuntime, AdvisorStatus,
    MAX_EVIDENCE, MAX_FIELD_BYTES, MAX_NOTE_METADATA, redact_secrets, truncate_utf8,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const STATE_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 256 * 1024;
const MAX_NOTE_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedAdvisorState {
    version: u32,
    session_fingerprint: String,
    turns_observed: u64,
    cursor: u64,
    #[serde(default)]
    last_note_fingerprint: Option<String>,
    #[serde(default)]
    next_note_ordinal: u64,
    #[serde(default)]
    enabled_override: Option<bool>,
    #[serde(default)]
    notes: Vec<AdvisorNoteMetadata>,
}

pub(super) fn capture(
    owner_session_id: &str,
    runtime: &AdvisorRuntime,
) -> PersistedAdvisorState {
    PersistedAdvisorState {
        version: STATE_VERSION,
        session_fingerprint: session_fingerprint(owner_session_id),
        turns_observed: runtime.turns_observed,
        cursor: runtime.cursor,
        last_note_fingerprint: runtime
            .last_note_fingerprint
            .clone()
            .filter(|fingerprint| is_sha256_hex(fingerprint)),
        next_note_ordinal: runtime.next_note_ordinal,
        enabled_override: runtime.enabled_override,
        notes: runtime
            .notes
            .iter()
            .rev()
            .take(MAX_NOTE_METADATA)
            .cloned()
            .map(sanitize_note)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

pub(super) fn load(root: &Path, owner_session_id: &str) -> Result<Option<AdvisorRuntime>> {
    let path = state_path(root, owner_session_id);
    if !path.exists() {
        return Ok(None);
    }
    enforce_size_limit(&path)?;
    let backup = path.with_extension("bak");
    if backup.exists() {
        enforce_size_limit(&backup)?;
    }

    let state: PersistedAdvisorState = crate::storage::read_json(&path)
        .with_context(|| format!("read advisor state at {}", path.display()))?;
    if state.version != STATE_VERSION {
        anyhow::bail!(
            "unsupported advisor state version {} at {}",
            state.version,
            path.display()
        );
    }
    let expected_fingerprint = session_fingerprint(owner_session_id);
    if state.session_fingerprint != expected_fingerprint {
        anyhow::bail!("advisor state session fingerprint mismatch");
    }

    let prefix = format!("adv-{}-", session_token(owner_session_id));
    let mut next_note_ordinal = state.next_note_ordinal;
    let mut notes = VecDeque::new();
    let retained: Vec<_> = state
        .notes
        .into_iter()
        .rev()
        .take(MAX_NOTE_METADATA)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    for note in retained {
        let mut note = sanitize_note(note);
        let ordinal = parse_note_ordinal(&prefix, &note.id).unwrap_or_else(|| {
            next_note_ordinal = next_note_ordinal.saturating_add(1);
            next_note_ordinal
        });
        next_note_ordinal = next_note_ordinal.max(ordinal);
        note.id = note_id(owner_session_id, ordinal);
        notes.push_back(note);
    }

    Ok(Some(AdvisorRuntime {
        turns_observed: state.turns_observed,
        cursor: state.cursor,
        status: if notes.is_empty() {
            AdvisorStatus::Idle
        } else {
            AdvisorStatus::Ready
        },
        private_context: Vec::new(),
        notes_emitted: 0,
        last_note_fingerprint: state
            .last_note_fingerprint
            .filter(|fingerprint| is_sha256_hex(fingerprint)),
        last_error: None,
        active_review_id: 0,
        pending: None,
        enabled_override: state.enabled_override,
        notes,
        next_note_ordinal,
    }))
}

pub(super) fn save(
    root: &Path,
    owner_session_id: &str,
    state: &PersistedAdvisorState,
) -> Result<()> {
    if state.session_fingerprint != session_fingerprint(owner_session_id) {
        anyhow::bail!("refusing to write advisor state with a mismatched fingerprint");
    }
    let path = state_path(root, owner_session_id);
    crate::storage::write_json_secret(&path, state)
        .with_context(|| format!("write advisor state at {}", path.display()))
}

pub(super) fn delete(root: &Path, owner_session_id: &str) -> Result<()> {
    let path = state_path(root, owner_session_id);
    for candidate in [path.clone(), path.with_extension("bak")] {
        match std::fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("remove advisor state at {}", candidate.display()));
            }
        }
    }
    Ok(())
}

pub(super) fn note_fingerprint(note: &AdvisorNote) -> String {
    let encoded = serde_json::to_vec(note).unwrap_or_default();
    sha256_hex(&encoded)
}

pub(super) fn note_id(owner_session_id: &str, ordinal: u64) -> String {
    format!("adv-{}-{ordinal:016x}", session_token(owner_session_id))
}

pub(super) fn state_path(root: &Path, owner_session_id: &str) -> PathBuf {
    root.join(format!("{}.json", session_fingerprint(owner_session_id)))
}

fn session_token(owner_session_id: &str) -> String {
    session_fingerprint(owner_session_id)[..12].to_string()
}

fn session_fingerprint(owner_session_id: &str) -> String {
    sha256_hex(owner_session_id.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_note_ordinal(prefix: &str, id: &str) -> Option<u64> {
    let ordinal = id.strip_prefix(prefix)?;
    if ordinal.len() != 16 {
        return None;
    }
    u64::from_str_radix(ordinal, 16).ok()
}

fn sanitize_note(mut note: AdvisorNoteMetadata) -> AdvisorNoteMetadata {
    note.id = truncate_utf8(note.id, MAX_NOTE_ID_BYTES);
    note.summary = sanitize_text(note.summary);
    note.recommended_action = sanitize_text(note.recommended_action);
    note.evidence.truncate(MAX_EVIDENCE);
    for evidence in &mut note.evidence {
        *evidence = sanitize_text(std::mem::take(evidence));
    }
    if note.disposition != AdvisorNoteDisposition::Unresolved {
        note.blocking = note.blocking;
    }
    note
}

fn sanitize_text(value: String) -> String {
    truncate_utf8(redact_secrets(&value), MAX_FIELD_BYTES)
}

fn enforce_size_limit(path: &Path) -> Result<()> {
    let bytes = std::fs::metadata(path)?.len();
    if bytes > MAX_STATE_BYTES {
        anyhow::bail!(
            "advisor state at {} exceeds {} bytes",
            path.display(),
            MAX_STATE_BYTES
        );
    }
    Ok(())
}
'''

advisor_dir = ROOT / "crates/jcode-app-core/src/advisor"
advisor_dir.mkdir(parents=True, exist_ok=True)
(advisor_dir / "persistence.rs").write_text(persistence)

replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "mod evidence;\n",
    "mod evidence;\nmod persistence;\n",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "use std::collections::{HashMap, VecDeque, hash_map::DefaultHasher};\nuse std::hash::{Hash, Hasher};\nuse std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};\nuse std::sync::{Arc, LazyLock, Mutex};",
    "use std::collections::{HashMap, HashSet, VecDeque};\nuse std::path::PathBuf;\nuse std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};\nuse std::sync::{Arc, LazyLock, Mutex};",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    fn dedupe_hash(&self) -> u64 {\n        let mut hasher = DefaultHasher::new();\n        self.severity.hash(&mut hasher);\n        self.summary.hash(&mut hasher);\n        self.recommended_action.hash(&mut hasher);\n        self.evidence.hash(&mut hasher);\n        hasher.finish()\n    }",
    "    fn dedupe_fingerprint(&self) -> String {\n        persistence::note_fingerprint(self)\n    }",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    last_note_hash: Option<u64>,\n    last_error: Option<String>,",
    "    last_note_fingerprint: Option<String>,\n    last_error: Option<String>,",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    enabled_override: Option<bool>,\n    notes: VecDeque<AdvisorNoteMetadata>,\n}\n\n#[derive(Default)]\npub struct AdvisorManager {\n    sessions: Mutex<HashMap<String, AdvisorRuntime>>,\n    next_review_id: AtomicU64,\n    next_note_id: AtomicU64,\n}\n\nimpl AdvisorManager {",
    "    enabled_override: Option<bool>,\n    notes: VecDeque<AdvisorNoteMetadata>,\n    next_note_ordinal: u64,\n}\n\npub struct AdvisorManager {\n    sessions: Mutex<HashMap<String, AdvisorRuntime>>,\n    loaded_sessions: Mutex<HashSet<String>>,\n    next_review_id: AtomicU64,\n    persistence_root: Option<PathBuf>,\n}\n\nimpl Default for AdvisorManager {\n    fn default() -> Self {\n        Self {\n            sessions: Mutex::new(HashMap::new()),\n            loaded_sessions: Mutex::new(HashSet::new()),\n            next_review_id: AtomicU64::new(0),\n            persistence_root: None,\n        }\n    }\n}\n\nimpl AdvisorManager {\n    fn persistent() -> Self {\n        Self {\n            persistence_root: Some(crate::storage::durable_state_dir().join(\"advisor\")),\n            ..Self::default()\n        }\n    }\n\n    #[cfg(test)]\n    fn persistent_at(root: PathBuf) -> Self {\n        Self {\n            persistence_root: Some(root),\n            ..Self::default()\n        }\n    }\n\n    fn ensure_loaded(&self, owner_session_id: &str) {\n        let Some(root) = self.persistence_root.as_deref() else {\n            return;\n        };\n        let Ok(mut loaded_sessions) = self.loaded_sessions.lock() else {\n            return;\n        };\n        if loaded_sessions.contains(owner_session_id) {\n            return;\n        }\n\n        match persistence::load(root, owner_session_id) {\n            Ok(Some(runtime)) => {\n                if let Ok(mut sessions) = self.sessions.lock() {\n                    sessions.entry(owner_session_id.to_string()).or_insert(runtime);\n                }\n            }\n            Ok(None) => {}\n            Err(error) => {\n                let error = truncate_utf8(redact_secrets(&error.to_string()), 1000);\n                crate::logging::warn(&format!(\n                    \"ADVISOR_STATE_LOAD_FAILED session={owner_session_id}: {error}\"\n                ));\n            }\n        }\n        loaded_sessions.insert(owner_session_id.to_string());\n    }\n\n    fn persist_runtime(&self, owner_session_id: &str) {\n        let Some(root) = self.persistence_root.as_deref() else {\n            return;\n        };\n        let state = {\n            let Ok(sessions) = self.sessions.lock() else {\n                return;\n            };\n            let Some(runtime) = sessions.get(owner_session_id) else {\n                return;\n            };\n            persistence::capture(owner_session_id, runtime)\n        };\n        if let Err(error) = persistence::save(root, owner_session_id, &state) {\n            let error = truncate_utf8(redact_secrets(&error.to_string()), 1000);\n            crate::logging::warn(&format!(\n                \"ADVISOR_STATE_SAVE_FAILED session={owner_session_id}: {error}\"\n            ));\n        }\n    }",
)

replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn snapshot(&self, owner_session_id: &str) -> Option<AdvisorRuntimeSnapshot> {\n        let sessions = self.sessions.lock().ok()?;",
    "    pub fn snapshot(&self, owner_session_id: &str) -> Option<AdvisorRuntimeSnapshot> {\n        self.ensure_loaded(owner_session_id);\n        let sessions = self.sessions.lock().ok()?;",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn notes(&self, owner_session_id: &str) -> Vec<AdvisorNoteMetadata> {\n        self.sessions",
    "    pub fn notes(&self, owner_session_id: &str) -> Vec<AdvisorNoteMetadata> {\n        self.ensure_loaded(owner_session_id);\n        self.sessions",
)
replace_between(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn set_enabled(&self, owner_session_id: &str, enabled: bool) {",
    "    pub fn is_enabled(&self, owner_session_id: &str, configured_default: bool) -> bool {",
    '''    pub fn set_enabled(&self, owner_session_id: &str, enabled: bool) {
        self.ensure_loaded(owner_session_id);
        let changed = if let Ok(mut sessions) = self.sessions.lock() {
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
            true
        } else {
            false
        };
        if changed {
            self.persist_runtime(owner_session_id);
        }
    }

''',
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn is_enabled(&self, owner_session_id: &str, configured_default: bool) -> bool {\n        self.sessions",
    "    pub fn is_enabled(&self, owner_session_id: &str, configured_default: bool) -> bool {\n        self.ensure_loaded(owner_session_id);\n        self.sessions",
)
replace_between(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn resolve_note(\n",
    "    pub fn blocks_tool_call(\n",
    '''    pub fn resolve_note(
        &self,
        owner_session_id: &str,
        id: &str,
        disposition: AdvisorNoteDisposition,
    ) -> bool {
        self.ensure_loaded(owner_session_id);
        let resolved = {
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
        };
        if resolved {
            self.persist_runtime(owner_session_id);
        }
        resolved
    }

''',
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "    ) -> Option<String> {\n        if !is_risky_tool_call(tool_name, input) {",
    "    ) -> Option<String> {\n        self.ensure_loaded(owner_session_id);\n        if !is_risky_tool_call(tool_name, input) {",
)
replace_between(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn remove(&self, owner_session_id: &str) {",
    "    pub fn schedule_turn(\n",
    '''    /// Unload a session runtime without deleting its restart-resumable state.
    pub fn unload(&self, owner_session_id: &str) {
        if let Ok(mut loaded_sessions) = self.loaded_sessions.lock() {
            loaded_sessions.remove(owner_session_id);
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(owner_session_id);
            }
        } else if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(owner_session_id);
        }
    }

    /// Reset advisor state because the owning transcript history changed.
    pub fn remove(&self, owner_session_id: &str) {
        self.unload(owner_session_id);
        let Some(root) = self.persistence_root.as_deref() else {
            return;
        };
        if let Err(error) = persistence::delete(root, owner_session_id) {
            let error = truncate_utf8(redact_secrets(&error.to_string()), 1000);
            crate::logging::warn(&format!(
                "ADVISOR_STATE_DELETE_FAILED session={owner_session_id}: {error}"
            ));
        }
    }

''',
)
replace_between(
    "crates/jcode-app-core/src/advisor.rs",
    "    pub fn schedule_turn(\n",
    "    fn spawn_review(\n",
    '''    pub fn schedule_turn(
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
        self.ensure_loaded(&owner_session_id);

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

        enum ScheduleDecision {
            Skipped,
            Queued,
            Start {
                review_id: u64,
                provider: Arc<dyn Provider>,
                queue: SoftInterruptQueue,
                input: AdvisorTurnInput,
                config: AdvisorConfig,
            },
        }

        let decision = {
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
                ScheduleDecision::Skipped
            } else if runtime.status == AdvisorStatus::Reviewing {
                runtime.pending = Some(pending);
                ScheduleDecision::Queued
            } else {
                let PendingReview {
                    provider,
                    queue,
                    input,
                    config,
                } = pending;
                let review_id = self
                    .next_review_id
                    .fetch_add(1, AtomicOrdering::Relaxed)
                    .saturating_add(1);
                runtime.cursor = runtime.cursor.saturating_add(1);
                runtime.status = AdvisorStatus::Reviewing;
                runtime.notes_emitted = 0;
                runtime.last_error = None;
                runtime.active_review_id = review_id;
                runtime.private_context.push(input.clone());
                if runtime.private_context.len() > MAX_PRIVATE_CONTEXT {
                    runtime.private_context.remove(0);
                }
                ScheduleDecision::Start {
                    review_id,
                    provider,
                    queue,
                    input,
                    config,
                }
            }
        };
        // Persist the turn/cursor before dispatch. A process crash can lose an
        // in-flight review, but it cannot replay it or retain its private input.
        self.persist_runtime(&owner_session_id);

        match decision {
            ScheduleDecision::Skipped => false,
            ScheduleDecision::Queued => true,
            ScheduleDecision::Start {
                review_id,
                provider,
                queue,
                input,
                config,
            } => {
                self.spawn_review(
                    owner_session_id,
                    review_id,
                    provider,
                    queue,
                    input,
                    config,
                );
                true
            }
        }
    }

''',
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "        };\n        self.spawn_review(\n            owner_session_id,\n            next.0,",
    "        };\n        self.persist_runtime(&owner_session_id);\n        self.spawn_review(\n            owner_session_id,\n            next.0,",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "        let note_hash = note.dedupe_hash();",
    "        let note_fingerprint = note.dedupe_fingerprint();",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "            if runtime.last_note_hash == Some(note_hash)\n                || runtime.notes_emitted >= config.max_notes_per_turn\n            {\n                false\n            } else {\n                runtime.last_note_hash = Some(note_hash);\n                runtime.notes_emitted += 1;\n                let note_id = self\n                    .next_note_id\n                    .fetch_add(1, AtomicOrdering::Relaxed)\n                    .saturating_add(1);\n                runtime.notes.push_back(AdvisorNoteMetadata {\n                    id: format!(\"adv-{note_id:016x}\"),",
    "            if runtime.last_note_fingerprint.as_deref()\n                == Some(note_fingerprint.as_str())\n                || runtime.notes_emitted >= config.max_notes_per_turn\n            {\n                false\n            } else {\n                runtime.last_note_fingerprint = Some(note_fingerprint);\n                runtime.notes_emitted += 1;\n                runtime.next_note_ordinal = runtime.next_note_ordinal.saturating_add(1);\n                let note_id =\n                    persistence::note_id(&owner_session_id, runtime.next_note_ordinal);\n                runtime.notes.push_back(AdvisorNoteMetadata {\n                    id: note_id,",
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "        };\n\n        if should_deliver && let Ok(mut pending) = queue.lock() {",
    "        };\n        self.persist_runtime(&owner_session_id);\n\n        if should_deliver && let Ok(mut pending) = queue.lock() {",
)
replace_between(
    "crates/jcode-app-core/src/advisor.rs",
    "    fn fail(&self, owner_session_id: &str, review_id: u64, error: String) {",
    "}\n\nfn truncate_utf8",
    '''    fn fail(&self, owner_session_id: &str, review_id: u64, error: String) {
        let error = redact_secrets(&error)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        crate::logging::warn(&format!(
            "ADVISOR_FAILURE session={owner_session_id}: {error}"
        ));
        let changed = if let Ok(mut sessions) = self.sessions.lock()
            && let Some(runtime) = sessions.get_mut(owner_session_id)
            && runtime.active_review_id == review_id
            && runtime.status == AdvisorStatus::Reviewing
        {
            runtime.status = AdvisorStatus::Failed;
            runtime.last_error = Some(truncate_utf8(error, 1000));
            true
        } else {
            false
        };
        if changed {
            self.persist_runtime(owner_session_id);
        }
    }
''',
)
replace_once(
    "crates/jcode-app-core/src/advisor.rs",
    "static ADVISOR_MANAGER: LazyLock<Arc<AdvisorManager>> =\n    LazyLock::new(|| Arc::new(AdvisorManager::default()));",
    "static ADVISOR_MANAGER: LazyLock<Arc<AdvisorManager>> =\n    LazyLock::new(|| Arc::new(AdvisorManager::persistent()));",
)
replace_once(
    "crates/jcode-app-core/src/agent.rs",
    "        crate::advisor::advisor_manager().remove(&self.session.id);",
    "        crate::advisor::advisor_manager().unload(&self.session.id);",
)
replace_once(
    "crates/jcode-app-core/src/agent/turn_execution.rs",
    "        crate::advisor::advisor_manager().remove(&previous_session_id);",
    "        crate::advisor::advisor_manager().unload(&previous_session_id);",
)

advisor_path = ROOT / "crates/jcode-app-core/src/advisor.rs"
text = advisor_path.read_text()
if not text.endswith("\n}\n"):
    raise SystemExit("advisor test module did not end as expected")
tests = r'''

    #[tokio::test]
    async fn redacted_notes_controls_and_cursors_resume_without_private_context() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("advisor-state");
        let session_id = "restart-resume";
        let manager = Arc::new(AdvisorManager::persistent_at(root.clone()));
        manager.set_enabled(session_id, true);
        assert!(manager.schedule_turn(
            session_id.to_string(),
            Arc::new(AdvisorProvider {
                calls: Arc::new(AtomicUsize::new(0)),
                response: r#"{"severity":"blocker","summary":"OPENAI_API_KEY=sk-test-openai-example","evidence":["private evidence"],"recommended_action":"acknowledge it","blocking":true}"#.to_string(),
            }),
            Arc::new(Mutex::new(Vec::new())),
            AdvisorTurnInput {
                objective: "private-objective-must-not-persist".to_string(),
                ..AdvisorTurnInput::default()
            },
            AdvisorConfig {
                enabled: false,
                redact: false,
                ..AdvisorConfig::default()
            },
        ));
        wait_for_status(&manager, session_id, AdvisorStatus::Ready).await;
        let note = manager.notes(session_id).pop().expect("advisor note");
        assert!(manager.resolve_note(
            session_id,
            &note.id,
            AdvisorNoteDisposition::Acknowledged,
        ));
        manager.unload(session_id);

        let state_path = persistence::state_path(&root, session_id);
        let encoded = std::fs::read_to_string(&state_path).expect("persisted advisor state");
        assert!(!encoded.contains("private-objective-must-not-persist"));
        assert!(!encoded.contains("sk-test-openai-example"));
        assert!(!encoded.contains(session_id));

        let restored = AdvisorManager::persistent_at(root.clone());
        let snapshot = restored.snapshot(session_id).expect("restored snapshot");
        assert_eq!(snapshot.turns_observed, 1);
        assert_eq!(snapshot.cursor, 1);
        assert_eq!(snapshot.private_context_len, 0);
        assert_eq!(snapshot.unresolved_blocking_notes, 0);
        assert!(restored.is_enabled(session_id, false));
        let restored_note = restored.notes(session_id).pop().expect("restored note");
        assert_eq!(restored_note.id, note.id);
        assert_eq!(
            restored_note.disposition,
            AdvisorNoteDisposition::Acknowledged
        );
        assert!(restored_note.summary.contains("[REDACTED_SECRET]"));

        restored.set_enabled(session_id, false);
        restored.unload(session_id);
        let after_second_restart = AdvisorManager::persistent_at(root);
        assert!(!after_second_restart.is_enabled(session_id, true));
    }

    #[test]
    fn transcript_reset_deletes_durable_advisor_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("advisor-state");
        let session_id = "history-reset";
        let manager = AdvisorManager::persistent_at(root.clone());
        manager.set_enabled(session_id, true);
        let state_path = persistence::state_path(&root, session_id);
        assert!(state_path.exists());

        manager.remove(session_id);
        assert!(!state_path.exists());
        assert!(!state_path.with_extension("bak").exists());
        assert!(
            AdvisorManager::persistent_at(root)
                .snapshot(session_id)
                .is_none()
        );
    }
'''
advisor_path.write_text(text[:-3] + tests + "}\n")
