use super::*;

const MAX_HISTORY_BYTES: usize = 192 * 1024;
const MAX_EXCHANGES: usize = 12;

/// Store complete exchanges so maintenance never strands a tool result or a
/// signed provider reasoning block. Primary hidden reasoning never enters here.
#[derive(Default)]
pub(super) struct AdvisorHistory {
    exchanges: VecDeque<Vec<Message>>,
    original_objective: String,
    initial_visible_context: String,
    maintained: bool,
}

impl AdvisorHistory {
    pub(super) fn messages(&self, objective: &str, budget: usize) -> Vec<Message> {
        let mut messages = Vec::new();
        if !self.original_objective.is_empty() && self.original_objective != objective {
            messages.push(Message::user(&format!(
                "Original task objective (retained across advisor context maintenance):\n{}",
                self.original_objective
            )));
        }
        if self.maintained && !self.initial_visible_context.is_empty() {
            messages.push(Message::user(&format!(
                "Initial visible project/task context (historical evidence; newer user instructions take precedence):\n{}",
                self.initial_visible_context
            )));
        }
        let mut remaining =
            budget.saturating_sub(serde_json::to_vec(&messages).map_or(0, |value| value.len()));
        let mut exchanges = Vec::new();
        for exchange in self.exchanges.iter().rev() {
            let size = serde_json::to_vec(exchange).map_or(usize::MAX, |value| value.len());
            if size > remaining {
                break;
            }
            remaining -= size;
            exchanges.push(exchange);
        }
        for exchange in exchanges.into_iter().rev() {
            messages.extend(exchange.iter().cloned());
        }
        messages
    }

    pub(super) fn retain(&mut self, objective: &str, exchange: Vec<Message>) {
        if self.original_objective.is_empty() {
            self.original_objective = truncate_utf8(redact_secrets(objective), MAX_FIELD_BYTES);
            self.initial_visible_context.clear();
            if let Some(context) = exchange.first().and_then(|message| {
                message.content.iter().find_map(|block| match block {
                    crate::message::ContentBlock::Text { text, .. } => {
                        Some(truncate_utf8(redact_secrets(text), 16 * 1024))
                    }
                    _ => None,
                })
            }) {
                self.initial_visible_context = context;
            }
        }
        self.exchanges.push_back(exchange);
        while self.exchanges.len() > MAX_EXCHANGES || self.bytes() > MAX_HISTORY_BYTES {
            self.exchanges.pop_front();
            self.maintained = true;
        }
    }

    fn bytes(&self) -> usize {
        self.exchanges
            .iter()
            .map(|exchange| {
                serde_json::to_vec(exchange).map_or(MAX_HISTORY_BYTES + 1, |value| value.len())
            })
            .sum()
    }

    pub(super) fn len(&self) -> usize {
        self.exchanges.iter().map(Vec::len).sum()
    }
}
