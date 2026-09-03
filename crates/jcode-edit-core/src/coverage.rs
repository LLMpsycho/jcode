use jcode_edit_types::{EditHunk, LineRange, MoveTarget, PutTarget, ReadSnapshot};

use crate::EditError;

/// Return the original-file ranges that must have been exposed before editing.
///
/// Insertions require coverage of their anchor line. An end insertion requires
/// the last line, except for an empty file where `full_file` coverage is checked
/// by [`validate_read_coverage`].
pub fn required_read_ranges(
    path: &str,
    hunks: &[EditHunk],
    line_count: u64,
) -> Result<Vec<LineRange>, EditError> {
    let mut required = Vec::new();
    for hunk in hunks {
        match hunk {
            EditHunk::Put { target, .. } => match target {
                PutTarget::Range { range } => {
                    validate_required_range(path, *range, line_count)?;
                    required.push(*range);
                }
                PutTarget::Before { line } | PutTarget::After { line } => {
                    let range = LineRange {
                        start: *line,
                        end: *line,
                    };
                    validate_required_range(path, range, line_count)?;
                    required.push(range);
                }
                PutTarget::End if line_count > 0 => required.push(LineRange {
                    start: line_count,
                    end: line_count,
                }),
                PutTarget::End => {}
            },
            EditHunk::Cut { range } => {
                validate_required_range(path, *range, line_count)?;
                required.push(*range);
            }
            EditHunk::Remove { line } => {
                let range = LineRange {
                    start: *line,
                    end: *line,
                };
                validate_required_range(path, range, line_count)?;
                required.push(range);
            }
            EditHunk::Move { range, destination } => {
                validate_required_range(path, *range, line_count)?;
                required.push(*range);
                match destination {
                    MoveTarget::Before { line } | MoveTarget::After { line } => {
                        let anchor = LineRange {
                            start: *line,
                            end: *line,
                        };
                        validate_required_range(path, anchor, line_count)?;
                        required.push(anchor);
                    }
                    MoveTarget::End if line_count > 0 => required.push(LineRange {
                        start: line_count,
                        end: line_count,
                    }),
                    MoveTarget::End => {}
                }
            }
        }
    }
    Ok(merge_ranges(required))
}

/// Verify that a same-revision read exposed every line an edit depends on.
pub fn validate_read_coverage(
    path: &str,
    hunks: &[EditHunk],
    line_count: u64,
    read: &ReadSnapshot,
) -> Result<(), EditError> {
    for range in &read.ranges {
        if range.start == 0 || range.start > range.end || range.end > line_count {
            return Err(EditError::InvalidCoverage {
                path: path.to_owned(),
                range: *range,
            });
        }
    }
    let required = required_read_ranges(path, hunks, line_count)?;
    if read.full_file {
        return Ok(());
    }
    if line_count == 0 && required.is_empty() {
        return Err(EditError::EmptyFileNotCovered {
            path: path.to_owned(),
        });
    }

    let covered = merge_ranges(read.ranges.clone());
    for range in required {
        if !covered
            .iter()
            .any(|candidate| candidate.start <= range.start && candidate.end >= range.end)
        {
            return Err(EditError::UncoveredRange {
                path: path.to_owned(),
                required: range,
                covered,
            });
        }
    }
    Ok(())
}

fn validate_required_range(path: &str, range: LineRange, line_count: u64) -> Result<(), EditError> {
    if range.start == 0 || range.start > range.end || range.end > line_count {
        Err(EditError::invalid_range(
            path,
            format!(
                "{}..={} is outside a {line_count}-line file",
                range.start, range.end
            ),
        ))
    } else {
        Ok(())
    }
}

fn merge_ranges(mut ranges: Vec<LineRange>) -> Vec<LineRange> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<LineRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end.saturating_add(1)
        {
            last.end = last.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    merged
}

#[cfg(test)]
mod tests {
    use jcode_edit_types::{ContentDigest, DisplayTag, FileRevision};

    use super::*;

    fn read(ranges: Vec<LineRange>, full_file: bool) -> ReadSnapshot {
        ReadSnapshot {
            path: "x".into(),
            revision: FileRevision {
                revision: 1,
                display_tag: DisplayTag { bytes: [0, 0] },
                content_digest: ContentDigest { bytes: [0; 32] },
                normalized_len: 0,
                mtime_ns: None,
            },
            ranges,
            full_file,
        }
    }

    #[test]
    fn covered_ranges_merge_and_accept_all_affected_lines() {
        let hunks = [
            EditHunk::Put {
                target: PutTarget::Range {
                    range: LineRange { start: 2, end: 3 },
                },
                lines: vec!["x".into()],
            },
            EditHunk::Move {
                range: LineRange { start: 5, end: 6 },
                destination: MoveTarget::After { line: 8 },
            },
        ];
        let coverage = read(
            vec![
                LineRange { start: 2, end: 2 },
                LineRange { start: 3, end: 3 },
                LineRange { start: 5, end: 6 },
                LineRange { start: 8, end: 8 },
            ],
            false,
        );
        assert_eq!(validate_read_coverage("x", &hunks, 10, &coverage), Ok(()));
    }

    #[test]
    fn uncovered_ranges_are_reported_with_normalized_coverage() {
        let hunks = [EditHunk::Cut {
            range: LineRange { start: 3, end: 5 },
        }];
        let coverage = read(
            vec![
                LineRange { start: 4, end: 5 },
                LineRange { start: 1, end: 2 },
            ],
            false,
        );
        assert_eq!(
            validate_read_coverage("x", &hunks, 8, &coverage),
            Err(EditError::UncoveredRange {
                path: "x".into(),
                required: LineRange { start: 3, end: 5 },
                covered: vec![
                    LineRange { start: 1, end: 2 },
                    LineRange { start: 4, end: 5 }
                ],
            })
        );
    }

    #[test]
    fn full_file_coverage_accepts_and_invalid_ranges_reject() {
        let hunks = [EditHunk::Remove { line: 2 }];
        assert_eq!(
            validate_read_coverage("x", &hunks, 2, &read(vec![], true)),
            Ok(())
        );
        assert!(matches!(
            validate_read_coverage(
                "x",
                &hunks,
                2,
                &read(vec![LineRange { start: 0, end: 1 }], false)
            ),
            Err(EditError::InvalidCoverage { .. })
        ));

        let invalid_hunk = [EditHunk::Remove { line: 3 }];
        assert!(matches!(
            validate_read_coverage("x", &invalid_hunk, 2, &read(vec![], true)),
            Err(EditError::InvalidRange { .. })
        ));
    }

    #[test]
    fn empty_file_end_insert_requires_full_file_read() {
        let hunks = [EditHunk::Put {
            target: PutTarget::End,
            lines: vec!["first".into()],
        }];
        assert_eq!(
            validate_read_coverage("x", &hunks, 0, &read(vec![], false)),
            Err(EditError::EmptyFileNotCovered { path: "x".into() })
        );
        assert_eq!(
            validate_read_coverage("x", &hunks, 0, &read(vec![], true)),
            Ok(())
        );
    }
}
