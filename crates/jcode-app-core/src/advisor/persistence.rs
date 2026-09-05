use super::*;
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_STATE_BYTES: u64 = 256 * 1024;

/// No transcript, provider context, pending request, or raw tool output belongs
/// in this checkpoint. Cursor is a lifetime cost counter, not a replay request.
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u8,
    enabled_override: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_override: Option<model_selection::AdvisorModelOverride>,
    turns_observed: u64,
    cursor: u64,
    #[serde(default)]
    immunity_until_turn: u64,
    #[serde(default)]
    immunity_turns: u64,
    notes: VecDeque<AdvisorNoteMetadata>,
}

impl AdvisorManager {
    pub fn persistent(root: PathBuf) -> Self {
        Self {
            store: Some(root),
            ..Self::default()
        }
    }

    fn state_path(&self, session: &str) -> Option<PathBuf> {
        // Public session IDs can contain arbitrary text; never interpolate them
        // into a pathname or expose their content in persistence errors.
        let key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, session.as_bytes());
        self.store
            .as_ref()
            .map(|root| root.join(format!("{key}.json")))
    }

    pub(super) fn persist(&self, session: &str, runtime: &mut AdvisorRuntime) -> Result<()> {
        let Some(path) = self.state_path(session) else {
            return Ok(());
        };
        let checkpoint = Checkpoint {
            version: 1,
            enabled_override: runtime.enabled_override,
            model_override: runtime.model_override.clone(),
            turns_observed: runtime.turns_observed,
            cursor: runtime.cursor,
            immunity_until_turn: runtime.immunity_until_turn,
            immunity_turns: runtime.immunity_turns,
            notes: runtime.notes.iter().cloned().map(sanitize_note).collect(),
        };
        let result = save(&path, &checkpoint);
        runtime.persistence_failed = result.is_err();
        if result.is_err() {
            runtime.last_error = Some("advisor checkpoint could not be saved".into());
        }
        result.map_err(|_| {
            anyhow::anyhow!("advisor checkpoint could not be saved; control is not durable")
        })
    }

    /// Restore controls and budget without replaying a provider request or
    /// delivering a historical interrupt. A corrupt checkpoint gates effects
    /// until the user explicitly disables or repairs the advisor.
    pub fn resume(&self, session: &str) {
        let Some(path) = self.state_path(session) else {
            return;
        };
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        if sessions.contains_key(session) {
            return;
        }
        match load(&path) {
            Ok(Some(checkpoint)) => {
                sessions.insert(
                    session.to_string(),
                    AdvisorRuntime {
                        enabled_override: checkpoint.enabled_override,
                        model_override: checkpoint.model_override,
                        turns_observed: checkpoint.turns_observed,
                        cursor: checkpoint.cursor,
                        immunity_until_turn: checkpoint.immunity_until_turn,
                        immunity_turns: checkpoint.immunity_turns.min(100),
                        notes: checkpoint.notes,
                        ..AdvisorRuntime::default()
                    },
                );
            }
            Ok(None) => {}
            Err(_) => {
                sessions.insert(session.to_string(), AdvisorRuntime {
                    enabled_override: Some(true),
                    persistence_failed: true,
                    status: AdvisorStatus::Failed,
                    last_error: Some("advisor checkpoint could not be restored; explicitly disable or repair it".into()),
                    ..AdvisorRuntime::default()
                });
            }
        }
    }

    /// Changed transcript invalidates review context and notes, but does not
    /// revoke a user's disable or replenish the session's provider budget.
    pub fn reset_history(&self, session: &str) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let Some(previous) = sessions.remove(session) else {
            return;
        };
        clear_queued_notes(&previous);
        let mut runtime = AdvisorRuntime {
            immunity_until_turn: previous.immunity_until_turn,
            immunity_turns: previous.immunity_turns,
            enabled_override: previous.enabled_override,
            model_override: previous.model_override,
            turns_observed: previous.turns_observed,
            cursor: previous.cursor,
            persistence_failed: previous.persistence_failed,
            last_error: if previous.persistence_failed {
                previous.last_error
            } else {
                None
            },
            status: if previous.persistence_failed {
                AdvisorStatus::Failed
            } else {
                AdvisorStatus::Idle
            },
            ..AdvisorRuntime::default()
        };
        // History changes cannot repair unknown durable state or erase the
        // evidence needed to recover it. Only an explicit control may retry.
        if !runtime.persistence_failed && self.persist(session, &mut runtime).is_err() {
            runtime.status = AdvisorStatus::Failed;
        }
        sessions.insert(session.to_string(), runtime);
    }
}

