use jcode_edit_types::{ContentDigest, DisplayTag, LineRange};
use serde::Serialize;
use thiserror::Error;

/// Structured, deterministic failure returned before any write can occur.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditError {
    /// The anchored-edit text does not match the strict grammar.
    #[error("anchored edit is malformed at line {line}: {message}")]
    Parse { line: usize, message: String },

    /// A section path is unsafe or non-canonical.
    #[error("unsafe edit path `{path}`: {reason}")]
    UnsafePath { path: String, reason: String },

    /// The edit document contains the same path more than once.
    #[error("duplicate edit path `{path}`")]
    DuplicatePath { path: String },

    /// The caller supplied duplicate live state for one path.
    #[error("duplicate observed file `{path}`")]
    DuplicateObservedFile { path: String },

    /// The caller supplied duplicate read state for one path.
    #[error("duplicate read snapshot `{path}`")]
    DuplicateReadSnapshot { path: String },

    /// No live in-memory file was supplied for a section.
    #[error("missing observed file `{path}`")]
    MissingObservedFile { path: String },

    /// No prior read snapshot was supplied for a section.
    #[error("missing read snapshot `{path}`")]
    MissingReadSnapshot { path: String },

    /// A supplied read snapshot contains internally inconsistent metadata.
    #[error("invalid read snapshot for `{path}`: {reason}")]
    InvalidReadSnapshot { path: String, reason: String },

    /// A supplied file is not valid UTF-8 after an optional BOM.
    #[error("file `{path}` is not valid UTF-8")]
    InvalidUtf8 { path: String },

    /// The short tag in a section does not match the read snapshot.
    #[error("display tag for `{path}` does not match the read snapshot")]
    TagMismatch {
        path: String,
        expected: DisplayTag,
        actual: DisplayTag,
    },

    /// The live ledger revision differs from the revision that was read.
    #[error(
        "file `{path}` changed after it was read (expected revision {expected}, current {actual})"
    )]
    StaleRevision {
        path: String,
        expected: u64,
        actual: u64,
    },

    /// The normalized live bytes differ from the full digest that was read.
    #[error("file `{path}` content changed after it was read")]
    StaleDigest {
        path: String,
        expected: ContentDigest,
        actual: ContentDigest,
        /// True when the short display tags collide despite distinct digests.
        same_display_tag: bool,
    },

    /// A line or range is outside the original file.
    #[error("invalid line range for `{path}`: {message}")]
    InvalidRange { path: String, message: String },

    /// Two hunks address overlapping source lines or boundaries.
    #[error("overlapping hunks for `{path}` at hunk {first_hunk} and {second_hunk}")]
    OverlappingHunks {
        path: String,
        first_hunk: usize,
        second_hunk: usize,
    },

    /// A read-coverage range is malformed.
    #[error("invalid read coverage for `{path}`: {range:?}")]
    InvalidCoverage { path: String, range: LineRange },

    /// A required line range was not exposed by the prior read.
    #[error("edit range {required:?} for `{path}` was not covered by the prior read")]
    UncoveredRange {
        path: String,
        required: LineRange,
        covered: Vec<LineRange>,
    },

    /// An end insertion into an empty file requires proof that the empty file was read.
    #[error("empty file `{path}` was not fully covered by the prior read")]
    EmptyFileNotCovered { path: String },

    /// The next monotonic revision cannot be represented.
    #[error("revision overflow for `{path}`")]
    RevisionOverflow { path: String },
}

impl EditError {
    pub(crate) fn parse(line: usize, message: impl Into<String>) -> Self {
        Self::Parse {
            line,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_range(path: &str, message: impl Into<String>) -> Self {
        Self::InvalidRange {
            path: path.to_owned(),
            message: message.into(),
        }
    }
}
