use super::*;

const MAX_CONCERNS: usize = 64;

/// A bounded concern identity ledger, independent of provider conversation.
/// Handling one concern must never blind the advisor to unrelated new issues.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct ConcernLedger {
    #[serde(default)]
    entries: VecDeque<ConcernRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ConcernRecord {
    key: String,
    #[serde(default)]
    identity: Option<String>,
    note_id: String,
    summary: String,
    severity: AdvisorSeverity,
    handled_until_turn: Option<u64>,
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn concern_key(explicit: Option<&str>, note: &AdvisorNote) -> String {
    let source = explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize)
        .unwrap_or_else(|| normalize(&format!("{} {}", note.summary, note.recommended_action)));
    // Keys cannot carry arbitrary repository content into durable state.
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, source.as_bytes()).to_string()
}

impl ConcernLedger {
    pub(super) fn accepts(&self, key: &str, severity: AdvisorSeverity, turn: u64) -> bool {
        self.entries
            .iter()
            .rev()
            .find(|item| item.key == key)
            .is_none_or(|item| {
                if severity > item.severity {
                    return true;
                }
                item.handled_until_turn.is_some_and(|until| turn > until)
            })
    }

    pub(super) fn record(
        &mut self,
        key: String,
        identity: Option<&str>,
        note: &AdvisorNoteMetadata,
    ) {
        self.entries.retain(|item| item.key != key);
        self.entries.push_back(ConcernRecord {
            key,
            identity: identity.map(|value| durable_text(value, 128)),
            note_id: note.id.clone(),
            summary: durable_text(&note.summary, 512),
            severity: note.severity,
            handled_until_turn: None,
        });
        while self.entries.len() > MAX_CONCERNS {
            self.entries.pop_front();
        }
    }

    pub(super) fn handle(&mut self, note_id: &str, until: u64) {
        if let Some(item) = self.entries.iter_mut().find(|item| item.note_id == note_id)
            && item.handled_until_turn.is_none()
        {
            item.handled_until_turn = Some(until);
        }
    }

    pub(super) fn context(&self, turn: u64) -> String {
        self.entries
            .iter()
            .rev()
            .take(16)
            .map(|item| {
                let state = match item.handled_until_turn {
                    Some(until) if turn <= until => {
                        "handled; do not repeat without stronger evidence"
                    }
                    Some(_) => "previously handled; re-check before raising again",
                    None => "already reported; do not repeat",
                };
                format!(
                    "- {:?}: {} (concern_id: {}; {state})",
                    item.severity,
                    item.summary,
                    item.identity.as_deref().unwrap_or("legacy finding")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn validate(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.entries.len() <= MAX_CONCERNS,
            "too many advisor concerns"
        );
        for item in &mut self.entries {
            anyhow::ensure!(
                item.key.len() <= 64 && item.note_id.len() <= 64,
                "invalid concern identity"
            );
            item.summary = durable_text(&item.summary, 512);
            item.identity = item
                .identity
                .as_deref()
                .map(|value| durable_text(value, 128));
        }
        Ok(())
    }
}

fn durable_text(value: &str, limit: usize) -> String {
    truncate_utf8(
        redact_secrets(value)
            .chars()
            .filter(|character| !character.is_control())
            .collect(),
        limit,
    )
}
