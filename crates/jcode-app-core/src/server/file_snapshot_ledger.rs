//! Server-owned file revision and per-session read coverage ledger.
//!
//! The ledger is intentionally independent from tool input/output. It provides
//! the shared revision substrate that later read and write tool slices can call
//! without introducing process-global state.

// Some write-side APIs intentionally remain unused until the next Phase 1 slice.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use jcode_edit_core::{EditError, file_revision, normalize_bytes, validate_relative_path};
use jcode_edit_types::{FileRevision, LineRange, ReadSnapshot};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

/// Stable identity for a file inside one canonical workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SnapshotKey {
    pub(crate) workspace_root: PathBuf,
    pub(crate) relative_path: String,
}

/// How the current ledger revision was minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotSource {
    Observation,
    Write,
}

/// Current server-owned metadata for one normalized text file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRecord {
    pub(crate) key: SnapshotKey,
    pub(crate) revision: FileRevision,
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) writer_session_id: Option<String>,
    pub(crate) source: SnapshotSource,
}

/// Result of comparing expected strong revision metadata with the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotFreshness {
    Missing,
    Current(SnapshotRecord),
    Stale {
        expected: FileRevision,
        current: SnapshotRecord,
    },
}

/// Result of comparing a session's last read with the current ledger record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionReadFreshness {
    NoRead,
    MissingCurrent {
        read: ReadSnapshot,
    },
    Current {
        read: ReadSnapshot,
    },
    Stale {
        read: ReadSnapshot,
        current: SnapshotRecord,
    },
}

/// Counts returned by expiry so callers can account for reclaimed state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpiryReport {
    pub(crate) snapshots: usize,
    pub(crate) session_reads: usize,
}

/// Exact line exposure supplied when a text read is registered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReadCoverage {
    pub(crate) ranges: Vec<LineRange>,
    pub(crate) full_file: bool,
}

/// Errors produced before any ledger state is changed.
#[derive(Debug)]
pub(crate) enum FileSnapshotLedgerError {
    WorkspaceCanonicalization {
        workspace_root: PathBuf,
        source: std::io::Error,
    },
    Edit(EditError),
    InvalidCoverage {
        path: String,
        range: LineRange,
        line_count: u64,
    },
    RevisionOverflow {
        path: String,
    },
}

impl fmt::Display for FileSnapshotLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkspaceCanonicalization {
                workspace_root,
                source,
            } => write!(
                formatter,
                "failed to canonicalize workspace {}: {source}",
                workspace_root.display()
            ),
            Self::Edit(error) => write!(formatter, "{error}"),
            Self::InvalidCoverage {
                path,
                range,
                line_count,
            } => write!(
                formatter,
                "read coverage {}..={} is outside {path}'s {line_count} lines",
                range.start, range.end
            ),
            Self::RevisionOverflow { path } => {
                write!(formatter, "file revision overflow for {path}")
            }
        }
    }
}

impl std::error::Error for FileSnapshotLedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WorkspaceCanonicalization { source, .. } => Some(source),
            Self::Edit(error) => Some(error),
            Self::InvalidCoverage { .. } | Self::RevisionOverflow { .. } => None,
        }
    }
}

impl From<EditError> for FileSnapshotLedgerError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

#[derive(Clone)]
pub(crate) struct FileSnapshotLedger {
    state: Arc<RwLock<LedgerState>>,
    write_transaction: Arc<Mutex<()>>,
}

#[derive(Clone)]
pub(crate) struct SnapshotWrite {
    pub(crate) relative_path: String,
    pub(crate) expected_revision: FileRevision,
    pub(crate) contents: Vec<u8>,
    pub(crate) mtime_ns: Option<u128>,
}

#[derive(Clone)]
pub(crate) struct SnapshotMove {
    pub(crate) source_relative_path: String,
    pub(crate) expected_revision: FileRevision,
    pub(crate) destination_relative_path: String,
    pub(crate) contents: Vec<u8>,
    pub(crate) mtime_ns: Option<u128>,
}

