//! Shared ledger preflight and post-write metadata for legacy mutation tools.
//!
//! The transaction lock closes races between ledger-aware tools. Multi-file
//! patch tools precompute every result before their first mutation, but still
//! publish files sequentially. An I/O failure during publication can therefore
//! leave a partial filesystem update; the scoped Phase 1 contract documents
//! that remaining gap rather than claiming full atomicity.

use super::ToolContext;
use crate::server::{FileSnapshotLedger, SessionReadFreshness, SnapshotRecord};
use anyhow::{Context, Result, bail};
use jcode_edit_core::{LineRange, validate_relative_path};
use jcode_edit_types::{FileRevision, ReadSnapshot};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::sync::OwnedMutexGuard;

#[derive(Clone)]
pub(crate) struct FileWriteGuard {
    ledger: FileSnapshotLedger,
    policy_override: Option<crate::config::ReadGuardConfig>,
}

#[derive(Clone, Debug)]
pub(crate) enum RequiredCoverage {
    Ranges(Vec<LineRange>),
    FullFile,
}

#[derive(Clone, Debug)]
pub(crate) struct GuardedFile {
    pub(crate) relative_path: String,
    pub(crate) revision_before: Option<FileRevision>,
    warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedWrite {
    pub(crate) path: String,
    pub(crate) revision_before: Option<FileRevision>,
    pub(crate) record: SnapshotRecord,
}

pub(crate) struct FileWriteTransaction {
    guard: FileWriteGuard,
    workspace_root: PathBuf,
    session_id: String,
    _lock: OwnedMutexGuard<()>,
}

impl FileWriteGuard {
    pub(crate) fn new(ledger: FileSnapshotLedger) -> Self {
        Self {
            ledger,
            policy_override: None,
        }
    }

    pub(crate) fn with_policy(
        ledger: FileSnapshotLedger,
        policy: crate::config::ReadGuardConfig,
    ) -> Self {
        Self {
            ledger,
            policy_override: Some(policy),
        }
    }

    pub(crate) async fn begin(&self, ctx: &ToolContext) -> Result<FileWriteTransaction> {
        let root = ctx
            .working_dir
            .as_deref()
            .context("ledger-backed file writes require a workspace working directory")?;
        let workspace_root = std::fs::canonicalize(root)
            .with_context(|| format!("failed to canonicalize workspace {}", root.display()))?;
        let lock = self.ledger.lock_write_transaction().await;
        Ok(FileWriteTransaction {
            guard: self.clone(),
            workspace_root,
            session_id: ctx.session_id.clone(),
            _lock: lock,
        })
    }
}

impl FileWriteTransaction {
    pub(crate) async fn preflight_existing(
        &self,
        path: &Path,
        contents: &[u8],
        coverage: RequiredCoverage,
    ) -> Result<GuardedFile> {
        let relative_path = self.relative_path(path)?;
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        let current = self
            .guard
            .ledger
            .observe_text(
                &self.workspace_root,
                &relative_path,
                contents,
                modified_ns(&metadata),
            )
            .await?;
        let policy = self
            .guard
            .policy_override
            .clone()
            .unwrap_or_else(|| crate::config::config().editing.read_guard.clone());
        let warnings = if policy.mode == crate::config::ReadGuardMode::Off {
            Vec::new()
        } else {
            let freshness = self
                .guard
                .ledger
                .check_session_read(&self.session_id, &self.workspace_root, &relative_path)
                .await?;
            guard_violations(&relative_path, &policy, freshness, coverage, contents)?
        };

        if policy.mode == crate::config::ReadGuardMode::Block && !warnings.is_empty() {
            bail!(
                "Overwrite rejected: {}\nNo bytes were written.",
                warnings.join("\n")
            );
        }

        Ok(GuardedFile {
            relative_path,
            revision_before: Some(current.revision),
            warnings,
        })
    }

    pub(crate) fn prepare_new(&self, path: &Path) -> Result<GuardedFile> {
        Ok(GuardedFile {
            relative_path: self.relative_path(path)?,
            revision_before: None,
            warnings: Vec::new(),
        })
    }

    pub(crate) async fn record_success(
        &self,
        file: GuardedFile,
        path: &Path,
        contents: &[u8],
    ) -> Result<RecordedWrite> {
        let mtime_ns = match std::fs::metadata(path) {
            Ok(metadata) => modified_ns(&metadata),
            Err(_) => {
                crate::logging::warn(
                    "Written file metadata unavailable; recording content revision only",
                );
                None
            }
        };
        let record = self
            .guard
            .ledger
            .record_write(
                &self.session_id,
                &self.workspace_root,
                &file.relative_path,
                contents,
                mtime_ns,
            )
            .await?;
        Ok(RecordedWrite {
            path: file.relative_path,
            revision_before: file.revision_before,
            record,
        })
    }

