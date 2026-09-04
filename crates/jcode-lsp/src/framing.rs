use crate::{LspError, Result};

pub const DEFAULT_MAX_HEADER_BYTES: usize = 8 * 1024;
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    max_header_bytes: usize,
    max_payload_bytes: usize,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_HEADER_BYTES, DEFAULT_MAX_PAYLOAD_BYTES)
    }
}

impl FrameDecoder {
    pub fn new(max_header_bytes: usize, max_payload_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_header_bytes,
            max_payload_bytes,
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();

        loop {
            let Some(header_end) = find_header_end(&self.buffer) else {
                if self.buffer.len() > self.max_header_bytes {
                    return Err(LspError::HeaderTooLarge {
                        limit: self.max_header_bytes,
                    });
                }
                return Ok(frames);
            };

            if header_end > self.max_header_bytes {
                return Err(LspError::HeaderTooLarge {
                    limit: self.max_header_bytes,
                });
            }
            let content_length = parse_content_length(&self.buffer[..header_end])?;
            if content_length > self.max_payload_bytes {
                return Err(LspError::PayloadTooLarge {
                    observed: content_length,
                    limit: self.max_payload_bytes,
                });
            }

            let payload_start = header_end + 4;
            let frame_end =
                payload_start
                    .checked_add(content_length)
                    .ok_or(LspError::PayloadTooLarge {
                        observed: usize::MAX,
                        limit: self.max_payload_bytes,
                    })?;
            if self.buffer.len() < frame_end {
                return Ok(frames);
            }

            frames.push(self.buffer[payload_start..frame_end].to_vec());
            self.buffer.drain(..frame_end);
        }
    }
}

pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    let mut frame = Vec::with_capacity(header.len() + payload.len());
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(header: &[u8]) -> Result<usize> {
    let header = std::str::from_utf8(header).map_err(|_| LspError::InvalidHeaderEncoding)?;
    if !header.is_ascii() {
        return Err(LspError::InvalidHeaderEncoding);
    }

    let mut content_length = None;
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            return Err(LspError::InvalidContentLength {
                value: line.to_owned(),
            });
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(LspError::DuplicateContentLength);
            }
            let value = value.trim();
            let parsed = value
                .parse::<usize>()
                .map_err(|_| LspError::InvalidContentLength {
                    value: value.to_owned(),
                })?;
            content_length = Some(parsed);
        }
    }
    content_length.ok_or(LspError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_frame_split_at_every_byte_boundary() {
        let payload = br#"{"jsonrpc":"2.0","method":"initialized"}"#;
        let encoded = encode_frame(payload);
        for split in 0..encoded.len() {
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(&encoded[..split]).unwrap().is_empty());
            let frames = decoder.push(&encoded[split..]).unwrap();
            assert_eq!(frames, vec![payload.to_vec()], "split at {split}");
            assert_eq!(decoder.buffered_bytes(), 0);
        }
    }

    #[test]
    fn decodes_multiple_frames_and_retains_an_incomplete_tail() {
        let first = encode_frame(b"one");
        let second = encode_frame(b"two");
        let third = encode_frame(b"three");
        let mut input = [first, second, third[..third.len() - 2].to_vec()].concat();
        let mut decoder = FrameDecoder::default();
        assert_eq!(
            decoder.push(&input).unwrap(),
            vec![b"one".to_vec(), b"two".to_vec()]
        );
        input.clear();
        assert_eq!(decoder.push(b"ee").unwrap(), vec![b"three".to_vec()]);
    }

    #[test]
    fn accepts_case_insensitive_header_and_ignores_content_type() {
        let frame = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
        let mut decoder = FrameDecoder::default();
        assert_eq!(decoder.push(frame).unwrap(), vec![b"{}".to_vec()]);
    }

    #[test]
    fn rejects_missing_duplicate_invalid_and_non_ascii_lengths() {
        let cases = [
            b"Content-Type: x\r\n\r\n".as_slice(),
            b"Content-Length: 1\r\nContent-Length: 1\r\n\r\nx".as_slice(),
            b"Content-Length: -1\r\n\r\n".as_slice(),
            b"Content-Leng\xffth: 1\r\n\r\nx".as_slice(),
        ];
        for case in cases {
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(case).is_err());
        }
    }

    #[test]
    fn enforces_header_and_payload_limits_before_allocation() {
        let mut header_decoder = FrameDecoder::new(8, 64);
        assert_eq!(
            header_decoder.push(b"123456789").unwrap_err(),
            LspError::HeaderTooLarge { limit: 8 }
        );

        let mut payload_decoder = FrameDecoder::new(128, 3);
        assert_eq!(
            payload_decoder
                .push(b"Content-Length: 4\r\n\r\n")
                .unwrap_err(),
            LspError::PayloadTooLarge {
                observed: 4,
                limit: 3
            }
        );
    }
}
