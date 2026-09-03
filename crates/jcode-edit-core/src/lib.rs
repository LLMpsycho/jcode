//! Pure anchored-edit primitives for Jcode.
//!
//! This crate performs no filesystem I/O. Callers provide observed bytes and
//! prior read snapshots, then receive either a fully validated in-memory plan or
//! a structured error. Publication, rollback, ledgers, and session storage stay
//! in higher-level crates.

mod apply;
mod coverage;
mod digest;
mod error;
mod normalize;
mod parser;
mod transaction;

pub use apply::apply_file_edit;
pub use coverage::{required_read_ranges, validate_read_coverage};
pub use digest::{
    digest_bytes, digest_hex, digest_text, display_tag, display_tag_hex, file_revision,
    parse_display_tag,
};
pub use error::EditError;
pub use normalize::{LineEnding, NormalizedText, normalize_bytes};
pub use parser::{parse_anchored_edit, validate_relative_path};
pub use transaction::preflight_plan;

pub use jcode_edit_types::{
    AnchoredEdit, ContentDigest, DisplayTag, EditHunk, FileEdit, FileRevision, LineRange,
    MoveTarget, ObservedFile, PlannedFileEdit, PreflightPlan, PutTarget, ReadSnapshot,
};