fn save(path: &Path, checkpoint: &Checkpoint) -> Result<()> {
    if serde_json::to_vec(checkpoint)?.len() as u64 > MAX_STATE_BYTES {
        bail!("checkpoint exceeds bound");
    }
    crate::storage::write_json_secret(path, checkpoint)
}

fn load(path: &Path) -> Result<Option<Checkpoint>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        bail!("checkpoint exceeds bound");
    }
    let mut checkpoint: Checkpoint =
        serde_json::from_slice(&bytes).context("invalid advisor checkpoint")?;
    if checkpoint.version != 1 || checkpoint.notes.len() > MAX_NOTE_METADATA {
        bail!("unsupported advisor checkpoint");
    }
    if let Some(model_selection::AdvisorModelOverride::Selected {
        selection,
        reasoning_effort,
    }) = &checkpoint.model_override
    {
        routing::validate_persisted_selection(selection)?;
        if reasoning_effort.as_ref().is_some_and(|effort| {
            effort.len() > 32
                || effort.chars().any(char::is_control)
                || redact_secrets(effort) != *effort
        }) {
            bail!("invalid advisor reasoning effort");
        }
    }
    for note in &checkpoint.notes {
        if !note.id.starts_with("adv-") || note.id.len() > 64 {
            bail!("invalid advisor note ID");
        }
    }
    checkpoint.notes = checkpoint.notes.into_iter().map(sanitize_note).collect();
    Ok(Some(checkpoint))
}

