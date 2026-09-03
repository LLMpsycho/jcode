use std::collections::HashSet;

use jcode_edit_types::{AnchoredEdit, EditHunk, FileEdit, LineRange, MoveTarget, PutTarget};

use crate::{EditError, parse_display_tag};

/// Parse the strict line-oriented anchored-edit grammar.
///
/// Each section starts with `[relative/path#ABCD]`. Supported commands are:
///
/// ```text
/// PUT 2.=4:
/// +replacement line
/// PUT BEFORE 2:
/// +inserted line
/// PUT AFTER 2:
/// +inserted line
/// PUT END:
/// +appended line
/// CUT 2.=4
/// REM 2
/// MV 2.=4 BEFORE 8
/// MV 2.=4 AFTER 8
/// MV 2.=4 END
/// ```
///
/// Line numbers are one-based and all ranges are inclusive. `PUT` body lines
/// must begin with `+`; the prefix is syntax and is not included in the text.
pub fn parse_anchored_edit(input: &str) -> Result<AnchoredEdit, EditError> {
    let mut lines: Vec<&str> = input.split('\n').map(strip_cr).collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.is_empty() {
        return Err(EditError::parse(1, "expected a file section"));
    }

    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut current: Option<FileEdit> = None;
    let mut index = 0usize;

    while index < lines.len() {
        let line_number = index + 1;
        let line = lines[index];
        if line.starts_with('[') {
            if let Some(file) = current.take() {
                if file.hunks.is_empty() {
                    return Err(EditError::parse(
                        line_number,
                        format!("section `{}` has no hunks", file.path),
                    ));
                }
                files.push(file);
            }
            let (path, display_tag) = parse_header(line, line_number)?;
            validate_relative_path(&path)?;
            if !seen_paths.insert(path.clone()) {
                return Err(EditError::DuplicatePath { path });
            }
            current = Some(FileEdit {
                path,
                display_tag,
                hunks: Vec::new(),
            });
            index += 1;
            continue;
        }

        let file = current
            .as_mut()
            .ok_or_else(|| EditError::parse(line_number, "expected a file section header"))?;
        if line.is_empty() {
            return Err(EditError::parse(line_number, "blank lines are not allowed"));
        }

        if let Some(command) = line.strip_prefix("PUT ") {
            let target_text = command
                .strip_suffix(':')
                .ok_or_else(|| EditError::parse(line_number, "PUT command must end with `:`"))?;
            let target = parse_put_target(target_text, line_number)?;
            index += 1;
            let body_start = index;
            let mut body = Vec::new();
            while index < lines.len() {
                if let Some(text) = lines[index].strip_prefix('+') {
                    body.push(text.to_owned());
                    index += 1;
                } else {
                    break;
                }
            }
            if body.is_empty() {
                return Err(EditError::parse(
                    body_start + 1,
                    "PUT requires at least one `+` body line",
                ));
            }
            file.hunks.push(EditHunk::Put {
                target,
                lines: body,
            });
            continue;
        }

        if let Some(range) = line.strip_prefix("CUT ") {
            reject_suffix(range, ':', line_number, "CUT must not end with `:`")?;
            file.hunks.push(EditHunk::Cut {
                range: parse_range(range, line_number)?,
            });
            index += 1;
            continue;
        }

        if let Some(value) = line.strip_prefix("REM ") {
            reject_suffix(value, ':', line_number, "REM must not end with `:`")?;
            file.hunks.push(EditHunk::Remove {
                line: parse_line_number(value, line_number)?,
            });
            index += 1;
            continue;
        }

        if let Some(command) = line.strip_prefix("MV ") {
            reject_suffix(command, ':', line_number, "MV must not end with `:`")?;
            let (range, destination) = parse_move(command, line_number)?;
            file.hunks.push(EditHunk::Move { range, destination });
            index += 1;
            continue;
        }

        return Err(EditError::parse(
            line_number,
            "expected PUT, CUT, REM, MV, or a file section header",
        ));
    }

    if let Some(file) = current {
        if file.hunks.is_empty() {
            return Err(EditError::parse(
                lines.len(),
                format!("section `{}` has no hunks", file.path),
            ));
        }
        files.push(file);
    }
    if files.is_empty() {
        return Err(EditError::parse(1, "expected a file section"));
    }
    Ok(AnchoredEdit { files })
}

/// Validate and return an unambiguous workspace-relative path.
pub fn validate_relative_path(path: &str) -> Result<(), EditError> {
    let reason = if path.is_empty() {
        Some("path is empty")
    } else if path.starts_with('/') || path.starts_with('\\') {
        Some("absolute paths are not allowed")
    } else if path.contains('\\') {
        Some("backslashes are not allowed; use `/` separators")
    } else if path
        .bytes()
        .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        Some("control characters are not allowed")
    } else if path.ends_with('/') || path.contains("//") {
        Some("path must not contain empty components")
    } else if path
        .split('/')
        .any(|component| component == "." || component == "..")
    {
        Some("`.` and `..` path components are not allowed")
    } else if path
        .split('/')
        .next()
        .is_some_and(|component| component.as_bytes().get(1) == Some(&b':'))
    {
        Some("Windows drive paths are not allowed")
    } else {
        None
    };

    match reason {
        Some(reason) => Err(EditError::UnsafePath {
            path: path.to_owned(),
            reason: reason.to_owned(),
        }),
        None => Ok(()),
    }
}

