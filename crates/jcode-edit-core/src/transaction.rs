use std::collections::HashMap;

use jcode_edit_types::{ObservedFile, PlannedFileEdit, PreflightPlan, ReadSnapshot};

use crate::{
    EditError, apply_file_edit, display_tag, file_revision, normalize_bytes, parse_anchored_edit,
    validate_read_coverage, validate_relative_path,
};

/// Parse and fully validate a multi-file anchored edit without writing bytes.
///
/// The caller supplies current file bytes and the snapshots recorded by prior
/// reads. The function validates every section before returning one complete
/// plan. Any failure returns only an error, so callers cannot accidentally
/// publish a validated prefix of a multi-file request.
pub fn preflight_plan(
    input: &str,
    observed_files: &[ObservedFile],
    read_snapshots: &[ReadSnapshot],
) -> Result<PreflightPlan, EditError> {
    let edit = parse_anchored_edit(input)?;
    let observed = index_observed(observed_files)?;
    let reads = index_reads(read_snapshots)?;
    let mut planned = Vec::with_capacity(edit.files.len());

    for file in edit.files {
        let path = file.path;
        let observed = observed
            .get(path.as_str())
            .ok_or_else(|| EditError::MissingObservedFile { path: path.clone() })?;
        let read = reads
            .get(path.as_str())
            .ok_or_else(|| EditError::MissingReadSnapshot { path: path.clone() })?;
        validate_read_metadata(&path, read)?;

        if file.display_tag != read.revision.display_tag {
            return Err(EditError::TagMismatch {
                path,
                expected: read.revision.display_tag,
                actual: file.display_tag,
            });
        }

        let normalized = normalize_bytes(&path, &observed.contents)?;
        let revision_before = file_revision(observed.revision, &normalized.text, observed.mtime_ns);
        if revision_before.revision != read.revision.revision {
            return Err(EditError::StaleRevision {
                path,
                expected: read.revision.revision,
                actual: revision_before.revision,
            });
        }
        if revision_before.content_digest != read.revision.content_digest {
            return Err(EditError::StaleDigest {
                path,
                expected: read.revision.content_digest,
                actual: revision_before.content_digest,
                same_display_tag: revision_before.display_tag == read.revision.display_tag,
            });
        }
        if revision_before.normalized_len != read.revision.normalized_len {
            return Err(EditError::InvalidReadSnapshot {
                path,
                reason: "normalized length does not match the verified content digest".into(),
            });
        }

        let result = apply_file_edit(&path, &normalized.text, &file.hunks)?;
        validate_read_coverage(&path, &file.hunks, line_count(&normalized.text), read)?;
        let next_revision = revision_before
            .revision
            .checked_add(1)
            .ok_or_else(|| EditError::RevisionOverflow { path: path.clone() })?;
        let revision_after = file_revision(next_revision, &result, None);
        planned.push(PlannedFileEdit {
            path,
            revision_before,
            revision_after,
            contents: normalized.render(&result),
        });
    }

    Ok(PreflightPlan { files: planned })
}

fn index_observed(files: &[ObservedFile]) -> Result<HashMap<&str, &ObservedFile>, EditError> {
    let mut indexed = HashMap::with_capacity(files.len());
    for file in files {
        validate_relative_path(&file.path)?;
        if indexed.insert(file.path.as_str(), file).is_some() {
            return Err(EditError::DuplicateObservedFile {
                path: file.path.clone(),
            });
        }
    }
    Ok(indexed)
}

fn index_reads(reads: &[ReadSnapshot]) -> Result<HashMap<&str, &ReadSnapshot>, EditError> {
    let mut indexed = HashMap::with_capacity(reads.len());
    for read in reads {
        validate_relative_path(&read.path)?;
        if indexed.insert(read.path.as_str(), read).is_some() {
            return Err(EditError::DuplicateReadSnapshot {
                path: read.path.clone(),
            });
        }
    }
    Ok(indexed)
}

