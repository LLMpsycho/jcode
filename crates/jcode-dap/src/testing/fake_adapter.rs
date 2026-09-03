use std::collections::VecDeque;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use crate::{
    DapClient, DapError, Event, FrameDecoder, Message, Request, Response, Result, decode_message,
    encode_message,
};

pub struct FakeAdapter {
    stream: DuplexStream,
    decoder: FrameDecoder,
    queued: VecDeque<Message>,
    next_seq: i64,
}

impl FakeAdapter {
    pub fn pair(capacity: usize) -> (DapClient, Self) {
        let (client, adapter) = tokio::io::duplex(capacity);
        (
            DapClient::start(client),
            Self {
                stream: adapter,
                decoder: FrameDecoder::default(),
                queued: VecDeque::new(),
                next_seq: 1,
            },
        )
    }

    pub async fn recv(&mut self) -> Result<Message> {
        if let Some(message) = self.queued.pop_front() {
            return Ok(message);
        }
        let mut buffer = [0_u8; 4096];
        loop {
            let count = self.stream.read(&mut buffer).await?;
            if count == 0 {
                return Err(DapError::TransportClosed);
            }
            for frame in self.decoder.push(&buffer[..count])? {
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

    pub async fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn respond_ok(&mut self, request: &Request, body: Option<Value>) -> Result<()> {
        let seq = self.take_seq();
        self.send(&Response::success(
            seq,
            request.seq,
            request.command.clone(),
            body,
        ))
        .await
    }

    pub async fn respond_error(&mut self, request: &Request, message: &str) -> Result<()> {
        let seq = self.take_seq();
        self.send(&Response::error(
            seq,
            request.seq,
            request.command.clone(),
            message,
        ))
        .await
    }

    pub async fn event(&mut self, event: &str, body: Option<Value>) -> Result<()> {
        let seq = self.take_seq();
        self.send(&Event::new(seq, event, body)).await
    }

    pub async fn reverse_request(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<i64> {
        let seq = self.take_seq();
        self.send(&Request::new(seq, command, arguments)).await?;
        Ok(seq)
    }

    fn take_seq(&mut self) -> i64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }
}
