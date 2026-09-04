use crate::{LspError, Position, Result, TextEdit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedTextEdits {
    pub original: String,
    pub updated: String,
    pub edit_count: usize,
}

pub fn apply_text_edits(text: &str, edits: &[TextEdit]) -> Result<PlannedTextEdits> {
    let mut replacements = edits
        .iter()
        .map(|edit| {
            let start = byte_offset(text, edit.range.start)?;
            let end = byte_offset(text, edit.range.end)?;
            if start > end {
                return Err(LspError::InvalidMessage(
                    "text edit range starts after its end".to_owned(),
                ));
            }
            Ok((start, end, edit.new_text.as_str()))
        })
        .collect::<Result<Vec<_>>>()?;
    replacements.sort_by_key(|(start, end, _)| (*start, *end));
    for pair in replacements.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(LspError::InvalidMessage(
                "workspace edit contains overlapping text edits".to_owned(),
            ));
        }
    }

    let mut updated = text.to_owned();
    for (start, end, replacement) in replacements.iter().rev() {
        updated.replace_range(*start..*end, replacement);
    }
    Ok(PlannedTextEdits {
        original: text.to_owned(),
        updated,
        edit_count: edits.len(),
    })
}

fn byte_offset(text: &str, position: Position) -> Result<usize> {
    let mut line = 0_u32;
    let mut line_start = 0_usize;
    for (index, byte) in text.bytes().enumerate() {
        if line == position.line {
            break;
        }
        if byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    if line != position.line {
        return Err(LspError::InvalidMessage(format!(
            "text edit line {} is outside the document",
            position.line
        )));
    }

    let line_end = text[line_start..]
        .find('\n')
        .map(|offset| line_start + offset)
        .unwrap_or(text.len());
    let line_text = &text[line_start..line_end];
    let mut utf16_offset = 0_u32;
    for (byte_offset, character) in line_text.char_indices() {
        if utf16_offset == position.character {
            return Ok(line_start + byte_offset);
        }
        utf16_offset += character.len_utf16() as u32;
        if utf16_offset > position.character {
            return Err(LspError::InvalidMessage(
                "text edit character splits a UTF-16 surrogate pair".to_owned(),
            ));
        }
    }
    if utf16_offset == position.character {
        Ok(line_end)
    } else {
        Err(LspError::InvalidMessage(format!(
            "text edit character {} is outside line {}",
            position.character, position.line
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::Range;

    use super::*;

    fn edit(start: Position, end: Position, new_text: &str) -> TextEdit {
        TextEdit {
            range: Range { start, end },
            new_text: new_text.to_owned(),
        }
    }

    #[test]
    fn applies_multiple_edits_in_original_document_coordinates() {
        let text = "alpha beta\ngamma\n";
        let plan = apply_text_edits(
            text,
            &[
                edit(
                    Position {
                        line: 0,
                        character: 0,
                    },
                    Position {
                        line: 0,
                        character: 5,
                    },
                    "one",
                ),
                edit(
                    Position {
                        line: 1,
                        character: 0,
                    },
                    Position {
                        line: 1,
                        character: 5,
                    },
                    "two",
                ),
            ],
        )
        .unwrap();
        assert_eq!(plan.updated, "one beta\ntwo\n");
        assert_eq!(plan.edit_count, 2);
    }

    #[test]
    fn converts_utf16_offsets_without_splitting_surrogates() {
        let text = "a😀b";
        let plan = apply_text_edits(
            text,
            &[edit(
                Position {
                    line: 0,
                    character: 1,
                },
                Position {
                    line: 0,
                    character: 3,
                },
                "x",
            )],
        )
        .unwrap();
        assert_eq!(plan.updated, "axb");
        assert!(
            apply_text_edits(
                text,
                &[edit(
                    Position {
                        line: 0,
                        character: 2
                    },
                    Position {
                        line: 0,
                        character: 3
                    },
                    "x",
                )]
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_overlapping_and_out_of_bounds_edits() {
        let overlap = [
            edit(
                Position {
                    line: 0,
                    character: 0,
                },
                Position {
                    line: 0,
                    character: 3,
                },
                "x",
            ),
            edit(
                Position {
                    line: 0,
                    character: 2,
                },
                Position {
                    line: 0,
                    character: 4,
                },
                "y",
            ),
        ];
        assert!(apply_text_edits("abcdef", &overlap).is_err());
        assert!(
            apply_text_edits(
                "one",
                &[edit(
                    Position {
                        line: 2,
                        character: 0
                    },
                    Position {
                        line: 2,
                        character: 0
                    },
                    "x",
                )]
            )
            .is_err()
        );
    }
}