fn validate_read_metadata(path: &str, read: &ReadSnapshot) -> Result<(), EditError> {
    if display_tag(read.revision.content_digest) != read.revision.display_tag {
        return Err(EditError::InvalidReadSnapshot {
            path: path.to_owned(),
            reason: "display tag does not match the full digest".into(),
        });
    }
    Ok(())
}

fn line_count(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        text.strip_suffix('\n').unwrap_or(text).split('\n').count() as u64
    }
}

#[cfg(test)]
mod tests {
    use jcode_edit_types::{ContentDigest, DisplayTag, FileRevision, LineRange};

    use super::*;
    use crate::display_tag_hex;

    fn observed(path: &str, revision: u64, contents: &[u8]) -> ObservedFile {
        ObservedFile {
            path: path.into(),
            revision,
            contents: contents.to_vec(),
            mtime_ns: Some(11),
        }
    }

    fn read_for(file: &ObservedFile, full_file: bool, ranges: Vec<LineRange>) -> ReadSnapshot {
        let normalized = normalize_bytes(&file.path, &file.contents).unwrap();
        ReadSnapshot {
            path: file.path.clone(),
            revision: file_revision(file.revision, &normalized.text, file.mtime_ns),
            ranges,
            full_file,
        }
    }

    fn header(read: &ReadSnapshot) -> String {
        format!(
            "[{}#{}]",
            read.path,
            display_tag_hex(read.revision.display_tag)
        )
    }

