use std::collections::BTreeMap;

use jcode_edit_types::{EditHunk, LineRange, MoveTarget, PutTarget};

use crate::EditError;

#[derive(Clone, Debug)]
struct Action {
    hunk_index: usize,
    source: Option<LineRange>,
    insertion_boundary: Option<u64>,
    inserted_lines: Vec<String>,
}

/// Apply validated line-oriented hunks to canonical normalized text in memory.
///
/// Every hunk is resolved against the original text. Overlapping source ranges,
/// duplicate insertion boundaries, and insertions into the interior of another
/// hunk's source range are rejected rather than resolved by ordering.
pub fn apply_file_edit(
    path: &str,
    normalized_text: &str,
    hunks: &[EditHunk],
) -> Result<String, EditError> {
    let (lines, had_final_newline) = split_lines(normalized_text);
    let line_count = lines.len() as u64;
    let actions = build_actions(path, &lines, line_count, hunks)?;
    validate_non_overlapping(path, &actions)?;

    let mut deleted = vec![false; lines.len()];
    let mut insertions: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for action in actions {
        if let Some(range) = action.source {
            for index in (range.start - 1)..range.end {
                deleted[index as usize] = true;
            }
        }
        if let Some(boundary) = action.insertion_boundary {
            insertions.insert(boundary, action.inserted_lines);
        }
    }

    let mut result = Vec::new();
    for boundary in 0..=line_count {
        if let Some(inserted) = insertions.remove(&boundary) {
            result.extend(inserted);
        }
        if boundary < line_count && !deleted[boundary as usize] {
            result.push(lines[boundary as usize].clone());
        }
    }

    let mut text = result.join("\n");
    if had_final_newline && !result.is_empty() {
        text.push('\n');
    }
    Ok(text)
}

fn build_actions(
    path: &str,
    lines: &[String],
    line_count: u64,
    hunks: &[EditHunk],
) -> Result<Vec<Action>, EditError> {
    let mut actions = Vec::with_capacity(hunks.len());
    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let action = match hunk {
            EditHunk::Put { target, lines } => {
                let inserted_lines = canonical_inserted_lines(lines);
                match target {
                    PutTarget::Range { range } => {
                        validate_range(path, *range, line_count)?;
                        Action {
                            hunk_index,
                            source: Some(*range),
                            insertion_boundary: Some(range.start - 1),
                            inserted_lines,
                        }
                    }
                    PutTarget::Before { line } => {
                        validate_line(path, *line, line_count)?;
                        Action {
                            hunk_index,
                            source: None,
                            insertion_boundary: Some(line - 1),
                            inserted_lines,
                        }
                    }
                    PutTarget::After { line } => {
                        validate_line(path, *line, line_count)?;
                        Action {
                            hunk_index,
                            source: None,
                            insertion_boundary: Some(*line),
                            inserted_lines,
                        }
                    }
                    PutTarget::End => Action {
                        hunk_index,
                        source: None,
                        insertion_boundary: Some(line_count),
                        inserted_lines,
                    },
                }
            }
            EditHunk::Cut { range } => {
                validate_range(path, *range, line_count)?;
                Action {
                    hunk_index,
                    source: Some(*range),
                    insertion_boundary: None,
                    inserted_lines: Vec::new(),
                }
            }
            EditHunk::Remove { line } => {
                validate_line(path, *line, line_count)?;
                Action {
                    hunk_index,
                    source: Some(LineRange {
                        start: *line,
                        end: *line,
                    }),
                    insertion_boundary: None,
                    inserted_lines: Vec::new(),
                }
            }
            EditHunk::Move { range, destination } => {
                validate_range(path, *range, line_count)?;
                let boundary = destination_boundary(path, destination, line_count)?;
                if boundary >= range.start - 1 && boundary <= range.end {
                    return Err(EditError::invalid_range(
                        path,
                        format!(
                            "move destination boundary {boundary} touches source {}..={}",
                            range.start, range.end
                        ),
                    ));
                }
                let inserted_lines = lines[(range.start - 1) as usize..range.end as usize].to_vec();
                Action {
                    hunk_index,
                    source: Some(*range),
                    insertion_boundary: Some(boundary),
                    inserted_lines,
                }
            }
        };
        actions.push(action);
    }
    Ok(actions)
}

fn validate_non_overlapping(path: &str, actions: &[Action]) -> Result<(), EditError> {
    for (left_index, left) in actions.iter().enumerate() {
        for right in &actions[left_index + 1..] {
            let source_overlap = match (left.source, right.source) {
                (Some(left), Some(right)) => left.start <= right.end && right.start <= left.end,
                _ => false,
            };
            let duplicate_boundary = left.insertion_boundary.is_some()
                && left.insertion_boundary == right.insertion_boundary;
            let left_in_right =
                left.insertion_boundary
                    .zip(right.source)
                    .is_some_and(|(boundary, source)| {
                        boundary >= source.start && boundary < source.end
                    });
            let right_in_left =
                right
                    .insertion_boundary
                    .zip(left.source)
                    .is_some_and(|(boundary, source)| {
                        boundary >= source.start && boundary < source.end
                    });
            if source_overlap || duplicate_boundary || left_in_right || right_in_left {
                return Err(EditError::OverlappingHunks {
                    path: path.to_owned(),
                    first_hunk: left.hunk_index,
                    second_hunk: right.hunk_index,
                });
            }
        }
    }
    Ok(())
}

