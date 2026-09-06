use jcode_edit_types::{ContentDigest, DisplayTag, FileRevision};
use sha2::{Digest, Sha256};

use crate::{EditError, normalize_bytes};

/// Compute the full SHA-256 digest of already normalized UTF-8 text.
pub fn digest_text(text: &str) -> ContentDigest {
    let bytes: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    ContentDigest { bytes }
}

/// Normalize bytes and compute their full SHA-256 digest.
pub fn digest_bytes(path: &str, bytes: &[u8]) -> Result<ContentDigest, EditError> {
    Ok(digest_text(&normalize_bytes(path, bytes)?.text))
}

/// Derive the model-facing two-byte tag from a full digest.
pub fn display_tag(digest: ContentDigest) -> DisplayTag {
    DisplayTag {
        bytes: [digest.bytes[0], digest.bytes[1]],
    }
}

/// Format a display tag as four uppercase hexadecimal characters.
pub fn display_tag_hex(tag: DisplayTag) -> String {
    hex::encode_upper(tag.bytes)
}

/// Format a full digest as 64 lowercase hexadecimal characters.
pub fn digest_hex(digest: ContentDigest) -> String {
    hex::encode(digest.bytes)
}

/// Parse exactly four hexadecimal characters into a display tag.
pub fn parse_display_tag(value: &str) -> Option<DisplayTag> {
    if value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0u8; 2];
    if hex::decode_to_slice(value, &mut bytes).is_err() {
        // A malformed display tag is absent, never a partial digest.
        return None;
    }
    Some(DisplayTag { bytes })
}

/// Construct revision metadata for normalized text.
pub fn file_revision(revision: u64, normalized_text: &str, mtime_ns: Option<u128>) -> FileRevision {
    let content_digest = digest_text(normalized_text);
    FileRevision {
        revision,
        display_tag: display_tag(content_digest),
        content_digest,
        normalized_len: normalized_text.len() as u64,
        mtime_ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_stable_across_normalized_encodings() {
        let lf = digest_bytes("x", b"hello\nworld\n").unwrap();
        let crlf = digest_bytes("x", b"\xEF\xBB\xBFhello \r\nworld\t\r\n").unwrap();
        assert_eq!(lf, crlf);
        assert_eq!(digest_hex(lf).len(), 64);
        assert_eq!(display_tag_hex(display_tag(lf)).len(), 4);
    }

    #[test]
    fn display_tag_parsing_is_exact_and_case_insensitive() {
        assert_eq!(
            parse_display_tag("a13F"),
            Some(DisplayTag {
                bytes: [0xa1, 0x3f]
            })
        );
        assert_eq!(parse_display_tag("A13"), None);
        assert_eq!(parse_display_tag("A13FG"), None);
        assert_eq!(parse_display_tag("ZZZZ"), None);
    }

    #[test]
    fn full_digest_comparison_is_collision_safe() {
        let left = ContentDigest {
            bytes: [
                0xab, 0xcd, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0,
            ],
        };
        let right = ContentDigest {
            bytes: [
                0xab, 0xcd, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0,
            ],
        };
        assert_eq!(display_tag(left), display_tag(right));
        assert_ne!(left, right);
    }

    #[test]
    fn unicode_length_is_normalized_utf8_bytes() {
        let revision = file_revision(3, "雪\n", None);
        assert_eq!(revision.normalized_len, 4);
    }
}
