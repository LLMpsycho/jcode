use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::{LspConfig, LspError, LspProcess, ProcessStatus, Result, config_digest};

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
        let capabilities = process
            .initialize(
                &key.canonical_root,
                Duration::from_secs(config.request_timeout_seconds),
            )
            .await?;
        let workspace = Arc::new(LspWorkspace {
            key: key.clone(),
            process,
            capabilities,
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
}