#[derive(Default)]
struct LedgerState {
    snapshots: HashMap<SnapshotKey, SnapshotEntry>,
    reads_by_session: HashMap<String, HashMap<SnapshotKey, SessionReadEntry>>,
}

struct SnapshotEntry {
    record: SnapshotRecord,
    line_count: u64,
    last_touched: Instant,
}

struct SessionReadEntry {
    snapshot: ReadSnapshot,
    last_touched: Instant,
}

struct PreparedObservation {
    key: SnapshotKey,
    normalized_text: String,
    line_count: u64,
    mtime_ns: Option<u128>,
}

impl FileSnapshotLedger {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(LedgerState::default())),
            write_transaction: Arc::new(Mutex::new(())),
        }
    }

    /// Serialize ledger-aware write transactions so two anchored edits cannot
    /// both preflight the same revision and then race to publish it.
    pub(crate) async fn lock_write_transaction(&self) -> OwnedMutexGuard<()> {
        self.write_transaction.clone().lock_owned().await
    }

    /// Register text observed from disk. Repeated observations of the same
    /// normalized bytes keep their revision; changed bytes mint the next one.
    pub(crate) async fn observe_text(
        &self,
        workspace_root: &Path,
        relative_path: &str,
        contents: &[u8],
        mtime_ns: Option<u128>,
    ) -> Result<SnapshotRecord, FileSnapshotLedgerError> {
        let prepared = prepare_observation(workspace_root, relative_path, contents, mtime_ns)?;
        let mut state = self.state.write().await;
        upsert_snapshot(
            &mut state,
            prepared,
            SnapshotSource::Observation,
            None,
            false,
        )
    }

    /// Atomically register an observation and the exact lines exposed to a
    /// session. Additional reads at the same revision merge their ranges.
    pub(crate) async fn record_read(
        &self,
        session_id: &str,
        workspace_root: &Path,
        relative_path: &str,
        contents: &[u8],
        mtime_ns: Option<u128>,
        coverage: ReadCoverage,
    ) -> Result<ReadSnapshot, FileSnapshotLedgerError> {
        let prepared = prepare_observation(workspace_root, relative_path, contents, mtime_ns)?;
        validate_ranges(relative_path, &coverage.ranges, prepared.line_count)?;

        let mut state = self.state.write().await;
        let record = upsert_snapshot(
            &mut state,
            prepared,
            SnapshotSource::Observation,
            None,
            false,
        )?;
        let now = Instant::now();
        let session_reads = state
            .reads_by_session
            .entry(session_id.to_owned())
            .or_default();
        let read = session_reads
            .entry(record.key.clone())
            .or_insert_with(|| SessionReadEntry {
                snapshot: ReadSnapshot {
                    path: record.key.relative_path.clone(),
                    revision: record.revision.clone(),
                    ranges: Vec::new(),
                    full_file: false,
                },
                last_touched: now,
            });
        if !same_strong_revision(&read.snapshot.revision, &record.revision) {
            read.snapshot = ReadSnapshot {
                path: record.key.relative_path.clone(),
                revision: record.revision.clone(),
                ranges: Vec::new(),
                full_file: false,
            };
        }
        read.snapshot.ranges.extend(coverage.ranges);
        merge_ranges(&mut read.snapshot.ranges);
        read.snapshot.full_file |= coverage.full_file;
        read.last_touched = now;
        Ok(read.snapshot.clone())
    }

    /// Record a successful write. Every write remints the revision, even when
    /// the normalized bytes happen to be unchanged.
    pub(crate) async fn record_write(
        &self,
        session_id: &str,
        workspace_root: &Path,
        relative_path: &str,
        contents: &[u8],
        mtime_ns: Option<u128>,
    ) -> Result<SnapshotRecord, FileSnapshotLedgerError> {
        let prepared = prepare_observation(workspace_root, relative_path, contents, mtime_ns)?;
        let mut state = self.state.write().await;
        upsert_snapshot(
            &mut state,
            prepared,
            SnapshotSource::Write,
            Some(session_id.to_owned()),
            true,
        )
    }

    /// Atomically update all ledger records after a multi-file publication.
    /// Every expected revision is checked before any ledger entry is changed.
    pub(crate) async fn record_writes(
        &self,
        session_id: &str,
        workspace_root: &Path,
        writes: Vec<SnapshotWrite>,
    ) -> Result<Vec<SnapshotRecord>, FileSnapshotLedgerError> {
        let mut prepared = Vec::with_capacity(writes.len());
        let mut seen = HashSet::with_capacity(writes.len());
        for write in writes {
            let observation = prepare_observation(
                workspace_root,
                &write.relative_path,
                &write.contents,
                write.mtime_ns,
            )?;
            if !seen.insert(observation.key.clone()) {
                return Err(EditError::DuplicateObservedFile {
                    path: write.relative_path,
                }
                .into());
            }
            prepared.push((observation, write.expected_revision));
        }

        let mut state = self.state.write().await;
        for (observation, expected) in &prepared {
            let Some(current) = state.snapshots.get(&observation.key) else {
                return Err(EditError::StaleRevision {
                    path: observation.key.relative_path.clone(),
                    expected: expected.revision,
                    actual: 0,
                }
                .into());
            };
            if !same_strong_revision(expected, &current.record.revision) {
                return Err(EditError::StaleRevision {
                    path: observation.key.relative_path.clone(),
                    expected: expected.revision,
                    actual: current.record.revision.revision,
                }
                .into());
            }
            expected.revision.checked_add(1).ok_or_else(|| {
                FileSnapshotLedgerError::RevisionOverflow {
                    path: observation.key.relative_path.clone(),
                }
            })?;
        }

        prepared
            .into_iter()
            .map(|(observation, _)| {
                upsert_snapshot(
                    &mut state,
                    observation,
                    SnapshotSource::Write,
                    Some(session_id.to_owned()),
                    true,
                )
            })
            .collect()
    }

    /// Atomically replace one path identity with another while recording any
    /// related text edits. The source revision and every edited file are
    /// checked before ledger state changes.
    pub(crate) async fn record_move_with_writes(
        &self,
        session_id: &str,
        workspace_root: &Path,
        movement: SnapshotMove,
        writes: Vec<SnapshotWrite>,
    ) -> Result<(SnapshotRecord, Vec<SnapshotRecord>), FileSnapshotLedgerError> {
        let source_key = snapshot_key(workspace_root, &movement.source_relative_path)?;
        let destination = prepare_observation(
            workspace_root,
            &movement.destination_relative_path,
            &movement.contents,
            movement.mtime_ns,
        )?;
        if source_key == destination.key {
            return Err(EditError::DuplicateObservedFile {
                path: movement.destination_relative_path,
            }
            .into());
        }

        let mut prepared_writes = Vec::with_capacity(writes.len());
        let mut seen = HashSet::from([source_key.clone(), destination.key.clone()]);
        for write in writes {
            let observation = prepare_observation(
                workspace_root,
                &write.relative_path,
                &write.contents,
                write.mtime_ns,
            )?;
            if !seen.insert(observation.key.clone()) {
                return Err(EditError::DuplicateObservedFile {
                    path: write.relative_path,
                }
                .into());
            }
            prepared_writes.push((observation, write.expected_revision));
        }

        let mut state = self.state.write().await;
        require_current_revision(&state, &source_key, &movement.expected_revision)?;
        for (observation, expected) in &prepared_writes {
            require_current_revision(&state, &observation.key, expected)?;
            expected.revision.checked_add(1).ok_or_else(|| {
                FileSnapshotLedgerError::RevisionOverflow {
                    path: observation.key.relative_path.clone(),
                }
            })?;
        }
        if let Some(current) = state.snapshots.get(&destination.key) {
            current
                .record
                .revision
                .revision
                .checked_add(1)
                .ok_or_else(|| FileSnapshotLedgerError::RevisionOverflow {
                    path: destination.key.relative_path.clone(),
                })?;
        }

        state.snapshots.remove(&source_key);
        state.reads_by_session.retain(|_, reads| {
            reads.remove(&source_key);
            !reads.is_empty()
        });
        let destination_record = upsert_snapshot(
            &mut state,
            destination,
            SnapshotSource::Write,
            Some(session_id.to_owned()),
            true,
        )?;
        let write_records = prepared_writes
            .into_iter()
            .map(|(observation, _)| {
                upsert_snapshot(
                    &mut state,
                    observation,
                    SnapshotSource::Write,
                    Some(session_id.to_owned()),
                    true,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((destination_record, write_records))
    }

    pub(crate) async fn snapshot(
        &self,
        workspace_root: &Path,
        relative_path: &str,
    ) -> Result<Option<SnapshotRecord>, FileSnapshotLedgerError> {
        let key = snapshot_key(workspace_root, relative_path)?;
        Ok(self
            .state
            .read()
            .await
            .snapshots
            .get(&key)
            .map(|entry| entry.record.clone()))
    }

    pub(crate) async fn session_read(
        &self,
        session_id: &str,
        workspace_root: &Path,
        relative_path: &str,
    ) -> Result<Option<ReadSnapshot>, FileSnapshotLedgerError> {
        let key = snapshot_key(workspace_root, relative_path)?;
        Ok(self
            .state
            .read()
            .await
            .reads_by_session
            .get(session_id)
            .and_then(|reads| reads.get(&key))
            .map(|entry| entry.snapshot.clone()))
    }

    /// Compare both the monotonic revision and full digest. The short display
    /// tag is deliberately excluded from correctness decisions.
    pub(crate) async fn check_revision(
        &self,
        workspace_root: &Path,
        relative_path: &str,
        expected: &FileRevision,
    ) -> Result<SnapshotFreshness, FileSnapshotLedgerError> {
        let current = self.snapshot(workspace_root, relative_path).await?;
        Ok(match current {
            None => SnapshotFreshness::Missing,
            Some(current) if same_strong_revision(expected, &current.revision) => {
                SnapshotFreshness::Current(current)
            }
            Some(current) => SnapshotFreshness::Stale {
                expected: expected.clone(),
                current,
            },
        })
    }

    pub(crate) async fn check_session_read(
        &self,
        session_id: &str,
        workspace_root: &Path,
        relative_path: &str,
    ) -> Result<SessionReadFreshness, FileSnapshotLedgerError> {
        let key = snapshot_key(workspace_root, relative_path)?;
        let state = self.state.read().await;
        let Some(read) = state
            .reads_by_session
            .get(session_id)
            .and_then(|reads| reads.get(&key))
            .map(|entry| entry.snapshot.clone())
        else {
            return Ok(SessionReadFreshness::NoRead);
        };
        let Some(current) = state.snapshots.get(&key).map(|entry| entry.record.clone()) else {
            return Ok(SessionReadFreshness::MissingCurrent { read });
        };
        if same_strong_revision(&read.revision, &current.revision) {
            Ok(SessionReadFreshness::Current { read })
        } else {
            Ok(SessionReadFreshness::Stale { read, current })
        }
    }

    /// Remove only session-owned read coverage. Shared file revisions survive.
    pub(crate) async fn clear_session(&self, session_id: &str) -> usize {
        self.state
            .write()
            .await
            .reads_by_session
            .remove(session_id)
            .map_or(0, |reads| reads.len())
    }

    /// Remove all snapshots and read coverage belonging to one workspace.
    pub(crate) async fn clear_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<ExpiryReport, FileSnapshotLedgerError> {
        let canonical_root = canonical_workspace_root(workspace_root)?;
        let mut state = self.state.write().await;
        let snapshots_before = state.snapshots.len();
        state
            .snapshots
            .retain(|key, _| key.workspace_root != canonical_root);
        let snapshots = snapshots_before - state.snapshots.len();

        let mut session_reads = 0;
        state.reads_by_session.retain(|_, reads| {
            let before = reads.len();
            reads.retain(|key, _| key.workspace_root != canonical_root);
            session_reads += before - reads.len();
            !reads.is_empty()
        });
        Ok(ExpiryReport {
            snapshots,
            session_reads,
        })
    }

    /// Expire idle snapshots and read coverage. Coverage is also removed when
    /// its corresponding shared snapshot expires.
    pub(crate) async fn expire_older_than(&self, max_age: Duration) -> ExpiryReport {
        let now = Instant::now();
        let mut state = self.state.write().await;
        let snapshots_before = state.snapshots.len();
        state
            .snapshots
            .retain(|_, entry| now.saturating_duration_since(entry.last_touched) < max_age);
        let snapshots = snapshots_before - state.snapshots.len();

        let live_keys = state.snapshots.keys().cloned().collect::<HashSet<_>>();
        let mut session_reads = 0;
        state.reads_by_session.retain(|_, reads| {
            let before = reads.len();
            reads.retain(|key, entry| {
                now.saturating_duration_since(entry.last_touched) < max_age
                    && live_keys.contains(key)
            });
            session_reads += before - reads.len();
            !reads.is_empty()
        });
        ExpiryReport {
            snapshots,
            session_reads,
        }
    }
}

fn prepare_observation(
    workspace_root: &Path,
    relative_path: &str,
    contents: &[u8],
    mtime_ns: Option<u128>,
) -> Result<PreparedObservation, FileSnapshotLedgerError> {
    let key = snapshot_key(workspace_root, relative_path)?;
    let normalized_text = normalize_bytes(relative_path, contents)?.text;
    let line_count = normalized_line_count(&normalized_text);
    Ok(PreparedObservation {
        key,
        normalized_text,
        line_count,
        mtime_ns,
    })
}

fn snapshot_key(
    workspace_root: &Path,
    relative_path: &str,
) -> Result<SnapshotKey, FileSnapshotLedgerError> {
    validate_relative_path(relative_path)?;
    Ok(SnapshotKey {
        workspace_root: canonical_workspace_root(workspace_root)?,
        relative_path: relative_path.to_owned(),
    })
}

fn canonical_workspace_root(workspace_root: &Path) -> Result<PathBuf, FileSnapshotLedgerError> {
    std::fs::canonicalize(workspace_root).map_err(|source| {
        FileSnapshotLedgerError::WorkspaceCanonicalization {
            workspace_root: workspace_root.to_path_buf(),
            source,
        }
    })
}

fn upsert_snapshot(
    state: &mut LedgerState,
    prepared: PreparedObservation,
    source: SnapshotSource,
    writer_session_id: Option<String>,
    force_revision: bool,
) -> Result<SnapshotRecord, FileSnapshotLedgerError> {
    let now = Instant::now();
    let observed_at = Utc::now();
    let candidate = file_revision(0, &prepared.normalized_text, prepared.mtime_ns);
    if let Some(existing) = state.snapshots.get_mut(&prepared.key) {
        let content_changed = existing.record.revision.content_digest != candidate.content_digest;
        if force_revision || content_changed {
            let next_revision = existing
                .record
                .revision
                .revision
                .checked_add(1)
                .ok_or_else(|| FileSnapshotLedgerError::RevisionOverflow {
                    path: prepared.key.relative_path.clone(),
                })?;
            existing.record = SnapshotRecord {
                key: prepared.key,
                revision: file_revision(
                    next_revision,
                    &prepared.normalized_text,
                    prepared.mtime_ns,
                ),
                observed_at,
                writer_session_id,
                source,
            };
            existing.line_count = prepared.line_count;
        } else {
            existing.record.revision.mtime_ns = prepared.mtime_ns;
            existing.record.observed_at = observed_at;
        }
        existing.last_touched = now;
        return Ok(existing.record.clone());
    }

    let record = SnapshotRecord {
        key: prepared.key.clone(),
        revision: file_revision(1, &prepared.normalized_text, prepared.mtime_ns),
        observed_at,
        writer_session_id,
        source,
    };
    state.snapshots.insert(
        prepared.key,
        SnapshotEntry {
            record: record.clone(),
            line_count: prepared.line_count,
            last_touched: now,
        },
    );
    Ok(record)
}

fn same_strong_revision(left: &FileRevision, right: &FileRevision) -> bool {
    left.revision == right.revision && left.content_digest == right.content_digest
}

fn require_current_revision(
    state: &LedgerState,
    key: &SnapshotKey,
    expected: &FileRevision,
) -> Result<(), FileSnapshotLedgerError> {
    let Some(current) = state.snapshots.get(key) else {
        return Err(EditError::StaleRevision {
            path: key.relative_path.clone(),
            expected: expected.revision,
            actual: 0,
        }
        .into());
    };
    if !same_strong_revision(expected, &current.record.revision) {
        return Err(EditError::StaleRevision {
            path: key.relative_path.clone(),
            expected: expected.revision,
            actual: current.record.revision.revision,
        }
        .into());
    }
    Ok(())
}

fn normalized_line_count(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        text.strip_suffix('\n').unwrap_or(text).split('\n').count() as u64
    }
}

fn validate_ranges(
    path: &str,
    ranges: &[LineRange],
    line_count: u64,
) -> Result<(), FileSnapshotLedgerError> {
    for range in ranges {
        if range.start == 0 || range.start > range.end || range.end > line_count {
            return Err(FileSnapshotLedgerError::InvalidCoverage {
                path: path.to_owned(),
                range: *range,
                line_count,
            });
        }
    }
    Ok(())
}

fn merge_ranges(ranges: &mut Vec<LineRange>) {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<LineRange> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end.saturating_add(1)
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use jcode_edit_core::{digest_text, display_tag};
    use std::collections::HashMap;
    use tokio::sync::Barrier;

    fn range(start: u64, end: u64) -> LineRange {
        LineRange { start, end }
    }

    fn coverage(ranges: Vec<LineRange>, full_file: bool) -> ReadCoverage {
        ReadCoverage { ranges, full_file }
    }

    #[tokio::test]
    async fn canonical_roots_share_normalized_monotonic_observations() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        let first = ledger
            .observe_text(
                workspace.path(),
                "src/lib.rs",
                b"one \r\ntwo\t\r\n",
                Some(1),
            )
            .await
            .unwrap();
        let equivalent = ledger
            .observe_text(
                &workspace.path().join("."),
                "src/lib.rs",
                b"one\ntwo\n",
                Some(2),
            )
            .await
            .unwrap();
        let changed = ledger
            .observe_text(workspace.path(), "src/lib.rs", b"one\nthree\n", Some(3))
            .await
            .unwrap();

        assert_eq!(
            first.key.workspace_root,
            workspace.path().canonicalize().unwrap()
        );
        assert_eq!(first.revision.revision, 1);
        assert_eq!(equivalent.revision.revision, 1);
        assert_eq!(
            equivalent.revision.content_digest,
            first.revision.content_digest
        );
        assert_eq!(equivalent.revision.mtime_ns, Some(2));
        assert_eq!(changed.revision.revision, 2);
        assert_ne!(
            changed.revision.content_digest,
            first.revision.content_digest
        );
        assert_eq!(
            changed.revision.display_tag,
            display_tag(changed.revision.content_digest)
        );
    }

    #[tokio::test]
    async fn same_revision_reads_merge_exact_coverage_and_new_revision_replaces_it() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        ledger
            .record_read(
                "reader",
                workspace.path(),
                "file.txt",
                b"a\nb\nc\nd\n",
                None,
                coverage(vec![range(3, 3), range(1, 1)], false),
            )
            .await
            .unwrap();
        let merged = ledger
            .record_read(
                "reader",
                workspace.path(),
                "file.txt",
                b"a\nb\nc\nd\n",
                None,
                coverage(vec![range(2, 2)], false),
            )
            .await
            .unwrap();
        assert_eq!(merged.ranges, vec![range(1, 3)]);
        assert!(!merged.full_file);

        let replacement = ledger
            .record_read(
                "reader",
                workspace.path(),
                "file.txt",
                b"changed\n",
                None,
                coverage(vec![], true),
            )
            .await
            .unwrap();
        assert_eq!(replacement.revision.revision, 2);
        assert!(replacement.ranges.is_empty());
        assert!(replacement.full_file);
    }

    #[tokio::test]
    async fn invalid_read_coverage_does_not_mutate_the_ledger() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        let error = ledger
            .record_read(
                "reader",
                workspace.path(),
                "file.txt",
                b"one\ntwo\n",
                None,
                coverage(vec![range(2, 3)], false),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FileSnapshotLedgerError::InvalidCoverage { line_count: 2, .. }
        ));
        assert_eq!(
            ledger.snapshot(workspace.path(), "file.txt").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn concurrent_writes_allocate_each_revision_exactly_once() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        ledger
            .observe_text(workspace.path(), "shared.txt", b"initial\n", None)
            .await
            .unwrap();

        let writers = 12;
        let barrier = Arc::new(Barrier::new(writers + 1));
        let mut tasks = Vec::new();
        for index in 0..writers {
            let ledger = ledger.clone();
            let barrier = Arc::clone(&barrier);
            let root = workspace.path().to_path_buf();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                ledger
                    .record_write(
                        &format!("writer-{index}"),
                        &root,
                        "shared.txt",
                        format!("value-{index}\n").as_bytes(),
                        None,
                    )
                    .await
                    .unwrap()
                    .revision
                    .revision
            }));
        }
        barrier.wait().await;
        let mut revisions = Vec::new();
        for task in tasks {
            revisions.push(task.await.unwrap());
        }
        revisions.sort_unstable();
        assert_eq!(revisions, (2..=(writers as u64 + 1)).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn peer_write_stales_prior_session_read_with_writer_attribution() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        let read = ledger
            .record_read(
                "reader",
                workspace.path(),
                "shared.txt",
                b"before\n",
                None,
                coverage(vec![range(1, 1)], false),
            )
            .await
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let writer_ledger = ledger.clone();
        let writer_root = workspace.path().to_path_buf();
        let writer_barrier = Arc::clone(&barrier);
        let writer = tokio::spawn(async move {
            writer_barrier.wait().await;
            writer_ledger
                .record_write("peer", &writer_root, "shared.txt", b"after\n", None)
                .await
                .unwrap()
        });
        barrier.wait().await;
        let written = writer.await.unwrap();

        assert_eq!(written.revision.revision, read.revision.revision + 1);
        assert_eq!(written.writer_session_id.as_deref(), Some("peer"));
        assert_eq!(written.source, SnapshotSource::Write);
        assert_eq!(
            ledger
                .check_session_read("reader", workspace.path(), "shared.txt")
                .await
                .unwrap(),
            SessionReadFreshness::Stale {
                read,
                current: written,
            }
        );
    }

    #[tokio::test]
    async fn stale_check_uses_full_digest_when_short_tags_collide() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        let (left, right) = find_display_tag_collision();
        let first = ledger
            .observe_text(workspace.path(), "collision.txt", left.as_bytes(), None)
            .await
            .unwrap();
        let current = ledger
            .record_write(
                "peer",
                workspace.path(),
                "collision.txt",
                right.as_bytes(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(first.revision.display_tag, current.revision.display_tag);
        assert_ne!(
            first.revision.content_digest,
            current.revision.content_digest
        );

        let mut forged_same_revision = first.revision.clone();
        forged_same_revision.revision = current.revision.revision;
        assert_eq!(
            ledger
                .check_revision(workspace.path(), "collision.txt", &forged_same_revision)
                .await
                .unwrap(),
            SnapshotFreshness::Stale {
                expected: forged_same_revision,
                current,
            }
        );
    }

    #[tokio::test]
    async fn file_move_and_related_writes_update_the_ledger_atomically() {
        let workspace = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        let source = ledger
            .record_read(
                "reader",
                workspace.path(),
                "src/value.ts",
                b"export const value = 1;\n",
                None,
                coverage(Vec::new(), true),
            )
            .await
            .unwrap();
        let importer = ledger
            .observe_text(
                workspace.path(),
                "src/main.ts",
                b"import { value } from './value';\n",
                None,
            )
            .await
            .unwrap();

        let mut stale = source.revision.clone();
        stale.revision += 1;
        assert!(
            ledger
                .record_move_with_writes(
                    "session",
                    workspace.path(),
                    SnapshotMove {
                        source_relative_path: "src/value.ts".to_owned(),
                        expected_revision: stale,
                        destination_relative_path: "src/renamed.ts".to_owned(),
                        contents: b"export const value = 1;\n".to_vec(),
                        mtime_ns: None,
                    },
                    vec![],
                )
                .await
                .is_err()
        );
        assert!(
            ledger
                .snapshot(workspace.path(), "src/value.ts")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            ledger
                .snapshot(workspace.path(), "src/renamed.ts")
                .await
                .unwrap()
                .is_none()
        );

        let (destination, writes) = ledger
            .record_move_with_writes(
                "session",
                workspace.path(),
                SnapshotMove {
                    source_relative_path: "src/value.ts".to_owned(),
                    expected_revision: source.revision,
                    destination_relative_path: "src/renamed.ts".to_owned(),
                    contents: b"export const value = 1;\n".to_vec(),
                    mtime_ns: None,
                },
                vec![SnapshotWrite {
                    relative_path: "src/main.ts".to_owned(),
                    expected_revision: importer.revision,
                    contents: b"import { value } from './renamed';\n".to_vec(),
                    mtime_ns: None,
                }],
            )
            .await
            .unwrap();
        assert_eq!(destination.key.relative_path, "src/renamed.ts");
        assert_eq!(destination.writer_session_id.as_deref(), Some("session"));
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].revision.revision, 2);
        assert!(
            ledger
                .snapshot(workspace.path(), "src/value.ts")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn session_workspace_and_expiry_cleanup_are_scoped() {
        let workspace_a = tempfile::tempdir().unwrap();
        let workspace_b = tempfile::tempdir().unwrap();
        let ledger = FileSnapshotLedger::new();
        for workspace in [&workspace_a, &workspace_b] {
            ledger
                .record_read(
                    "reader",
                    workspace.path(),
                    "file.txt",
                    b"text\n",
                    None,
                    coverage(vec![range(1, 1)], false),
                )
                .await
                .unwrap();
        }

        assert_eq!(ledger.clear_session("reader").await, 2);
        assert!(
            ledger
                .snapshot(workspace_a.path(), "file.txt")
                .await
                .unwrap()
                .is_some()
        );
        ledger
            .record_read(
                "reader",
                workspace_a.path(),
                "file.txt",
                b"text\n",
                None,
                coverage(vec![range(1, 1)], false),
            )
            .await
            .unwrap();
        assert_eq!(
            ledger.clear_workspace(workspace_a.path()).await.unwrap(),
            ExpiryReport {
                snapshots: 1,
                session_reads: 1,
            }
        );
        assert!(
            ledger
                .snapshot(workspace_b.path(), "file.txt")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            ledger.expire_older_than(Duration::ZERO).await,
            ExpiryReport {
                snapshots: 1,
                session_reads: 0,
            }
        );
    }

    fn find_display_tag_collision() -> (String, String) {
        let mut by_tag = HashMap::new();
        for index in 0..100_000u64 {
            let candidate = format!("collision-candidate-{index}");
            let digest = digest_text(&candidate);
            let tag = display_tag(digest);
            if let Some(previous) = by_tag.insert(tag, candidate.clone())
                && digest_text(&previous) != digest
            {
                return (previous, candidate);
            }
        }
        panic!("deterministic collision search exhausted")
    }
}

#[cfg(test)]
#[path = "file_snapshot_ledger_move_tests.rs"]
mod move_read_tests;
