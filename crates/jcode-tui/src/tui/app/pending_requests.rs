use super::PreparedTransferSession;
use std::sync::mpsc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub(super) struct PendingRemoteMessage {
    pub(super) content: String,
    pub(super) images: Vec<(String, String)>,
    pub(super) is_system: bool,
    pub(super) system_reminder: Option<String>,
    pub(super) auto_retry: bool,
    pub(super) retry_attempts: u8,
    pub(super) retry_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingSplitPrompt {
    pub(super) content: String,
    pub(super) images: Vec<(String, String)>,
}

pub(super) struct PendingLocalTransfer {
    pub(super) receiver: mpsc::Receiver<anyhow::Result<PreparedTransferSession>>,
}
