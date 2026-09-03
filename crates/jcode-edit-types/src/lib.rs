//! Stable, serialization-only contracts for Jcode's anchored-edit foundation.
//!
//! This crate intentionally contains data transfer objects and no filesystem,
//! parsing, hashing, or application logic. Behavior belongs in
//! `jcode-edit-core`, allowing these wire shapes to remain small and stable.

use serde::{Deserialize, Serialize};

/// A full SHA-256 digest of canonical normalized text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentDigest {
    /// Digest bytes in network order.
    pub bytes: [u8; 32],
}

/// The two-byte, four-hex-character tag shown in model-facing file headers.
///
/// This tag is only a display hint. Correctness checks must compare the full
/// [`ContentDigest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayTag {
    /// The first two bytes of the corresponding full digest.
    pub bytes: [u8; 2],
}

/// Revision metadata recorded when a text file is observed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRevision {
    /// Monotonically increasing ledger revision.
    pub revision: u64,
    /// Short model-facing tag.
    pub display_tag: DisplayTag,
    /// Full digest used for all correctness checks.
    pub content_digest: ContentDigest,
    /// Byte length of the canonical normalized UTF-8 text.
    pub normalized_len: u64,
    /// Optional filesystem modification time in nanoseconds.
    pub mtime_ns: Option<u128>,
}

/// A one-based inclusive range of lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineRange {
    /// First covered line, starting at one.
    pub start: u64,
    /// Last covered line, inclusive.
    pub end: u64,
}

/// The file revision and line ranges exposed by a prior read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSnapshot {
    /// Workspace-relative path using `/` separators.
    pub path: String,
    /// Revision observed by the read.
    pub revision: FileRevision,
    /// One-based inclusive line ranges exposed by partial reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<LineRange>,
    /// Whether the whole file was exposed.
    #[serde(default)]
    pub full_file: bool,
}

/// An in-memory live file supplied to pure preflight planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedFile {
    /// Workspace-relative path using `/` separators.
    pub path: String,
    /// Current ledger revision.
    pub revision: u64,
    /// Current bytes. The core accepts only UTF-8 text, with an optional BOM.
    pub contents: Vec<u8>,
    /// Optional filesystem modification time in nanoseconds.
    pub mtime_ns: Option<u128>,
}

/// A parsed anchored-edit document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchoredEdit {
    /// File sections in source order.
    pub files: Vec<FileEdit>,
}

/// All anchored hunks targeting one workspace-relative file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdit {
    /// Workspace-relative path from the section header.
    pub path: String,
    /// Short tag from `[path#ABCD]`.
    pub display_tag: DisplayTag,
    /// Hunks in source order.
    pub hunks: Vec<EditHunk>,
}

/// One strict line-oriented edit command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditHunk {
    /// Replace a range or insert at a line boundary.
    Put {
        target: PutTarget,
        lines: Vec<String>,
    },
    /// Delete an inclusive range.
    Cut { range: LineRange },
    /// Delete one line.
    Remove { line: u64 },
    /// Move an inclusive range to another line boundary.
    Move {
        range: LineRange,
        destination: MoveTarget,
    },
}

/// Target accepted by a `PUT` command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PutTarget {
    /// Replace an inclusive range.
    Range { range: LineRange },
    /// Insert immediately before a line.
    Before { line: u64 },
    /// Insert immediately after a line.
    After { line: u64 },
    /// Insert at the end of the file.
    End,
}

/// Destination accepted by an `MV` command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoveTarget {
    /// Move immediately before a line.
    Before { line: u64 },
    /// Move immediately after a line.
    After { line: u64 },
    /// Move to the end of the file.
    End,
}

/// The zero-write result of preflight planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightPlan {
    /// Fully validated file results in input section order.
    pub files: Vec<PlannedFileEdit>,
}

/// One file's validated before/after state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFileEdit {
    /// Workspace-relative path.
    pub path: String,
    /// Verified revision before the edit.
    pub revision_before: FileRevision,
    /// Proposed revision after the edit.
    pub revision_after: FileRevision,
    /// Proposed UTF-8 bytes. Preflight never writes them.
    pub contents: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_shape_is_deterministic_and_round_trips() {
        let value = ReadSnapshot {
            path: "src/lib.rs".into(),
            revision: FileRevision {
                revision: 7,
                display_tag: DisplayTag {
                    bytes: [0xab, 0xcd],
                },
                content_digest: ContentDigest { bytes: [9; 32] },
                normalized_len: 12,
                mtime_ns: None,
            },
            ranges: vec![LineRange { start: 2, end: 4 }],
            full_file: false,
        };

        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\"path\":\"src/lib.rs\",\"revision\":{{\"revision\":7,\"display_tag\":{{\"bytes\":[171,205]}},\"content_digest\":{{\"bytes\":[{}]}},\"normalized_len\":12,\"mtime_ns\":null}},\"ranges\":[{{\"start\":2,\"end\":4}}],\"full_file\":false}}",
                vec!["9"; 32].join(",")
            )
        );
        assert_eq!(serde_json::from_str::<ReadSnapshot>(&json).unwrap(), value);
    }

    #[test]
    fn empty_coverage_fields_have_stable_defaults() {
        let json = r#"{"path":"empty","revision":{"revision":0,"display_tag":{"bytes":[0,0]},"content_digest":{"bytes":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]},"normalized_len":0,"mtime_ns":null}}"#;
        let value: ReadSnapshot = serde_json::from_str(json).unwrap();
        assert!(value.ranges.is_empty());
        assert!(!value.full_file);
    }
}
