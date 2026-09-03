use std::io::Write;

use serde::Serialize;
use serde_json::Value;

use crate::{DEFAULT_MAX_PAYLOAD_BYTES, DapError, Event, Request, Response, Result, encode_frame};

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Request(Request),
    Response(Response),
    Event(Event),
}

pub fn decode_message(payload: &[u8]) -> Result<Message> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| DapError::InvalidJson(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| DapError::InvalidMessage("expected a JSON object".to_owned()))?;
    let seq = object
        .get("seq")
        .and_then(Value::as_i64)
        .ok_or_else(|| DapError::InvalidMessage("missing integer seq".to_owned()))?;
    if seq <= 0 {
        return Err(DapError::InvalidMessage("seq must be positive".to_owned()));
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| DapError::InvalidMessage("missing string type".to_owned()))?;
    let message = match kind {
        "request" => Message::Request(serde_json::from_value(value).map_err(invalid_message)?),
        "response" => Message::Response(serde_json::from_value(value).map_err(invalid_message)?),
        "event" => Message::Event(serde_json::from_value(value).map_err(invalid_message)?),
        other => {
            return Err(DapError::InvalidMessage(format!(
                "unknown DAP message type: {other}"
            )));
        }
    };
    validate_identifiers(&message)?;
    Ok(message)
}

pub fn encode_message(message: &impl Serialize) -> Result<Vec<u8>> {
    let mut payload = BoundedPayload::new(DEFAULT_MAX_PAYLOAD_BYTES);
    if let Err(error) = serde_json::to_writer(&mut payload, message) {
        if payload.observed > payload.limit {
            return Err(DapError::PayloadTooLarge {
                observed: payload.observed,
                limit: payload.limit,
            });
        }
        return Err(DapError::InvalidJson(error.to_string()));
    }
    let payload = payload.bytes;
    decode_message(&payload)?;
    Ok(encode_frame(&payload))
}

struct BoundedPayload {
    bytes: Vec<u8>,
    limit: usize,
    observed: usize,
}

impl BoundedPayload {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            observed: 0,
        }
    }
}

impl Write for BoundedPayload {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.observed = self.observed.saturating_add(buffer.len());
        if self.observed > self.limit {
            return Err(std::io::Error::other("DAP payload limit exceeded"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn invalid_message(error: serde_json::Error) -> DapError {
    DapError::InvalidMessage(error.to_string())
}

fn validate_identifiers(message: &Message) -> Result<()> {
    match message {
        Message::Request(request) if request.command.is_empty() => Err(DapError::InvalidMessage(
            "request command must not be empty".to_owned(),
        )),
        Message::Response(response) if response.request_seq <= 0 => Err(DapError::InvalidMessage(
            "response request_seq must be positive".to_owned(),
        )),
        Message::Response(response) if response.command.is_empty() => Err(
            DapError::InvalidMessage("response command must not be empty".to_owned()),
        ),
        Message::Event(event) if event.event.is_empty() => Err(DapError::InvalidMessage(
            "event identifier must not be empty".to_owned(),
        )),
        _ => Ok(()),
    }
}
