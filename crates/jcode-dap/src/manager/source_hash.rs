use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use tokio::time::Instant;

use super::SessionEntry;
use crate::{DapError, DebugOperationConfig, DebugSourceRevision, DebugWorkspaceKey, Result};

pub(super) struct ResolvedSource {
    pub original: PathBuf,
    pub canonical: PathBuf,
    pub wire_path: String,
    pub relative: PathBuf,
    pub revision: DebugSourceRevision,
}

pub(super) async fn resolve_source(
    workspace: &DebugWorkspaceKey,
    entry: &SessionEntry,
    path: &Path,
    operations: &DebugOperationConfig,
    deadline: Instant,
) -> Result<ResolvedSource> {
    let encoded = path.as_os_str().as_encoded_bytes();
    if encoded.contains(&0) || encoded.len() > operations.max_source_path_bytes {
        return Err(DapError::InvalidDebugSource {
            path: path.to_path_buf(),
            message: "source path exceeds configured byte limit".to_owned(),
        });
    }
    let root = workspace.canonical_root().to_path_buf();
    let input = path.to_path_buf();
    let original = input.clone();
    let limit = operations.max_source_revision_bytes;
    let path_limit = operations.max_source_path_bytes;
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = Arc::clone(&cancelled);
    let permit = tokio::time::timeout_at(deadline, Arc::clone(&entry.source_hash).acquire_owned())
        .await
        .map_err(|_| DapError::RequestTimeout {
            command: "source revision".to_owned(),
        })?
        .map_err(|_| DapError::TransportClosed)?;
    let mut task = tokio::task::spawn_blocking(move || -> Result<_> {
        let _permit = permit;
        if task_cancelled.load(Ordering::Acquire) {
            return Err(DapError::RequestTimeout {
                command: "source revision".to_owned(),
            });
        }
        let candidate = if input.is_absolute() {
            input
        } else {
            root.join(input)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|error| DapError::InvalidDebugSource {
                path: candidate.clone(),
                message: error.to_string(),
            })?;
        if !canonical.starts_with(&root) {
            return Err(DapError::DebugSourceOutsideWorkspace {
                path: canonical,
                workspace: root,
            });
        }
        if !canonical.is_file() {
            return Err(DapError::InvalidDebugSource {
                path: canonical,
                message: "source is not a regular file".to_owned(),
            });
        }
        let relative = canonical
            .strip_prefix(&root)
            .map_err(|_| DapError::DebugSourceOutsideWorkspace {
                path: canonical.clone(),
                workspace: root.clone(),
            })?
            .to_path_buf();
        let wire_path = canonical
            .to_str()
            .ok_or_else(|| DapError::InvalidDebugSource {
                path: canonical.clone(),
                message: "canonical source path is not valid UTF-8".to_owned(),
            })?
            .to_owned();
        if wire_path.as_bytes().contains(&0) || wire_path.len() > path_limit {
            return Err(DapError::InvalidDebugSource {
                path: canonical,
                message: "canonical source path exceeds configured byte limit or contains NUL"
                    .to_owned(),
            });
        }
        let revision = hash_file(&canonical, limit, &task_cancelled)?;
        Ok((canonical, wire_path, relative, revision))
    });
    let joined = tokio::time::timeout_at(deadline, &mut task).await;
    let (canonical, wire_path, relative, revision) = match joined {
        Ok(joined) => joined.map_err(|error| DapError::DebugOperationTaskFailed {
            operation: "source revision",
            message: error.to_string(),
        })??,
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            return Err(DapError::RequestTimeout {
                command: "source revision".to_owned(),
            });
        }
    };
    Ok(ResolvedSource {
        original,
        canonical,
        wire_path,
        relative,
        revision,
    })
}

fn hash_file(path: &Path, limit: u64, cancelled: &AtomicBool) -> Result<DebugSourceRevision> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(DapError::RequestTimeout {
                command: "source revision".to_owned(),
            });
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        if bytes > limit {
            return Err(DapError::DebugSourceTooLarge {
                path: path.to_path_buf(),
                observed: bytes,
                limit,
            });
        }
        hasher.update(&buffer[..read]);
    }
    Ok(DebugSourceRevision {
        sha256: hasher.finalize().into(),
        byte_len: bytes,
    })
}
