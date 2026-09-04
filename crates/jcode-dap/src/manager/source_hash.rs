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
    let metadata_len = file.metadata()?.len();
    if metadata_len > limit {
        return Err(DapError::DebugSourceTooLarge {
            path: path.to_path_buf(),
            observed: metadata_len,
            limit,
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> [u8; 32] {
        let mut result = [0_u8; 32];
        for (index, byte) in result.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        result
    }

    #[test]
    fn exact_source_digest_distinguishes_lf_crlf_bom_whitespace_and_byte_changes() {
        let root = std::env::temp_dir().join(format!(
            "jcode-dap-source-hash-{}-{}",
            std::process::id(),
            crate::session::next_manager_id().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("source.rs");
        let cases = [
            (
                b"a\n".as_slice(),
                "87428fc522803d31065e7bce3cf03fe475096631e5e07bbd7a0fde60c4cf25c7",
            ),
            (
                b"a\r\n".as_slice(),
                "8e4621379786ef42a4fec155cd525c291dd7db3c1fde3478522f4f61c03fd1bd",
            ),
            (
                b"\xef\xbb\xbfa\n".as_slice(),
                "be4fccb045869c7ad387b9081a44cfd495b37efc020da4b44542de0d980c747f",
            ),
            (
                b"a \n".as_slice(),
                "19ae96e7938ec564866a1bb552e51bc3f1b9aa32c5221f3e8c3a75d0080c7004",
            ),
            (
                b"b\n".as_slice(),
                "0263829989b6fd954f72baaf2fc64bc2e2f01d692d4de72986ea808f6e99813f",
            ),
        ];
        for (bytes, digest) in cases {
            std::fs::write(&path, bytes).unwrap();
            let revision = hash_file(&path, u64::MAX, &AtomicBool::new(false)).unwrap();
            assert_eq!(revision.byte_len, u64::try_from(bytes.len()).unwrap());
            assert_eq!(revision.sha256, decode_hex(digest));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_hash_byte_limit_accepts_boundary_and_rejects_boundary_plus_one() {
        let root = std::env::temp_dir().join(format!(
            "jcode-dap-source-limit-{}-{}",
            std::process::id(),
            crate::session::next_manager_id().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("source.rs");
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(
            hash_file(&path, 5, &AtomicBool::new(false))
                .unwrap()
                .byte_len,
            5
        );
        assert!(matches!(
            hash_file(&path, 4, &AtomicBool::new(false)),
            Err(DapError::DebugSourceTooLarge {
                observed: 5,
                limit: 4,
                ..
            })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opened_file_metadata_length_accepts_boundary_and_rejects_boundary_plus_one() {
        let root = std::env::temp_dir().join(format!(
            "jcode-dap-source-metadata-{}-{}",
            std::process::id(),
            crate::session::next_manager_id().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("source.rs");
        std::fs::write(&path, b"1234").unwrap();
        assert_eq!(
            hash_file(&path, 4, &AtomicBool::new(false))
                .unwrap()
                .byte_len,
            4
        );
        assert!(matches!(
            hash_file(&path, 3, &AtomicBool::new(false)),
            Err(DapError::DebugSourceTooLarge {
                observed: 4,
                limit: 3,
                ..
            })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