    #[test]
    fn plans_one_file_and_preserves_crlf_and_bom_presentation() {
        let file = observed("a.txt", 4, b"\xEF\xBB\xBFone\r\ntwo\r\n");
        let read = read_for(&file, true, vec![]);
        let input = format!("{}\nPUT 2.=2:\n+deux", header(&read));
        let plan = preflight_plan(&input, &[file], &[read]).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].contents, b"\xEF\xBB\xBFone\r\ndeux\r\n");
        assert_eq!(plan.files[0].revision_before.revision, 4);
        assert_eq!(plan.files[0].revision_after.revision, 5);
        assert_ne!(
            plan.files[0].revision_before.content_digest,
            plan.files[0].revision_after.content_digest
        );
    }

    #[test]
    fn plans_multiple_files_in_section_order() {
        let a = observed("a", 1, b"a\n");
        let b = observed("b", 9, b"b");
        let read_a = read_for(&a, true, vec![]);
        let read_b = read_for(&b, true, vec![]);
        let input = format!(
            "{}\nPUT 1.=1:\n+A\n{}\nPUT AFTER 1:\n+B2",
            header(&read_a),
            header(&read_b)
        );
        let plan = preflight_plan(&input, &[b, a], &[read_b, read_a]).unwrap();
        assert_eq!(plan.files[0].path, "a");
        assert_eq!(plan.files[0].contents, b"A\n");
        assert_eq!(plan.files[1].path, "b");
        assert_eq!(plan.files[1].contents, b"b\nB2");
    }

    #[test]
    fn stale_revision_and_digest_are_rejected() {
        let live = observed("a", 2, b"new\n");
        let old = observed("a", 1, b"old\n");
        let read = read_for(&old, true, vec![]);
        let input = format!("{}\nREM 1", header(&read));
        assert_eq!(
            preflight_plan(
                &input,
                std::slice::from_ref(&live),
                std::slice::from_ref(&read)
            ),
            Err(EditError::StaleRevision {
                path: "a".into(),
                expected: 1,
                actual: 2,
            })
        );

        let same_revision = ObservedFile {
            revision: 1,
            ..live
        };
        assert!(matches!(
            preflight_plan(&input, &[same_revision], &[read]),
            Err(EditError::StaleDigest {
                same_display_tag: false,
                ..
            })
        ));
    }

    #[test]
    fn full_digest_detects_a_short_tag_collision() {
        let live = observed("a", 1, b"live\n");
        let mut read = read_for(&live, true, vec![]);
        let mut colliding = read.revision.content_digest.bytes;
        colliding[2] ^= 0xff;
        read.revision.content_digest = ContentDigest { bytes: colliding };
        read.revision.display_tag = DisplayTag {
            bytes: [colliding[0], colliding[1]],
        };
        let input = format!("{}\nREM 1", header(&read));
        assert!(matches!(
            preflight_plan(&input, &[live], &[read]),
            Err(EditError::StaleDigest {
                same_display_tag: true,
                ..
            })
        ));
    }

    #[test]
    fn partial_coverage_accepts_covered_and_rejects_uncovered_ranges() {
        let file = observed("a", 1, b"one\ntwo\nthree\n");
        let covered = read_for(&file, false, vec![LineRange { start: 2, end: 2 }]);
        let input = format!("{}\nPUT 2.=2:\n+TWO", header(&covered));
        assert!(preflight_plan(&input, std::slice::from_ref(&file), &[covered]).is_ok());

        let uncovered = read_for(&file, false, vec![LineRange { start: 1, end: 1 }]);
        let input = format!("{}\nPUT 2.=2:\n+TWO", header(&uncovered));
        assert!(matches!(
            preflight_plan(&input, &[file], &[uncovered]),
            Err(EditError::UncoveredRange { .. })
        ));
    }

    #[test]
    fn one_stale_file_makes_multi_file_preflight_atomic() {
        let a = observed("a", 1, b"a\n");
        let b_read_state = observed("b", 3, b"old\n");
        let b_live = observed("b", 3, b"changed\n");
        let read_a = read_for(&a, true, vec![]);
        let read_b = read_for(&b_read_state, true, vec![]);
        let before_a = a.contents.clone();
        let before_b = b_live.contents.clone();
        let input = format!(
            "{}\nPUT 1.=1:\n+A\n{}\nPUT 1.=1:\n+B",
            header(&read_a),
            header(&read_b)
        );

        assert!(matches!(
            preflight_plan(&input, &[a.clone(), b_live.clone()], &[read_a, read_b]),
            Err(EditError::StaleDigest { path, .. }) if path == "b"
        ));
        assert_eq!(a.contents, before_a);
        assert_eq!(b_live.contents, before_b);
    }

    #[test]
    fn duplicate_inputs_and_inconsistent_read_metadata_reject() {
        let file = observed("a", 1, b"a");
        let mut read = read_for(&file, true, vec![]);
        let input = format!("{}\nREM 1", header(&read));
        assert!(matches!(
            preflight_plan(&input, &[file.clone(), file.clone()], &[read.clone()]),
            Err(EditError::DuplicateObservedFile { .. })
        ));
        assert!(matches!(
            preflight_plan(
                &input,
                std::slice::from_ref(&file),
                &[read.clone(), read.clone()]
            ),
            Err(EditError::DuplicateReadSnapshot { .. })
        ));

        read.revision.display_tag.bytes[0] ^= 1;
        assert!(matches!(
            preflight_plan(&input, &[file], &[read]),
            Err(EditError::InvalidReadSnapshot { .. })
        ));
    }

    #[test]
    fn revision_overflow_is_structured() {
        let file = observed("a", u64::MAX, b"a");
        let read = read_for(&file, true, vec![]);
        let input = format!("{}\nPUT 1.=1:\n+b", header(&read));
        assert_eq!(
            preflight_plan(&input, &[file], &[read]),
            Err(EditError::RevisionOverflow { path: "a".into() })
        );
    }

    #[test]
    fn structured_errors_have_stable_kinds() {
        let error = EditError::StaleRevision {
            path: "a".into(),
            expected: 1,
            actual: 2,
        };
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["kind"], "stale_revision");
        assert_eq!(value["path"], "a");
    }

    #[allow(dead_code)]
    fn _assert_file_revision_is_public(_: FileRevision) {}
}
