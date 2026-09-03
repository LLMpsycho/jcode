use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, broadcast, oneshot};

use crate::{
    DapError, Event, FrameDecoder, Message, Request, Response, Result, decode_message,
    encode_message,
};

type BoxWriter = Pin<Box<dyn AsyncWrite + Send>>;
const MAX_PENDING_REQUESTS: usize = 1024;

struct PendingRequest {
    command: String,
    sender: oneshot::Sender<Result<Response>>,
}
type Pending = Arc<Mutex<HashMap<i64, PendingRequest>>>;

#[derive(Clone)]
pub struct DapClient {
    writer: Arc<Mutex<BoxWriter>>,
    pending: Pending,
    events: broadcast::Sender<Event>,
    reverse_requests: broadcast::Sender<Request>,
    next_seq: Arc<AtomicI64>,
    supports_cancel: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}

impl DapClient {
    pub fn start<T>(transport: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (reader, writer) = tokio::io::split(transport);
        Self::start_split(reader, writer)
    }

    pub fn start_split<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let writer: Arc<Mutex<BoxWriter>> = Arc::new(Mutex::new(Box::pin(writer)));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(128);
        let (reverse_requests, _) = broadcast::channel(32);
        let next_seq = Arc::new(AtomicI64::new(1));
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(read_loop(
            reader,
            Arc::clone(&writer),
            Arc::clone(&pending),
            events.clone(),
            reverse_requests.clone(),
            Arc::clone(&next_seq),
            Arc::clone(&closed),
        ));
        Self {
            writer,
            pending,
            events,
            reverse_requests,
            next_seq,
            supports_cancel: Arc::new(AtomicBool::new(false)),
            closed,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
    pub fn subscribe_reverse_requests(&self) -> broadcast::Receiver<Request> {
        self.reverse_requests.subscribe()
    }
    pub fn set_supports_cancel_request(&self, supported: bool) {
        self.supports_cancel.store(supported, Ordering::Release);
    }

    pub async fn request(
        &self,
        command: impl Into<String>,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let command = command.into();
        if self.closed.load(Ordering::Acquire) {
            return Err(DapError::TransportClosed);
        }
        let seq = next_sequence(&self.next_seq)?;
        let request = Request::new(seq, command.clone(), arguments);
        let (sender, receiver) = oneshot::channel();
        let mut pending = self.pending.lock().await;
        if pending.len() >= MAX_PENDING_REQUESTS {
            return Err(DapError::InvalidMessage(
                "too many pending DAP requests".to_owned(),
            ));
        }
        pending.insert(
            seq,
            PendingRequest {
                command: command.clone(),
                sender,
            },
        );
        drop(pending);
        if let Err(error) = write_message(&self.writer, &request).await {
            self.pending.lock().await.remove(&seq);
            return Err(error);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(DapError::TransportClosed),
            Err(_) => {
                self.pending.lock().await.remove(&seq);
                if self.supports_cancel.load(Ordering::Acquire) {
                    let _best_effort = self.send_cancel(seq).await;
                }
                Err(DapError::RequestTimeout { command })
            }
        }
    }

    async fn send_cancel(&self, request_id: i64) -> Result<()> {
        let seq = next_sequence(&self.next_seq)?;
        write_message(
            &self.writer,
            &Request::new(seq, "cancel", Some(json!({ "requestId": request_id }))),
        )
        .await
    }
}

fn next_sequence(counter: &AtomicI64) -> Result<i64> {
    let seq = counter.fetch_add(1, Ordering::Relaxed);
    if seq <= 0 {
        return Err(DapError::InvalidMessage(
            "client sequence exhausted".to_owned(),
        ));
    }
    Ok(seq)
}

async fn write_message(
    writer: &Arc<Mutex<BoxWriter>>,
    message: &impl serde::Serialize,
) -> Result<()> {
    let frame = encode_message(message)?;
    let mut writer = writer.lock().await;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_loop<R>(
    mut reader: R,
    writer: Arc<Mutex<BoxWriter>>,
    pending: Pending,
    events: broadcast::Sender<Event>,
    reverse_requests: broadcast::Sender<Request>,
    next_seq: Arc<AtomicI64>,
    closed: Arc<AtomicBool>,
) where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut decoder = FrameDecoder::default();
    let mut buffer = [0_u8; 8192];
    let terminal_error = 'transport: loop {
        let count = match reader.read(&mut buffer).await {
            Ok(0) => break DapError::TransportClosed,
            Ok(count) => count,
            Err(error) => break error.into(),
        };
        let frames = match decoder.push(&buffer[..count]) {
            Ok(frames) => frames,
            Err(error) => break error,
        };
        for frame in frames {
            let message = match decode_message(&frame) {
                Ok(message) => message,
                Err(error) => break 'transport error,
            };
            match message {
                Message::Response(response) => {
                    if let Some(pending_request) =
                        pending.lock().await.remove(&response.request_seq)
                    {
                        let result = if response.command != pending_request.command {
                            Err(DapError::InvalidMessage(format!(
                                "response command mismatch: expected {}, got {}",
                                pending_request.command, response.command
                            )))
                        } else if response.success {
                            Ok(response)
                        } else {
                            Err(DapError::Response {
                                command: response.command.clone(),
                                message: response
                                    .message
                                    .clone()
                                    .unwrap_or_else(|| "adapter returned an error".to_owned()),
                            })
                        };
                        let _ignored = pending_request.sender.send(result);
                    }
                }
                Message::Event(event) => {
                    let _ignored = events.send(event);
                }
                Message::Request(request) => {
                    let _ignored = reverse_requests.send(request.clone());
                    let response = match next_sequence(&next_seq) {
                        Ok(seq) => Response::error(
                            seq,
                            request.seq,
                            request.command,
                            "reverse requests are not supported",
                        ),
                        Err(error) => {
                            fail_pending(&pending, error).await;
                            return;
                        }
                    };
                    if let Err(error) = write_message(&writer, &response).await {
                        break 'transport error;
                    }
                }
            }
        }
    };
    closed.store(true, Ordering::Release);
    fail_pending(&pending, terminal_error).await;
}

async fn fail_pending(pending: &Pending, error: DapError) {
    let senders = pending
        .lock()
        .await
        .drain()
        .map(|(_, pending)| pending.sender)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ignored = sender.send(Err(error.clone()));
    }
}
