//! Durable, minimal advisor state.
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

pub(super) fn capture(owner_session_id: &str, runtime: &AdvisorRuntime) -> PersistedAdvisorState {
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
