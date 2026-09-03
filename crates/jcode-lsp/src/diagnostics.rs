use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, broadcast};

use crate::{Diagnostic, NotificationMessage, PublishDiagnosticsParams};

#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticSnapshot {
    pub uri: String,
    pub version: Option<i64>,
    pub items: Vec<Diagnostic>,
}

#[derive(Clone, Default)]
pub struct DiagnosticsCache {
    snapshots: Arc<RwLock<HashMap<String, DiagnosticSnapshot>>>,
}

impl DiagnosticsCache {
    pub fn listen(mut notifications: broadcast::Receiver<NotificationMessage>) -> Self {
        let cache = Self::default();
        let snapshots = Arc::clone(&cache.snapshots);
        tokio::spawn(async move {
            loop {
                match notifications.recv().await {
                    Ok(notification)
                        if notification.method == "textDocument/publishDiagnostics" =>
                    {
                        let Some(params) = notification.params else {
                            continue;
                        };
                        let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params)
                        else {
                            continue;
                        };
                        snapshots.write().await.insert(
                            params.uri.clone(),
                            DiagnosticSnapshot {
                                uri: params.uri,
                                version: params.version,
                                items: params.diagnostics,
                            },
                        );
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        cache
    }

    pub async fn get(&self, uri: &str) -> Option<DiagnosticSnapshot> {
        self.snapshots.read().await.get(uri).cloned()
    }

    pub async fn wait_for_version(
        &self,
        uri: &str,
        minimum_version: i64,
        timeout: Duration,
    ) -> Option<DiagnosticSnapshot> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(snapshot) = self.get(uri).await
                && snapshot
                    .version
                    .is_some_and(|version| version >= minimum_version)
            {
                return Some(snapshot);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::*;

    #[tokio::test]
    async fn tracks_only_well_formed_diagnostics_notifications() {
        let (sender, receiver) = broadcast::channel(8);
        let cache = DiagnosticsCache::listen(receiver);
        sender
            .send(NotificationMessage::new(
                "window/logMessage",
                Some(json!({"message": "ignored"})),
            ))
            .unwrap();
        sender
            .send(NotificationMessage::new(
                "textDocument/publishDiagnostics",
                Some(json!({
                    "uri": "file:///workspace/src/lib.rs",
                    "version": 4,
                    "diagnostics": [{
                        "range": {
                            "start": {"line": 1, "character": 0},
                            "end": {"line": 1, "character": 3}
                        },
                        "severity": 1,
                        "message": "broken"
                    }]
                })),
            ))
            .unwrap();
        let snapshot = cache
            .wait_for_version("file:///workspace/src/lib.rs", 4, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(snapshot.version, Some(4));
        assert_eq!(snapshot.items[0].message, "broken");
        assert!(cache.get("file:///ignored").await.is_none());
    }
}