fn validate_range(path: &str, range: LineRange, line_count: u64) -> Result<(), EditError> {
    if range.start == 0 || range.start > range.end || range.end > line_count {
        return Err(EditError::invalid_range(
            path,
            format!(
                "{}..={} is outside a {line_count}-line file",
                range.start, range.end
            ),
        ));
    }
    Ok(())
}

fn validate_line(path: &str, line: u64, line_count: u64) -> Result<(), EditError> {
    validate_range(
        path,
        LineRange {
            start: line,
            end: line,
        },
        line_count,
    )
}

fn destination_boundary(
    path: &str,
    destination: &MoveTarget,
    line_count: u64,
) -> Result<u64, EditError> {
    match destination {
        MoveTarget::Before { line } => {
            validate_line(path, *line, line_count)?;
            Ok(line - 1)
        }
        MoveTarget::After { line } => {
            validate_line(path, *line, line_count)?;
            Ok(*line)
        }
        MoveTarget::End => Ok(line_count),
    }
}

fn canonical_inserted_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.trim_end_matches([' ', '\t']).to_owned())
        .collect()
}

fn split_lines(text: &str) -> (Vec<String>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let had_final_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    (
        body.split('\n').map(ToOwned::to_owned).collect(),
        had_final_newline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> LineRange {
        LineRange { start, end }
    }

    #[test]
    fn replacement_selects_by_position_even_with_repeated_lines() {
        let hunks = [EditHunk::Put {
            target: PutTarget::Range { range: range(2, 2) },
            lines: vec!["changed".into()],
        }];
        assert_eq!(
            apply_file_edit("x", "same\nsame\nsame\n", &hunks).unwrap(),
            "same\nchanged\nsame\n"
        );
    }

    #[test]
    fn supports_insertions_before_after_end_and_empty_file() {
        let hunks = [
            EditHunk::Put {
                target: PutTarget::Before { line: 1 },
                lines: vec!["before".into()],
            },
            EditHunk::Put {
                target: PutTarget::After { line: 1 },
                lines: vec!["after".into()],
            },
            EditHunk::Put {
                target: PutTarget::End,
                lines: vec!["end".into()],
            },
        ];
        assert_eq!(
            apply_file_edit("x", "one\ntwo", &hunks).unwrap(),
            "before\none\nafter\ntwo\nend"
        );
        let empty = [EditHunk::Put {
            target: PutTarget::End,
            lines: vec!["first".into()],
        }];
        assert_eq!(apply_file_edit("x", "", &empty).unwrap(), "first");
    }

    #[test]
    fn cut_remove_and_move_preserve_no_final_newline() {
        let hunks = [
            EditHunk::Cut { range: range(2, 2) },
            EditHunk::Remove { line: 4 },
            EditHunk::Move {
                range: range(1, 1),
                destination: MoveTarget::End,
            },
        ];
        assert_eq!(
            apply_file_edit("x", "one\ntwo\nthree\nfour", &hunks).unwrap(),
            "three\none"
        );
    }

    #[test]
    fn deleting_every_line_produces_an_empty_file() {
        let hunks = [EditHunk::Cut { range: range(1, 1) }];
        assert_eq!(apply_file_edit("x", "only\n", &hunks).unwrap(), "");
    }

    #[test]
    fn rejects_out_of_bounds_and_overlapping_hunks() {
        let out_of_bounds = [EditHunk::Remove { line: 4 }];
        assert!(matches!(
            apply_file_edit("x", "a\nb\n", &out_of_bounds),
            Err(EditError::InvalidRange { .. })
        ));

        let overlaps = [
            EditHunk::Cut { range: range(1, 2) },
            EditHunk::Put {
                target: PutTarget::Range { range: range(2, 3) },
                lines: vec!["x".into()],
            },
        ];
        assert_eq!(
            apply_file_edit("x", "a\nb\nc\n", &overlaps),
            Err(EditError::OverlappingHunks {
                path: "x".into(),
                first_hunk: 0,
                second_hunk: 1,
            })
        );
    }

    #[test]
    fn rejects_duplicate_boundaries_and_moves_into_their_source() {
        let duplicate = [
            EditHunk::Put {
                target: PutTarget::Before { line: 2 },
                lines: vec!["x".into()],
            },
            EditHunk::Put {
                target: PutTarget::After { line: 1 },
                lines: vec!["y".into()],
            },
        ];
        assert!(matches!(
            apply_file_edit("x", "a\nb", &duplicate),
            Err(EditError::OverlappingHunks { .. })
        ));

        let invalid_move = [EditHunk::Move {
            range: range(2, 3),
            destination: MoveTarget::After { line: 2 },
        }];
        assert!(matches!(
            apply_file_edit("x", "a\nb\nc\nd", &invalid_move),
            Err(EditError::InvalidRange { .. })
        ));
    }

    #[test]
    fn unicode_lines_and_trailing_whitespace_are_safe() {
        let hunks = [EditHunk::Put {
            target: PutTarget::Range { range: range(1, 1) },
            lines: vec!["雪☃️  ".into()],
        }];
        assert_eq!(apply_file_edit("x", "λ\n", &hunks).unwrap(), "雪☃️\n");
    }
}
