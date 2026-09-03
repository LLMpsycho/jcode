use crate::EditError;
use serde::Serialize;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Newline style retained when rendering a planned result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// Unix line feeds, also used for empty and mixed-ending input.
    Lf,
    /// Consistent Windows carriage-return/line-feed pairs.
    Crlf,
}

/// Canonical UTF-8 text plus presentation details from the source bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedText {
    /// UTF-8 BOM-free text with LF endings and trailing spaces/tabs removed.
    pub text: String,
    /// Whether the source started with a UTF-8 BOM.
    pub had_bom: bool,
    /// Consistent source newline style, if detectable.
    pub line_ending: LineEnding,
}

impl NormalizedText {
    /// Render canonical text using the source BOM and consistent newline style.
    pub fn render(&self, text: &str) -> Vec<u8> {
        let mut rendered = String::new();
        if self.line_ending == LineEnding::Crlf {
            rendered.reserve(text.len());
            for part in text.split_inclusive('\n') {
                if let Some(without_lf) = part.strip_suffix('\n') {
                    rendered.push_str(without_lf);
                    rendered.push_str("\r\n");
                } else {
                    rendered.push_str(part);
                }
            }
        } else {
            rendered.push_str(text);
        }

        let mut bytes = Vec::with_capacity(rendered.len() + usize::from(self.had_bom) * 3);
        if self.had_bom {
            bytes.extend_from_slice(UTF8_BOM);
        }
        bytes.extend_from_slice(rendered.as_bytes());
        bytes
    }
}

/// Normalize UTF-8 file bytes for stable anchoring and digesting.
///
/// Normalization strips one leading UTF-8 BOM, converts CRLF and lone CR to LF,
/// removes spaces and tabs immediately before line endings or EOF, and
/// preserves whether the file has a final newline.
pub fn normalize_bytes(path: &str, bytes: &[u8]) -> Result<NormalizedText, EditError> {
    let (had_bom, bytes) = match bytes.strip_prefix(UTF8_BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let source = std::str::from_utf8(bytes).map_err(|_| EditError::InvalidUtf8 {
        path: path.to_owned(),
    })?;

    let line_ending = detect_line_ending(source);
    let mut normalized = String::with_capacity(source.len());
    let mut line = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                trim_line_end(&mut line);
                normalized.push_str(&line);
                normalized.push('\n');
                line.clear();
            }
            '\n' => {
                trim_line_end(&mut line);
                normalized.push_str(&line);
                normalized.push('\n');
                line.clear();
            }
            _ => line.push(ch),
        }
    }
    trim_line_end(&mut line);
    normalized.push_str(&line);

    Ok(NormalizedText {
        text: normalized,
        had_bom,
        line_ending,
    })
}

fn trim_line_end(line: &mut String) {
    let trimmed_len = line.trim_end_matches([' ', '\t']).len();
    line.truncate(trimmed_len);
}

fn detect_line_ending(source: &str) -> LineEnding {
    let bytes = source.as_bytes();
    let mut crlf = 0usize;
    let mut non_crlf = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' | b'\n' => {
                non_crlf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if crlf > 0 && non_crlf == 0 {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lf_crlf_bom_and_trailing_whitespace_normalize_identically() {
        let lf = normalize_bytes("x", b"alpha\nbeta\n").unwrap();
        let crlf = normalize_bytes("x", b"\xEF\xBB\xBFalpha \t\r\nbeta\t\r\n").unwrap();
        assert_eq!(lf.text, "alpha\nbeta\n");
        assert_eq!(lf.text, crlf.text);
        assert!(!lf.had_bom);
        assert!(crlf.had_bom);
        assert_eq!(crlf.line_ending, LineEnding::Crlf);
        assert_eq!(crlf.render(&crlf.text), b"\xEF\xBB\xBFalpha\r\nbeta\r\n");
    }

    #[test]
    fn normalization_preserves_empty_and_no_final_newline_files() {
        assert_eq!(normalize_bytes("x", b"").unwrap().text, "");
        assert_eq!(normalize_bytes("x", b"last  ").unwrap().text, "last");
        assert_eq!(normalize_bytes("x", b"last\n").unwrap().text, "last\n");
    }

    #[test]
    fn unicode_is_processed_on_character_boundaries() {
        let normalized = normalize_bytes("x", "雪☃️  \r\nλ\t".as_bytes()).unwrap();
        assert_eq!(normalized.text, "雪☃️\nλ");
    }

    #[test]
    fn mixed_and_lone_cr_endings_become_lf() {
        let normalized = normalize_bytes("x", b"a\rb\r\nc\n").unwrap();
        assert_eq!(normalized.text, "a\nb\nc\n");
        assert_eq!(normalized.line_ending, LineEnding::Lf);
    }
}
