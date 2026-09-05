use super::*;

pub(super) fn push_aside(runtime: &mut AdvisorRuntime, id: String) {
    if !runtime.asides.contains(&id) {
        runtime.asides.push_back(id);
    }
    while runtime.asides.len() > MAX_NOTE_METADATA {
        runtime.asides.pop_front();
    }
}

impl AdvisorManager {
    /// Called only after the primary committed a system-source interrupt to its
    /// conversation. Merely reviewing or displaying a card never starts this
    /// owner-wide interruption cooldown.
    pub fn record_delivery(&self, owner: &str, content: &str) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let delivered = sessions.iter().any(|(key, runtime)| {
            (key == owner || runtime.owner_session_id == owner)
                && runtime.queued_notes.iter().any(|(id, queued)| {
                    content.contains(queued)
                        && runtime
                            .notes
                            .iter()
                            .any(|note| note.id == *id && note.severity != AdvisorSeverity::Nit)
                })
        });
        if !delivered {
            return;
        }
        for (key, runtime) in sessions.iter_mut() {
            if key != owner && runtime.owner_session_id != owner {
                continue;
            }
            runtime
                .queued_notes
                .retain(|(_, queued)| !content.contains(queued));
            runtime.interruption_immunity_until_turn = runtime
                .turns_observed
                .saturating_add(runtime.interruption_immunity_turns);
            let _ = self.persist(key, runtime);
        }
    }

    pub fn retain_advisors(&self, owner: &str, active_keys: &[String]) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        for (key, runtime) in sessions.iter_mut() {
            if runtime.owner_session_id != owner || active_keys.contains(key) {
                continue;
            }
            clear_queued_notes(runtime);
            runtime.pending = None;
            runtime.active_review_id = 0;
            runtime.status = AdvisorStatus::Idle;
            runtime.capture = None;
        }
    }

    /// The final response is already visible. Keep lesser findings as cards;
    /// only a blocker can justify continuing the same user invocation.
    pub fn prepare_terminal_delivery(&self, owner: &str) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        for (key, runtime) in sessions.iter_mut() {
            if key != owner && runtime.owner_session_id != owner {
                continue;
            }
            runtime.terminal_phase = true;
            let asides: Vec<_> = runtime
                .queued_notes
                .iter()
                .filter(|(id, _)| {
                    runtime
                        .notes
                        .iter()
                        .any(|note| &note.id == id && note.severity != AdvisorSeverity::Blocker)
                })
                .cloned()
                .collect();
            if let Some(queue) = runtime
                .delivery_queue
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                && let Ok(mut pending) = queue.lock()
            {
                for (id, content) in asides {
                    if pending.iter().any(|message| message.content == content) {
                        pending.retain(|message| message.content != content);
                        push_aside(runtime, id);
                    }
                }
            }
        }
    }

    pub fn take_asides(&self, owner: &str) -> Vec<AdvisorNoteMetadata> {
        let Ok(mut sessions) = self.sessions.lock() else {
            return Vec::new();
        };
        let mut notes = Vec::new();
        for (key, runtime) in sessions.iter_mut() {
            if key != owner && runtime.owner_session_id != owner {
                continue;
            }
            for id in runtime.asides.drain(..) {
                if let Some(note) = runtime.notes.iter().find(|note| note.id == id) {
                    let mut note = note.clone();
                    if !runtime.advisor_label.is_empty() && runtime.advisor_label != "default" {
                        note.summary = format!("[{}] {}", runtime.advisor_label, note.summary);
                    }
                    notes.push(note);
                }
            }
        }
        notes
    }

    /// Cancel outstanding transport/tool futures as well as publication. This
    /// never disables the next explicit user turn or cancels primary tools.
    pub fn cancel_turn(&self, owner: &str) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        for (key, runtime) in sessions.iter_mut() {
            if key != owner && runtime.owner_session_id != owner {
                continue;
            }
            if let Some(cancellation) = runtime.cancellation.take() {
                cancellation.cancel();
            }
            runtime.pending = None;
            runtime.active_review_id = 0;
            if runtime.persistence_failed {
                runtime.status = AdvisorStatus::Failed;
            } else if runtime.status == AdvisorStatus::Reviewing {
                runtime.status = AdvisorStatus::Idle;
            }
            clear_queued_notes(runtime);
            runtime.queued_notes.clear();
        }
    }

    pub fn has_pending_review(&self, owner: &str) -> bool {
        self.sessions.lock().ok().is_some_and(|sessions| {
            sessions.iter().any(|(key, runtime)| {
                (key == owner || runtime.owner_session_id == owner)
                    && (runtime.status == AdvisorStatus::Reviewing || runtime.pending.is_some())
            })
        })
    }

    pub async fn wait_for_idle(&self, owner: &str, timeout: std::time::Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while self.has_pending_review(owner) {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        true
    }
}
