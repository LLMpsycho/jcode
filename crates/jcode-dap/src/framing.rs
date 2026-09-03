use crate::{DapError, Result};

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
            let Some(header_end) = self
                .buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
            else {
                if self.buffer.len() > self.max_header_bytes {
                    return Err(DapError::HeaderTooLarge {
                        limit: self.max_header_bytes,
                    });
                }
                return Ok(frames);
            };
            if header_end > self.max_header_bytes {
                return Err(DapError::HeaderTooLarge {
                    limit: self.max_header_bytes,
                });
            }
            let content_length = parse_content_length(&self.buffer[..header_end])?;
            if content_length > self.max_payload_bytes {
                return Err(DapError::PayloadTooLarge {
                    observed: content_length,
                    limit: self.max_payload_bytes,
                });
            }
            let payload_start = header_end + 4;
            let frame_end =
                payload_start
                    .checked_add(content_length)
                    .ok_or(DapError::PayloadTooLarge {
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

fn parse_content_length(header: &[u8]) -> Result<usize> {
    let header = std::str::from_utf8(header).map_err(|_| DapError::InvalidHeaderEncoding)?;
    if !header.is_ascii() {
        return Err(DapError::InvalidHeaderEncoding);
    }
    let mut length = None;
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            return Err(DapError::InvalidContentLength {
                value: line.to_owned(),
            });
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(DapError::DuplicateContentLength);
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(DapError::InvalidContentLength {
                    value: value.to_owned(),
                });
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| DapError::InvalidContentLength {
                        value: value.to_owned(),
                    })?,
            );
        }
    }
    length.ok_or(DapError::MissingContentLength)
}