fn parse_header(
    line: &str,
    line_number: usize,
) -> Result<(String, jcode_edit_types::DisplayTag), EditError> {
    let inner = line
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| EditError::parse(line_number, "malformed file section header"))?;
    let (path, tag) = inner
        .rsplit_once('#')
        .ok_or_else(|| EditError::parse(line_number, "header must contain `#` and a tag"))?;
    let display_tag = parse_display_tag(tag).ok_or_else(|| {
        EditError::parse(
            line_number,
            "display tag must be exactly four hexadecimal characters",
        )
    })?;
    Ok((path.to_owned(), display_tag))
}

fn parse_put_target(value: &str, line_number: usize) -> Result<PutTarget, EditError> {
    if value == "END" {
        return Ok(PutTarget::End);
    }
    if let Some(line) = value.strip_prefix("BEFORE ") {
        return Ok(PutTarget::Before {
            line: parse_line_number(line, line_number)?,
        });
    }
    if let Some(line) = value.strip_prefix("AFTER ") {
        return Ok(PutTarget::After {
            line: parse_line_number(line, line_number)?,
        });
    }
    Ok(PutTarget::Range {
        range: parse_range(value, line_number)?,
    })
}

fn parse_move(value: &str, line_number: usize) -> Result<(LineRange, MoveTarget), EditError> {
    if let Some(range) = value.strip_suffix(" END") {
        return Ok((parse_range(range, line_number)?, MoveTarget::End));
    }
    if let Some((range, line)) = value.split_once(" BEFORE ") {
        return Ok((
            parse_range(range, line_number)?,
            MoveTarget::Before {
                line: parse_line_number(line, line_number)?,
            },
        ));
    }
    if let Some((range, line)) = value.split_once(" AFTER ") {
        return Ok((
            parse_range(range, line_number)?,
            MoveTarget::After {
                line: parse_line_number(line, line_number)?,
            },
        ));
    }
    Err(EditError::parse(
        line_number,
        "MV destination must be BEFORE <line>, AFTER <line>, or END",
    ))
}

fn parse_range(value: &str, line_number: usize) -> Result<LineRange, EditError> {
    let (start, end) = value
        .split_once(".=")
        .ok_or_else(|| EditError::parse(line_number, "range must use `<start>.=<end>`"))?;
    if end.contains(".=") {
        return Err(EditError::parse(
            line_number,
            "range has too many separators",
        ));
    }
    let range = LineRange {
        start: parse_line_number(start, line_number)?,
        end: parse_line_number(end, line_number)?,
    };
    if range.start > range.end {
        return Err(EditError::parse(
            line_number,
            "range start must not exceed its end",
        ));
    }
    Ok(range)
}

fn parse_line_number(value: &str, line_number: usize) -> Result<u64, EditError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EditError::parse(
            line_number,
            "line number must contain only decimal digits",
        ));
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| EditError::parse(line_number, "line number is too large"))?;
    if parsed == 0 {
        return Err(EditError::parse(line_number, "line numbers start at one"));
    }
    Ok(parsed)
}

fn reject_suffix(
    value: &str,
    suffix: char,
    line_number: usize,
    message: &str,
) -> Result<(), EditError> {
    if value.ends_with(suffix) {
        Err(EditError::parse(line_number, message))
    } else {
        Ok(())
    }
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands_and_plus_prefixed_content() {
        let parsed = parse_anchored_edit(
            "[src/lib.rs#a13f]\r\nPUT 2.=3:\r\n+雪\r\n++literal plus\r\nPUT BEFORE 1:\r\n+head\r\nPUT AFTER 4:\r\n+tail\r\nPUT END:\r\n+end\r\nCUT 5.=6\r\nREM 7\r\nMV 8.=9 BEFORE 2\r\nMV 10.=11 AFTER 3\r\nMV 12.=13 END\r\n",
        )
        .unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].hunks.len(), 9);
        assert!(matches!(
            &parsed.files[0].hunks[0],
            EditHunk::Put { lines, .. } if lines == &["雪", "+literal plus"]
        ));
    }

    #[test]
    fn rejects_malformed_grammar_deterministically() {
        let cases = [
            "",
            "PUT END:\n+x",
            "[a#123]\nREM 1",
            "[a#GGGG]\nREM 1",
            "[a#1234]\n",
            "[a#1234]\nPUT END:\n",
            "[a#1234]\nPUT 2-3:\n+x",
            "[a#1234]\nCUT 3.=2",
            "[a#1234]\nREM 0",
            "[a#1234]\nMV 1.=2 TO 3",
            "[a#1234]\n\nREM 1",
            "[a#1234]\nREM 1:",
        ];
        for case in cases {
            assert!(
                matches!(parse_anchored_edit(case), Err(EditError::Parse { .. })),
                "{case:?}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_and_traversing_paths() {
        assert_eq!(
            parse_anchored_edit("[a#1234]\nREM 1\n[a#1234]\nREM 2"),
            Err(EditError::DuplicatePath { path: "a".into() })
        );
        for path in [
            "../a",
            "a/../b",
            "/etc/passwd",
            "C:/boot.ini",
            "C:boot.ini",
            "a\\b",
            "a//b",
        ] {
            assert!(matches!(
                parse_anchored_edit(&format!("[{path}#1234]\nREM 1")),
                Err(EditError::UnsafePath { .. })
            ));
        }
    }

    #[test]
    fn repeated_hashes_in_a_path_use_the_last_hash_as_tag_separator() {
        let parsed = parse_anchored_edit("[dir/#name#1234]\nREM 1").unwrap();
        assert_eq!(parsed.files[0].path, "dir/#name");
    }
}
