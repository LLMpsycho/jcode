use serde::Serialize;
use serde_json::Value;

use crate::{LspError, NotificationMessage, RequestMessage, ResponseMessage, Result, encode_frame};

#[derive(Clone, Debug, PartialEq)]
pub enum IncomingMessage {
    Request(RequestMessage),
    Notification(NotificationMessage),
    Response(ResponseMessage),
}

pub fn decode_message(payload: &[u8]) -> Result<IncomingMessage> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| LspError::InvalidJson(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| LspError::InvalidMessage("expected a JSON object".to_owned()))?;

    match (object.contains_key("method"), object.contains_key("id")) {
        (true, true) => serde_json::from_value(value)
            .map(IncomingMessage::Request)
            .map_err(|error| LspError::InvalidMessage(error.to_string())),
        (true, false) => serde_json::from_value(value)
            .map(IncomingMessage::Notification)
            .map_err(|error| LspError::InvalidMessage(error.to_string())),
        (false, true) => serde_json::from_value(value)
            .map(IncomingMessage::Response)
            .map_err(|error| LspError::InvalidMessage(error.to_string())),
        (false, false) => Err(LspError::InvalidMessage(
            "message has neither method nor id".to_owned(),
        )),
    }
}

pub fn encode_message(message: &impl Serialize) -> Result<Vec<u8>> {
    let payload =
        serde_json::to_vec(message).map_err(|error| LspError::InvalidJson(error.to_string()))?;
    Ok(encode_frame(&payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestId;

    #[test]
    fn classifies_requests_notifications_and_responses() {
        assert!(matches!(
            decode_message(br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).unwrap(),
            IncomingMessage::Request(_)
        ));
        assert!(matches!(
            decode_message(br#"{"jsonrpc":"2.0","method":"initialized"}"#).unwrap(),
            IncomingMessage::Notification(_)
        ));
        assert!(matches!(
            decode_message(br#"{"jsonrpc":"2.0","id":1,"result":null}"#).unwrap(),
            IncomingMessage::Response(_)
        ));
    }

    #[test]
    fn rejects_malformed_and_unclassifiable_messages() {
        assert!(matches!(
            decode_message(b"{"),
            Err(LspError::InvalidJson(_))
        ));
        assert!(matches!(
            decode_message(br#"{"jsonrpc":"2.0"}"#),
            Err(LspError::InvalidMessage(_))
        ));
    }

    #[test]
    fn serializes_a_message_inside_a_content_length_frame() {
        let request =
            RequestMessage::new(RequestId::String("request-1".to_owned()), "shutdown", None);
        let encoded = encode_message(&request).unwrap();
        let mut decoder = crate::FrameDecoder::default();
        let frames = decoder.push(&encoded).unwrap();
        assert_eq!(
            decode_message(&frames[0]).unwrap(),
            IncomingMessage::Request(request)
        );
    }
}
