use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::document_sync::file_uri;
use crate::{
    DiagnosticSnapshot, DiagnosticsCache, DocumentState, DocumentSync, LspConfig, LspError,
    LspProcess, Position, ProcessStatus, Result, config_digest,
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

    pub async fn hover(&self, path: &Path, position: Position) -> Result<Value> {
        self.position_request("textDocument/hover", path, position, None)
            .await
    }

    pub async fn definition(&self, path: &Path, position: Position) -> Result<Value> {
        let mut result = Value::Null;
        for _ in 0..20 {
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
}

#[derive(Default)]
pub struct LspServicePool {
    workspaces: Mutex<HashMap<LspWorkspaceKey, Arc<LspWorkspace>>>,
}

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
                return Ok(Arc::clone(workspace));
            }
            workspaces.remove(&key);
        }

        let server = config
            .servers
            .get(server_id)
            .ok_or_else(|| LspError::UnknownServer {
                server_id: server_id.to_owned(),
            })?;
        let process = LspProcess::spawn(server, &key.canonical_root).await?;
        let diagnostics = DiagnosticsCache::listen(process.client().subscribe());
        let capabilities = process
            .initialize(
                &key.canonical_root,
                Duration::from_secs(config.request_timeout_seconds),
            )
            .await?;
        let incremental_sync = text_sync_kind(&capabilities) == 2;
        let workspace = Arc::new(LspWorkspace {
            key: key.clone(),
            process,
            capabilities,
            documents: DocumentSync::new(),
            diagnostics,
            request_timeout: Duration::from_secs(config.request_timeout_seconds),
            incremental_sync,
        });
        workspaces.insert(key, Arc::clone(&workspace));
        Ok(workspace)
    }

    pub async fn get(&self, key: &LspWorkspaceKey) -> Option<Arc<LspWorkspace>> {
        self.workspaces.lock().await.get(key).cloned()
    }

    pub async fn len(&self) -> usize {
        self.workspaces.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.workspaces.lock().await.is_empty()
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
}