    fn relative_path(&self, path: &Path) -> Result<String> {
        let absolute = if path.exists() {
            std::fs::canonicalize(path)
                .with_context(|| format!("failed to canonicalize {}", path.display()))?
        } else {
            canonicalize_nonexistent(path)?
        };
        if !absolute.starts_with(&self.workspace_root) {
            bail!("file path escapes the ledger workspace: {}", path.display());
        }
        let relative = absolute
            .strip_prefix(&self.workspace_root)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        validate_relative_path(&relative)?;
        Ok(relative)
    }
}

fn canonicalize_nonexistent(path: &Path) -> Result<PathBuf> {
    let mut cursor = path;
    let mut suffix = Vec::new();
    while !cursor.exists() {
        suffix.push(
            cursor
                .file_name()
                .context("file path has no name")?
                .to_owned(),
        );
        cursor = cursor
            .parent()
            .context("file path has no existing ancestor")?;
    }
    let mut canonical = std::fs::canonicalize(cursor)
        .with_context(|| format!("failed to canonicalize {}", cursor.display()))?;
    for component in suffix.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

impl GuardedFile {
    pub(crate) fn append_warnings(&self, output: &mut String) {
        for warning in &self.warnings {
            output.push_str("\n\nWarning: overwrite guard: ");
            output.push_str(warning);
        }
    }
}

pub(crate) fn metadata_for_writes(writes: &[RecordedWrite]) -> Value {
    json!({
        "files": writes.iter().map(|write| json!({
            "path": write.path,
            "revision_before": write.revision_before,
            "revision_after": write.record.revision,
            "writer_session_id": write.record.writer_session_id,
        })).collect::<Vec<_>>()
    })
}

pub(crate) fn full_file_range(contents: &str) -> Vec<LineRange> {
    let line_count = normalized_line_count(contents.as_bytes());
    if line_count == 0 {
        Vec::new()
    } else {
        vec![LineRange {
            start: 1,
            end: line_count,
        }]
    }
}

pub(crate) fn matched_ranges(contents: &str, needle: &str, replace_all: bool) -> Vec<LineRange> {
    let mut ranges = Vec::new();
    let line_count = normalized_line_count(contents.as_bytes()).max(1);
    for (index, _) in contents.match_indices(needle) {
        let start = contents[..index]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u64
            + 1;
        let end =
            (start + needle.bytes().filter(|byte| *byte == b'\n').count() as u64).min(line_count);
        ranges.push(LineRange { start, end });
        if !replace_all {
            break;
        }
    }
    ranges
}

fn guard_violations(
    path: &str,
    policy: &crate::config::ReadGuardConfig,
    freshness: SessionReadFreshness,
    coverage: RequiredCoverage,
    contents: &[u8],
) -> Result<Vec<String>> {
    let mut violations = Vec::new();
    let read = match freshness {
        SessionReadFreshness::NoRead => {
            if policy.require_same_revision || policy.require_covered_ranges {
                violations.push(format!("{path} was not read in this session"));
            }
            None
        }
        SessionReadFreshness::MissingCurrent { .. } => {
            if policy.require_same_revision {
                violations.push(format!("{path} has no current ledger revision"));
            }
            if policy.require_covered_ranges {
                violations.push(format!(
                    "{path} read coverage is not tied to a current revision"
                ));
            }
            None
        }
        SessionReadFreshness::Current { read } => Some(read),
        SessionReadFreshness::Stale { read, current } => {
            if policy.require_same_revision {
                let writer = current
                    .writer_session_id
                    .as_deref()
                    .map(|session| format!(", peer {session}"))
                    .unwrap_or_else(String::new);
                violations.push(format!(
                    "{path} changed after your read (rev {} -> {}{writer})",
                    read.revision.revision, current.revision.revision
                ));
            }
            if policy.require_covered_ranges && !policy.require_same_revision {
                violations.push(format!("{path} read coverage belongs to a stale revision"));
            }
            None
        }
    };

    if policy.require_covered_ranges {
        if let Some(read) = read.as_ref() {
            match coverage {
                RequiredCoverage::FullFile => {
                    if !policy.allow_full_file_write {
                        violations.push(format!("{path} is a whole-file overwrite"));
                    } else if !read.full_file {
                        violations.push(format!("{path} was not read in full"));
                    }
                }
                RequiredCoverage::Ranges(required) => {
                    validate_ranges(path, read, &required, normalized_line_count(contents))
                        .map_err(anyhow::Error::msg)?;
                    if !read.full_file {
                        for range in required {
                            if !read.ranges.iter().any(|covered| {
                                covered.start <= range.start && covered.end >= range.end
                            }) {
                                violations.push(format!(
                                    "{path} lines {}-{} were not covered by the session read",
                                    range.start, range.end
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    violations.sort();
    violations.dedup();
    Ok(violations)
}

fn validate_ranges(
    path: &str,
    read: &ReadSnapshot,
    required: &[LineRange],
    line_count: u64,
) -> std::result::Result<(), String> {
    for range in read.ranges.iter().chain(required) {
        if range.start == 0 || range.start > range.end || range.end > line_count {
            return Err(format!(
                "invalid overwrite guard range {}-{} for {path} ({line_count} lines)",
                range.start, range.end
            ));
        }
    }
    Ok(())
}

fn normalized_line_count(contents: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(contents);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.is_empty() {
        0
    } else {
        text.lines().count() as u64
    }
}

fn modified_ns(metadata: &std::fs::Metadata) -> Option<u128> {
    match metadata.modified() {
        Ok(modified) => match modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => Some(duration.as_nanos()),
            Err(_) => None, // Pre-epoch timestamps cannot be represented; content revisions still guard writes.
        },
        Err(_) => {
            crate::logging::debug(
                "File modification time unavailable; using content revision only",
            );
            None
        }
    }
}
