use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;

use crate::document_sync::file_uri;
use crate::{
    DiagnosticSnapshot, DiagnosticsCache, DocumentState, DocumentSync, LspConfig, LspError,
    LspProcess, Position, ProcessStatus, Range, Result, SemanticVerification,
    SemanticVerificationStatus, config_digest, diagnostic_delta,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LspWorkspaceKey {
    pub canonical_root: PathBuf,
    pub worktree_identity: String,
    pub server_id: String,
    pub config_digest: [u8; 32],
}

impl LspWorkspaceKey {
    pub fn new(
        root: &Path,
        worktree_identity: impl Into<String>,
        server_id: impl Into<String>,
        config_digest: [u8; 32],
    ) -> Result<Self> {
        Ok(Self {
            canonical_root: root.canonicalize()?,
            worktree_identity: worktree_identity.into(),
            server_id: server_id.into(),
            config_digest,
        })
    }
}

pub struct LspWorkspace {
    key: LspWorkspaceKey,
    process: LspProcess,
    capabilities: Value,
    documents: DocumentSync,
    diagnostics: DiagnosticsCache,
    request_timeout: Duration,
    incremental_sync: bool,
    pull_diagnostics: bool,
    last_used_epoch_seconds: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LspWorkspaceStatus {
    pub process_status: ProcessStatus,
    pub recent_stderr: String,
}

impl LspWorkspace {
    pub fn key(&self) -> &LspWorkspaceKey {
        &self.key
    }

    pub fn process(&self) -> &LspProcess {
        &self.process
    }

    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }

    fn touch(&self) {
        self.last_used_epoch_seconds
            .store(epoch_seconds(), Ordering::Relaxed);
    }

    pub async fn sync_document(
        &self,
        path: &Path,
        language_id: &str,
        text: String,
    ) -> Result<DocumentState> {
        self.documents
            .sync(
                self.process.client(),
                &self.key.canonical_root,
                path,
                language_id,
                text,
                self.incremental_sync,
            )
            .await
    }

    pub async fn sync_document_from_disk(
        &self,
        path: &Path,
        language_id: &str,
    ) -> Result<DocumentState> {
        let text = tokio::fs::read_to_string(path).await?;
        let document = self.sync_document(path, language_id, text).await?;
        self.process
            .client()
            .notify(
                "textDocument/didSave",
                Some(serde_json::json!({"textDocument": {"uri": document.uri}})),
            )
            .await?;
        Ok(document)
    }

    pub async fn diagnostics(&self, path: &Path) -> Result<Option<DiagnosticSnapshot>> {
        let uri = file_uri(&path.canonicalize()?)?;
        Ok(self.diagnostics.get(&uri).await)
    }

    pub async fn wait_for_diagnostics(
        &self,
        document: &DocumentState,
        timeout: Duration,
    ) -> Option<DiagnosticSnapshot> {
        self.diagnostics
            .wait_for_version(&document.uri, document.version, timeout)
            .await
    }

    pub async fn current_diagnostics(
        &self,
        document: &DocumentState,
        timeout: Duration,
    ) -> Result<Option<DiagnosticSnapshot>> {
        if self.pull_diagnostics {
            tokio::time::sleep(timeout.min(Duration::from_millis(750))).await;
            return self.pull_diagnostics(document).await.map(Some);
        }
        Ok(self.wait_for_diagnostics(document, timeout).await)
    }

    pub async fn verify_disk_change(
        &self,
        path: &Path,
        language_id: &str,
        timeout: Duration,
    ) -> Result<SemanticVerification> {
        let before = self.diagnostics(path).await?;
        let document = self.sync_document_from_disk(path, language_id).await?;
        let Some(after) = self.current_diagnostics(&document, timeout).await? else {
            return Ok(SemanticVerification {
                status: SemanticVerificationStatus::Stale,
                document_version: Some(document.version),
                diagnostics: Vec::new(),
            });
        };
        let diagnostics = diagnostic_delta(before.as_ref(), &after);
        Ok(SemanticVerification {
            status: if !diagnostics.is_empty() {
                SemanticVerificationStatus::IssuesFound
            } else if before.is_none() {
                SemanticVerificationStatus::Stale
            } else {
                SemanticVerificationStatus::Clean
            },
            document_version: after.version,
            diagnostics,
        })
    }

    pub async fn hover(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request("textDocument/hover", path, position, None)
            .await
    }

    pub async fn definition(&self, path: &Path, position: Position) -> Result<Value> {
        let mut result = Value::Null;
        for _ in 0..60 {
            result = self
                .position_request("textDocument/definition", path, position, None)
                .await?;
            if !result.is_null() && result.as_array().is_none_or(|items| !items.is_empty()) {
                return Ok(result);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(result)
    }

    pub async fn references(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request(
            "textDocument/references",
            path,
            position,
            Some(serde_json::json!({"includeDeclaration": true})),
        )
        .await
    }

    pub async fn implementation(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request("textDocument/implementation", path, position, None)
            .await
    }

    pub async fn type_definition(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request("textDocument/typeDefinition", path, position, None)
            .await
    }

    pub async fn signature_help(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request("textDocument/signatureHelp", path, position, None)
            .await
    }

    pub async fn code_actions(&self, path: &Path, range: Range) -> Result<Value> {
        let uri = file_uri(&path.canonicalize()?)?;
        self.request_with_content_modified_retry(
            "textDocument/codeAction",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "range": range,
                "context": {"diagnostics": []}
            }),
        )
        .await
    }

    pub async fn resolve_code_action(&self, action: Value) -> Result<Value> {
        self.request_with_content_modified_retry("codeAction/resolve", action)
            .await
    }

    pub async fn will_rename_file(&self, old_path: &Path, new_path: &Path) -> Result<Value> {
        let old_uri = file_uri(&old_path.canonicalize()?)?;
        let new_uri = file_uri(new_path)?;
        match self
            .request_with_content_modified_retry(
                "workspace/willRenameFiles",
                serde_json::json!({"files": [{"oldUri": old_uri, "newUri": new_uri}]}),
            )
            .await
        {
            Err(LspError::Response { code: -32601, .. }) => Ok(Value::Null),
            result => result,
        }
    }

    pub async fn close_document(&self, path: &Path) -> Result<()> {
        self.documents.close(self.process.client(), path).await
    }

    pub async fn did_rename_file(&self, old_path: &Path, new_path: &Path) -> Result<()> {
        let old_uri = file_uri(old_path)?;
        let new_uri = file_uri(new_path)?;
        self.process
            .client()
            .notify(
                "workspace/didRenameFiles",
                Some(serde_json::json!({
                    "files": [{"oldUri": old_uri, "newUri": new_uri}]
                })),
            )
            .await
    }

    pub async fn incoming_calls(&self, path: &Path, position: Position) -> Result<Value> {
        self.call_hierarchy("callHierarchy/incomingCalls", path, position)
            .await
    }

    pub async fn outgoing_calls(&self, path: &Path, position: Position) -> Result<Value> {
        self.call_hierarchy("callHierarchy/outgoingCalls", path, position)
            .await
    }

    pub async fn prepare_rename(&self, path: &Path, position: Position) -> Result<Value> {
        for attempt in 0..60 {
            match self
                .position_request("textDocument/prepareRename", path, position, None)
                .await
            {
                Ok(value) if !value.is_null() => return Ok(value),
                Ok(value) if attempt == 59 => return Ok(value),
                Err(LspError::Response { code: -32602, .. }) if attempt < 59 => {}
                Err(error) => return Err(error),
                Ok(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        unreachable!("bounded prepareRename retry loop always returns")
    }

    pub async fn rename(&self, path: &Path, position: Position, new_name: &str) -> Result<Value> {
        let uri = file_uri(&path.canonicalize()?)?;
        let params = serde_json::json!({
            "textDocument": {"uri": uri},
            "position": position,
            "newName": new_name
        });
        for attempt in 0..60 {
            match self
                .request_with_content_modified_retry("textDocument/rename", params.clone())
                .await
            {
                Ok(value) if !value.is_null() => return Ok(value),
                Ok(value) if attempt == 59 => return Ok(value),
                Err(LspError::Response { code: -32602, .. }) if attempt < 59 => {}
                Err(error) => return Err(error),
                Ok(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        unreachable!("bounded rename retry loop always returns")
    }

    pub async fn document_symbols(&self, path: &Path) -> Result<Value> {
        let uri = file_uri(&path.canonicalize()?)?;
        self.process
            .client()
            .request(
                "textDocument/documentSymbol",
                Some(serde_json::json!({"textDocument": {"uri": uri}})),
                self.request_timeout,
            )
            .await
    }

    pub async fn workspace_symbols(&self, query: &str) -> Result<Value> {
        self.process
            .client()
            .request(
                "workspace/symbol",
                Some(serde_json::json!({"query": query})),
                self.request_timeout,
            )
            .await
    }

    async fn position_request(
        &self,
        method: &str,
        path: &Path,
        position: Position,
        context: Option<Value>,
    ) -> Result<Value> {
        let uri = file_uri(&path.canonicalize()?)?;
        let mut params = serde_json::json!({
            "textDocument": {"uri": uri},
            "position": position
        });
        if let Some(context) = context {
            params["context"] = context;
        }
        self.request_with_content_modified_retry(method, params)
            .await
    }

    async fn call_hierarchy(&self, method: &str, path: &Path, position: Position) -> Result<Value> {
        let items = self
            .position_request("textDocument/prepareCallHierarchy", path, position, None)
            .await?;
        let Some(item) = items.as_array().and_then(|items| items.first()).cloned() else {
            return Ok(serde_json::json!([]));
        };
        self.request_with_content_modified_retry(method, serde_json::json!({"item": item}))
            .await
    }

    async fn request_with_content_modified_retry(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        for attempt in 0..5 {
            match self
                .process
                .client()
                .request(method, Some(params.clone()), self.request_timeout)
                .await
            {
                Err(LspError::Response { code: -32801, .. }) if attempt < 4 => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                result => return result,
            }
        }
        unreachable!("bounded request retry loop always returns")
    }

    async fn pull_diagnostics(&self, document: &DocumentState) -> Result<DiagnosticSnapshot> {
        let value = self
            .request_with_content_modified_retry(
                "textDocument/diagnostic",
                serde_json::json!({"textDocument": {"uri": document.uri}}),
            )
            .await?;
        let items = match value.get("kind").and_then(Value::as_str) {
            Some("unchanged") => self
                .diagnostics
                .get(&document.uri)
                .await
                .map(|snapshot| snapshot.items)
                .unwrap_or_else(Vec::new),
            _ => serde_json::from_value(
                value
                    .get("items")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .map_err(|error| LspError::InvalidMessage(error.to_string()))?,
        };
        let snapshot = DiagnosticSnapshot {
            uri: document.uri.clone(),
            version: Some(document.version),
            items,
        };
        self.diagnostics.put(snapshot.clone()).await;
        Ok(snapshot)
    }
}

#[derive(Default)]
pub struct LspServicePool {
    workspaces: Mutex<HashMap<LspWorkspaceKey, Arc<LspWorkspace>>>,
    restart_states: Mutex<HashMap<LspWorkspaceKey, RestartState>>,
}

#[derive(Clone, Copy, Debug)]
struct RestartState {
    failures: u32,
    retry_at: Instant,
}

const INITIAL_RESTART_BACKOFF: Duration = Duration::from_millis(100);
const MAX_RESTART_BACKOFF: Duration = Duration::from_secs(5);

impl LspServicePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_or_start(
        &self,
        root: &Path,
        worktree_identity: impl Into<String>,
        server_id: &str,
        config: &LspConfig,
    ) -> Result<Arc<LspWorkspace>> {
        let key = LspWorkspaceKey::new(root, worktree_identity, server_id, config_digest(config)?)?;
        let mut workspaces = self.workspaces.lock().await;
        if let Some(workspace) = workspaces.get(&key) {
            if workspace.process.status().await? == ProcessStatus::Running {
                workspace.touch();
                return Ok(Arc::clone(workspace));
            }
            workspaces.remove(&key);
            self.record_restart_failure(&key).await;
        }
        self.ensure_restart_allowed(&key).await?;

        let server = config
            .servers
            .get(server_id)
            .ok_or_else(|| LspError::UnknownServer {
                server_id: server_id.to_owned(),
            })?;
        let process = match LspProcess::spawn(server, &key.canonical_root).await {
            Ok(process) => process,
            Err(error) => {
                self.record_restart_failure(&key).await;
                return Err(error);
            }
        };
        let diagnostics = DiagnosticsCache::listen(process.client().subscribe());
        let capabilities = match process
            .initialize(
                &key.canonical_root,
                Duration::from_secs(config.request_timeout_seconds),
            )
            .await
        {
            Ok(capabilities) => capabilities,
            Err(error) => {
                self.record_restart_failure(&key).await;
                return Err(error);
            }
        };
        let incremental_sync = text_sync_kind(&capabilities) == 2;
        let pull_diagnostics = capabilities
            .get("capabilities")
            .and_then(|capabilities| capabilities.get("diagnosticProvider"))
            .is_some();
        let workspace = Arc::new(LspWorkspace {
            key: key.clone(),
            process,
            capabilities,
            documents: DocumentSync::new(),
            diagnostics,
            request_timeout: Duration::from_secs(config.request_timeout_seconds),
            incremental_sync,
            pull_diagnostics,
            last_used_epoch_seconds: AtomicU64::new(epoch_seconds()),
        });
        self.restart_states.lock().await.remove(&key);
        workspaces.insert(key, Arc::clone(&workspace));
        Ok(workspace)
    }

    pub async fn get(&self, key: &LspWorkspaceKey) -> Option<Arc<LspWorkspace>> {
        self.workspaces.lock().await.get(key).cloned()
    }

    pub async fn status(
        &self,
        root: &Path,
        worktree_identity: impl Into<String>,
        server_id: &str,
        config: &LspConfig,
    ) -> Result<Option<LspWorkspaceStatus>> {
        let key = LspWorkspaceKey::new(root, worktree_identity, server_id, config_digest(config)?)?;
        let workspace = self.workspaces.lock().await.get(&key).cloned();
        let Some(workspace) = workspace else {
            return Ok(None);
        };
        Ok(Some(LspWorkspaceStatus {
            process_status: workspace.process.status().await?,
            recent_stderr: workspace.process.recent_stderr(),
        }))
    }

    pub async fn reload(
        &self,
        root: &Path,
        worktree_identity: impl Into<String>,
        server_id: &str,
        config: &LspConfig,
    ) -> Result<Arc<LspWorkspace>> {
        let worktree_identity = worktree_identity.into();
        let key = LspWorkspaceKey::new(
            root,
            worktree_identity.clone(),
            server_id,
            config_digest(config)?,
        )?;
        let previous = self.workspaces.lock().await.remove(&key);
        if let Some(previous) = previous {
            previous.process.shutdown(Duration::from_secs(2)).await;
        }
        self.restart_states.lock().await.remove(&key);
        self.get_or_start(root, worktree_identity, server_id, config)
            .await
    }

    pub async fn len(&self) -> usize {
        self.workspaces.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.workspaces.lock().await.is_empty()
    }

    pub async fn evict_idle(&self, idle_timeout: Duration) -> usize {
        let now = epoch_seconds();
        let idle_seconds = idle_timeout.as_secs();
        let evicted = {
            let mut workspaces = self.workspaces.lock().await;
            let keys = workspaces
                .iter()
                .filter(|(_, workspace)| {
                    Arc::strong_count(workspace) == 1
                        && now.saturating_sub(
                            workspace.last_used_epoch_seconds.load(Ordering::Relaxed),
                        ) >= idle_seconds
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| workspaces.remove(&key))
                .collect::<Vec<_>>()
        };
        let count = evicted.len();
        for workspace in evicted {
            workspace.process.shutdown(Duration::from_secs(2)).await;
        }
        count
    }

    pub async fn shutdown_all(&self, timeout: Duration) {
        let workspaces = {
            let mut workspaces = self.workspaces.lock().await;
            workspaces
                .drain()
                .map(|(_, workspace)| workspace)
                .collect::<Vec<_>>()
        };
        for workspace in workspaces {
            workspace.process.shutdown(timeout).await;
        }
        self.restart_states.lock().await.clear();
    }

    async fn ensure_restart_allowed(&self, key: &LspWorkspaceKey) -> Result<()> {
        let states = self.restart_states.lock().await;
        let Some(state) = states.get(key) else {
            return Ok(());
        };
        let now = Instant::now();
        if now >= state.retry_at {
            return Ok(());
        }
        let remaining = state.retry_at.saturating_duration_since(now);
        let retry_after_ms = u64::try_from(remaining.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        Err(LspError::RestartBackoff {
            server_id: key.server_id.clone(),
            retry_after_ms,
            failures: state.failures,
        })
    }

    async fn record_restart_failure(&self, key: &LspWorkspaceKey) {
        let mut states = self.restart_states.lock().await;
        let failures = states
            .get(key)
            .map(|state| state.failures.saturating_add(1))
            .unwrap_or(1);
        states.insert(
            key.clone(),
            RestartState {
                failures,
                retry_at: Instant::now() + restart_backoff(failures),
            },
        );
    }
}

fn restart_backoff(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(31);
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    INITIAL_RESTART_BACKOFF
        .checked_mul(multiplier)
        .unwrap_or(MAX_RESTART_BACKOFF)
        .min(MAX_RESTART_BACKOFF)
}

fn epoch_seconds() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs(),
        // A wall clock before the epoch has no positive timestamp.
        Err(_) => 0,
    }
}

fn text_sync_kind(initialize_result: &Value) -> u64 {
    let Some(sync) = initialize_result
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("textDocumentSync"))
    else {
        return 1;
    };
    sync.as_u64()
        .or_else(|| sync.get("change").and_then(Value::as_u64))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_keys_include_worktree_server_and_configuration_identity() {
        let root = std::env::current_dir().unwrap();
        let one = LspWorkspaceKey::new(&root, "worktree-a", "rust-analyzer", [1; 32]).unwrap();
        let same = LspWorkspaceKey::new(&root, "worktree-a", "rust-analyzer", [1; 32]).unwrap();
        let other_worktree =
            LspWorkspaceKey::new(&root, "worktree-b", "rust-analyzer", [1; 32]).unwrap();
        let other_server =
            LspWorkspaceKey::new(&root, "worktree-a", "typescript", [1; 32]).unwrap();
        let other_config =
            LspWorkspaceKey::new(&root, "worktree-a", "rust-analyzer", [2; 32]).unwrap();
        assert_eq!(one, same);
        assert_ne!(one, other_worktree);
        assert_ne!(one, other_server);
        assert_ne!(one, other_config);
    }

    #[test]
    fn text_sync_kind_supports_numeric_and_option_shapes() {
        assert_eq!(
            text_sync_kind(&serde_json::json!({"capabilities": {"textDocumentSync": 2}})),
            2
        );
        assert_eq!(
            text_sync_kind(
                &serde_json::json!({"capabilities": {"textDocumentSync": {"change": 2}}})
            ),
            2
        );
        assert_eq!(text_sync_kind(&serde_json::json!({"capabilities": {}})), 1);
    }

    #[test]
    fn restart_backoff_is_exponential_and_bounded() {
        assert_eq!(restart_backoff(1), Duration::from_millis(100));
        assert_eq!(restart_backoff(2), Duration::from_millis(200));
        assert_eq!(restart_backoff(3), Duration::from_millis(400));
        assert_eq!(restart_backoff(7), Duration::from_secs(5));
        assert_eq!(restart_backoff(u32::MAX), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn repeated_start_failures_are_throttled() {
        let root = tempfile::tempdir().unwrap();
        let mut config = LspConfig::default();
        config.servers.get_mut("rust-analyzer").unwrap().command =
            "jcode-lsp-command-that-does-not-exist".to_owned();
        let pool = LspServicePool::new();

        assert!(matches!(
            pool.get_or_start(root.path(), "fixture", "rust-analyzer", &config)
                .await,
            Err(LspError::ExecutableNotFound { .. })
        ));
        assert!(matches!(
            pool.get_or_start(root.path(), "fixture", "rust-analyzer", &config)
                .await,
            Err(LspError::RestartBackoff {
                failures: 1,
                retry_after_ms: 1..=100,
                ..
            })
        ));
        tokio::time::sleep(Duration::from_millis(110)).await;
        assert!(matches!(
            pool.get_or_start(root.path(), "fixture", "rust-analyzer", &config)
                .await,
            Err(LspError::ExecutableNotFound { .. })
        ));
        assert!(matches!(
            pool.get_or_start(root.path(), "fixture", "rust-analyzer", &config)
                .await,
            Err(LspError::RestartBackoff {
                failures: 2,
                retry_after_ms: 1..=200,
                ..
            })
        ));
    }
}
