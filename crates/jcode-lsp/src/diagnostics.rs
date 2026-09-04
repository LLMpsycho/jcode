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

pub fn diagnostic_delta(
    before: Option<&DiagnosticSnapshot>,
    after: &DiagnosticSnapshot,
) -> Vec<Diagnostic> {
    let before = before
        .map(|snapshot| {
            snapshot
                .items
                .iter()
                .map(|diagnostic| (diagnostic_identity(diagnostic), severity_rank(diagnostic)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut delta = after
        .items
        .iter()
        .filter(|diagnostic| {
            let identity = diagnostic_identity(diagnostic);
            before
                .get(&identity)
                .is_none_or(|previous| severity_rank(diagnostic) < *previous)
        })
        .cloned()
        .collect::<Vec<_>>();
    delta.sort_by_key(|diagnostic| {
        (
            severity_rank(diagnostic),
            diagnostic.range.start.line,
            diagnostic.range.start.character,
        )
    });
    delta
}

fn diagnostic_identity(diagnostic: &Diagnostic) -> String {
    serde_json::to_string(&(
        diagnostic.range,
        &diagnostic.code,
        &diagnostic.source,
        &diagnostic.message,
    ))
    .unwrap_or_else(|_| diagnostic.message.clone())
}

fn severity_rank(diagnostic: &Diagnostic) -> u32 {
    diagnostic.severity.map(|severity| severity.0).unwrap_or(5)
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

    pub async fn put(&self, snapshot: DiagnosticSnapshot) {
        self.snapshots
            .write()
            .await
            .insert(snapshot.uri.clone(), snapshot);
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

    #[test]
    fn delta_returns_only_new_and_worsened_diagnostics() {
        let make = |severity, message: &str, line| Diagnostic {
            range: crate::Range {
                start: crate::Position { line, character: 0 },
                end: crate::Position { line, character: 1 },
            },
            severity: Some(crate::DiagnosticSeverity(severity)),
            code: None,
            source: Some("test".to_owned()),
            message: message.to_owned(),
            data: None,
        };
        let before = DiagnosticSnapshot {
            uri: "file:///x".to_owned(),
            version: Some(1),
            items: vec![make(2, "existing", 1), make(2, "worsened", 2)],
        };
        let after = DiagnosticSnapshot {
            uri: "file:///x".to_owned(),
            version: Some(2),
            items: vec![
                make(2, "existing", 1),
                make(1, "worsened", 2),
                make(1, "new", 3),
            ],
        };
        let delta = diagnostic_delta(Some(&before), &after);
        assert_eq!(
            delta
                .iter()
                .map(|item| item.message.as_str())
                .collect::<Vec<_>>(),
            ["worsened", "new"]
        );
    }
}
