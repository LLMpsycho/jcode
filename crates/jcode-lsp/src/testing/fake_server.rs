use std::collections::VecDeque;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use crate::{
    FrameDecoder, IncomingMessage, NotificationMessage, RequestId, RequestMessage, ResponseMessage,
    Result, decode_message, encode_message,
};

pub struct FakeLspServer {
    stream: DuplexStream,
    decoder: FrameDecoder,
    queued: VecDeque<IncomingMessage>,
}

impl FakeLspServer {
    pub fn pair(capacity: usize) -> (crate::LspClient, Self) {
        let (client_stream, server_stream) = tokio::io::duplex(capacity);
        (
            crate::LspClient::start(client_stream),
            Self {
                stream: server_stream,
                decoder: FrameDecoder::default(),
                queued: VecDeque::new(),
            },
        )
    }

    pub async fn recv(&mut self) -> Result<IncomingMessage> {
        if let Some(message) = self.queued.pop_front() {
            return Ok(message);
        }
        let mut buffer = [0_u8; 4096];
        loop {
            let count = self.stream.read(&mut buffer).await?;
            if count == 0 {
                return Err(crate::LspError::TransportClosed);
            }
            let frames = self.decoder.push(&buffer[..count])?;
            for frame in frames {
                self.queued.push_back(decode_message(&frame)?);
            }
            if let Some(message) = self.queued.pop_front() {
                return Ok(message);
            }
        }
    }

    pub async fn send(&mut self, message: &impl serde::Serialize) -> Result<()> {
        self.stream.write_all(&encode_message(message)?).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn respond_ok(&mut self, id: RequestId, result: Value) -> Result<()> {
        self.send(&ResponseMessage {
            jsonrpc: crate::JSON_RPC_VERSION.to_owned(),
            id,
            result: Some(result),
            error: None,
        })
        .await
    }

    pub async fn notify(&mut self, method: &str, params: Option<Value>) -> Result<()> {
        self.send(&NotificationMessage::new(method, params)).await
    }

    pub async fn request(
        &mut self,
        id: RequestId,
        method: &str,
        params: Option<Value>,
    ) -> Result<()> {
        self.send(&RequestMessage::new(id, method, params)).await
    }
}