fn sanitize_note(mut note: AdvisorNoteMetadata) -> AdvisorNoteMetadata {
    note.summary = truncate_utf8(redact_secrets(&note.summary), 1024);
    note.recommended_action = truncate_utf8(redact_secrets(&note.recommended_action), 1024);
    note.evidence.truncate(4);
    for evidence in &mut note.evidence {
        *evidence = truncate_utf8(redact_secrets(evidence), 256);
    }
    // Account for JSON escaping, including pathological control characters.
    while serde_json::to_vec(&note).is_ok_and(|bytes| bytes.len() > 4096) {
        if note.evidence.pop().is_some() {
            continue;
        }
        note.summary = truncate_utf8(note.summary.clone(), note.summary.len() / 2);
        note.recommended_action = truncate_utf8(
            note.recommended_action.clone(),
            note.recommended_action.len() / 2,
        );
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_preserves_controls_without_private_context_or_replay() {
        let dir = tempfile::tempdir().expect("directory");
        let manager = AdvisorManager::persistent(dir.path().to_path_buf());
        manager.set_enabled("../session", true).expect("enable");
        {
            let mut sessions = manager.sessions.lock().expect("sessions");
            let runtime = sessions.get_mut("../session").expect("runtime");
            runtime.cursor = 7;
            runtime.private_context.push(AdvisorTurnInput {
                objective: "PRIVATE_TRANSCRIPT".into(),
                ..AdvisorTurnInput::default()
            });
            runtime.notes.push_back(AdvisorNoteMetadata {
                id: "adv-existing".into(),
                severity: AdvisorSeverity::Blocker,
                summary: "verify".into(),
                evidence: vec![],
                recommended_action: "test".into(),
                blocking: true,
                disposition: AdvisorNoteDisposition::Unresolved,
            });
            manager.persist("../session", runtime).expect("persist");
        }
        let bytes =
            std::fs::read_to_string(manager.state_path("../session").expect("path")).expect("read");
        assert!(!bytes.contains("PRIVATE_TRANSCRIPT"));
        drop(manager);
        let manager = AdvisorManager::persistent(dir.path().to_path_buf());
        manager.resume("../session");
        assert_eq!(manager.snapshot("../session").expect("snapshot").cursor, 7);
        assert_eq!(
            manager
                .snapshot("../session")
                .expect("snapshot")
                .private_context_len,
            0
        );
        assert_eq!(manager.notes("../session")[0].id, "adv-existing");
        assert!(
            manager
                .resolve_note(
                    "../session",
                    "adv-existing",
                    AdvisorNoteDisposition::Dismissed
                )
                .expect("dismiss")
        );
        manager.set_enabled("../session", false).expect("disable");
        let restarted = AdvisorManager::persistent(dir.path().to_path_buf());
        restarted.resume("../session");
        assert!(!restarted.is_enabled("../session", true));
        assert_eq!(
            restarted.notes("../session")[0].disposition,
            AdvisorNoteDisposition::Dismissed
        );
        restarted.reset_history("../session");
        assert!(!restarted.is_enabled("../session", true));
        assert!(restarted.notes("../session").is_empty());
    }

    #[test]
    fn corrupt_checkpoint_and_failed_control_write_are_visible() {
        let dir = tempfile::tempdir().expect("directory");
        let manager = AdvisorManager::persistent(dir.path().to_path_buf());
        std::fs::write(manager.state_path("broken").expect("path"), "{").expect("corrupt fixture");
        manager.resume("broken");
        assert!(
            manager
                .blocks_tool_call("broken", "effect", crate::tool::ToolCapability::Execute)
                .is_some()
        );
        manager
            .set_enabled("broken", false)
            .expect("explicit recovery");
        assert!(
            manager
                .blocks_tool_call("broken", "effect", crate::tool::ToolCapability::Execute)
                .is_none()
        );
        let file = dir.path().join("file");
        std::fs::write(&file, "not a directory").expect("fixture");
        let blocked = AdvisorManager::persistent(file);
        assert!(blocked.set_enabled("session", false).is_err());
    }

    #[test]
    fn history_reset_preserves_corrupt_checkpoint_until_explicit_recovery() {
        let dir = tempfile::tempdir().expect("directory");
        let manager = AdvisorManager::persistent(dir.path().to_path_buf());
        let session = "broken_history";
        let path = manager.state_path(session).expect("path");
        std::fs::write(&path, "{").expect("corrupt fixture");
        manager.resume(session);
        let restore_error = manager.snapshot(session).expect("failed state").last_error;

        for _ in 0..2 {
            manager.reset_history(session);
            assert_eq!(std::fs::read_to_string(&path).expect("checkpoint"), "{");
            assert!(
                manager
                    .blocks_tool_call(session, "effect", crate::tool::ToolCapability::Execute)
                    .is_some()
            );
            let snapshot = manager.snapshot(session).expect("failed state");
            assert_eq!(snapshot.status, AdvisorStatus::Failed);
            assert_eq!(snapshot.last_error, restore_error);
        }

        manager
            .set_enabled(session, false)
            .expect("explicit recovery");
        assert!(
            manager
                .blocks_tool_call(session, "effect", crate::tool::ToolCapability::Execute)
                .is_none()
        );
        assert!(
            !load(&path)
                .expect("repaired checkpoint")
                .expect("state")
                .enabled_override
                .expect("explicit disable")
        );
    }

    #[test]
    fn checkpoint_redacts_notes_and_bounds_escaped_content() {
        let note = AdvisorNoteMetadata {
            id: "adv-safe".into(),
            severity: AdvisorSeverity::Concern,
            summary: "OPENAI_API_KEY=sk-test-openai-example".into(),
            evidence: vec!["\0".repeat(4096); 8],
            recommended_action: "\0".repeat(4096),
            blocking: false,
            disposition: AdvisorNoteDisposition::Acknowledged,
        };
        let cleaned = sanitize_note(note);
        let bytes = serde_json::to_string(&cleaned).expect("encode");
        assert!(!bytes.contains("sk-test-openai-example"));
        assert!(bytes.len() <= 4096);
        assert_eq!(cleaned.disposition, AdvisorNoteDisposition::Acknowledged);
    }
}
