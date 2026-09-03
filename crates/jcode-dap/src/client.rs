use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::{
    DapError, Event, FrameDecoder, Message, Request, Response, Result, decode_message,
    encode_message,
};

const MAX_PENDING_REQUESTS: usize = 1024;
const WRITER_QUEUE_CAPACITY: usize = 32;
const REVERSE_RESPONSE_QUEUE_CAPACITY: usize = 32;
pub const EVENT_CHANNEL_CAPACITY: usize = 128;
pub const MAX_RETAINED_EVENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RETAINED_EVENT_SIZE: usize = MAX_RETAINED_EVENT_BYTES / EVENT_CHANNEL_CAPACITY;

struct PendingRequest {
    command: String,
    sender: oneshot::Sender<Result<Response>>,
}
type Pending = Arc<Mutex<HashMap<i64, PendingRequest>>>;

struct PendingCleanup {
    pending: Pending,
    seq: i64,
}

impl PendingCleanup {
    fn remove(&mut self) {
        lock_pending(&self.pending).remove(&self.seq);
    }
}

impl Drop for PendingCleanup {
    fn drop(&mut self) {
        self.remove();
    }
}

struct WriteCommand {
    frame: Vec<u8>,
    completed: Option<oneshot::Sender<Result<()>>>,
}

struct Shared {
    pending: Pending,
    closed: AtomicBool,
    shutdown: watch::Sender<bool>,
    next_seq: AtomicI64,
    enqueue: AsyncMutex<()>,
    serializer: Arc<Semaphore>,
    status: watch::Sender<DapClientStatus>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DapClientStatus {
    pub closed: bool,
    pub dropped_output_events: u64,
    pub dropped_non_output_events: u64,
}

struct ClientInner {
    shared: Arc<Shared>,
    writer: Mutex<Option<mpsc::Sender<WriteCommand>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    events: broadcast::Sender<Event>,
    reverse_requests: broadcast::Sender<Request>,
    supports_cancel: AtomicBool,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        close_inner(self);
    }
}

#[derive(Clone)]
pub struct DapClient {
    inner: Arc<ClientInner>,
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
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (reverse_requests, _) = broadcast::channel(32);
        let (writer_tx, writer_rx) = mpsc::channel(WRITER_QUEUE_CAPACITY);
        let (reverse_response_tx, reverse_response_rx) =
            mpsc::channel(REVERSE_RESPONSE_QUEUE_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (status, _) = watch::channel(DapClientStatus::default());
        let shared = Arc::new(Shared {
            pending,
            closed: AtomicBool::new(false),
            shutdown,
            next_seq: AtomicI64::new(1),
            enqueue: AsyncMutex::new(()),
            serializer: Arc::new(Semaphore::new(1)),
            status,
        });
        let writer_task = tokio::spawn(write_loop(
            writer,
            writer_rx,
            shutdown_rx.clone(),
            Arc::clone(&shared),
        ));
        let reader_task = tokio::spawn(read_loop(
            reader,
            reverse_response_tx,
            events.clone(),
            reverse_requests.clone(),
            Arc::clone(&shared),
            shutdown_rx.clone(),
        ));
        let reverse_response_task = tokio::spawn(reverse_response_loop(
            writer_tx.clone(),
            reverse_response_rx,
            Arc::clone(&shared),
            shutdown_rx,
        ));
        Self {
            inner: Arc::new(ClientInner {
                shared,
                writer: Mutex::new(Some(writer_tx)),
                tasks: Mutex::new(vec![reader_task, writer_task, reverse_response_task]),
                events,
                reverse_requests,
                supports_cancel: AtomicBool::new(false),
            }),
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn subscribe_reverse_requests(&self) -> broadcast::Receiver<Request> {
        self.inner.reverse_requests.subscribe()
    }

    pub fn set_supports_cancel_request(&self, supported: bool) {
        self.inner
            .supports_cancel
            .store(supported, Ordering::Release);
    }

    pub fn close(&self) {
        close_inner(&self.inner);
    }

    pub(crate) fn subscribe_status(&self) -> watch::Receiver<DapClientStatus> {
        self.inner.shared.status.subscribe()
    }

    pub async fn request(
        &self,
        command: impl Into<String>,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let command = command.into();
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(DapError::InvalidRequestTimeout)?;
        self.ensure_open()?;
        let mut request_seq = None;
        let result = tokio::time::timeout_at(deadline, async {
            let serializer = Arc::clone(&self.inner.shared.serializer)
                .acquire_owned()
                .await
                .map_err(|_| DapError::TransportClosed)?;
            let _enqueue = self.inner.shared.enqueue.lock().await;
            self.ensure_open()?;
            let seq = next_sequence(&self.inner.shared.next_seq)?;
            let message = Request::new(seq, command.clone(), arguments);
            let frame = tokio::task::spawn_blocking(move || {
                let _serializer = serializer;
                encode_message(&message)
            })
            .await
            .map_err(|error| {
                DapError::InvalidMessage(format!("DAP request serialization task failed: {error}"))
            })??;
            request_seq = Some(seq);
            let (sender, receiver) = oneshot::channel();
            {
                let mut pending = lock_pending(&self.inner.shared.pending);
                self.ensure_open()?;
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
            }
            let _cleanup = PendingCleanup {
                pending: Arc::clone(&self.inner.shared.pending),
                seq,
            };
            let mut receiver = receiver;
            tokio::select! {
                write_result = self.queue_frame(frame) => {
                    write_result?;
                    drop(_enqueue);
                    match receiver.await {
                        Ok(result) => result,
                        Err(_) => Err(DapError::TransportClosed),
                    }
                }
                response = &mut receiver => {
                    drop(_enqueue);
                    match response {
                        Ok(result) => result,
                        Err(_) => Err(DapError::TransportClosed),
                    }
                }
            }
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => {
                if let Some(seq) = request_seq {
                    self.try_cancel(seq);
                }
                Err(DapError::RequestTimeout { command })
            }
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.inner.shared.closed.load(Ordering::Acquire) {
            Err(DapError::TransportClosed)
        } else {
            Ok(())
        }
    }

    async fn queue_frame(&self, frame: Vec<u8>) -> Result<()> {
        let writer = lock_writer(&self.inner.writer)
            .clone()
            .ok_or(DapError::TransportClosed)?;
        let (completed, receiver) = oneshot::channel();
        writer
            .send(WriteCommand {
                frame,
                completed: Some(completed),
            })
            .await
            .map_err(|_| DapError::TransportClosed)?;
        receiver.await.unwrap_or(Err(DapError::TransportClosed))
    }

    fn try_cancel(&self, request_id: i64) {
        if !self.inner.supports_cancel.load(Ordering::Acquire) {
            return;
        }
        let Ok(_enqueue) = self.inner.shared.enqueue.try_lock() else {
            return;
        };
        let Ok(seq) = next_sequence(&self.inner.shared.next_seq) else {
            return;
        };
        let Ok(frame) = encode_message(&Request::new(
            seq,
            "cancel",
            Some(json!({ "requestId": request_id })),
        )) else {
            return;
        };
        if let Some(writer) = lock_writer(&self.inner.writer).as_ref() {
            let _ignored = writer.try_send(WriteCommand {
                frame,
                completed: None,
            });
        }
    }
}

fn close_inner(inner: &ClientInner) {
    if !inner.shared.closed.swap(true, Ordering::AcqRel) {
        inner
            .shared
            .status
            .send_modify(|status| status.closed = true);
        fail_pending(&inner.shared.pending, DapError::TransportClosed);
        let _ignored = inner.shared.shutdown.send(true);
    }
    lock_writer(&inner.writer).take();
    for task in lock_tasks(&inner.tasks).drain(..) {
        task.abort();
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

async fn write_loop<W>(
    mut writer: W,
    mut commands: mpsc::Receiver<WriteCommand>,
    mut shutdown: watch::Receiver<bool>,
    shared: Arc<Shared>,
) where
    W: AsyncWrite + Send + Unpin + 'static,
{
    loop {
        let command = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
                continue;
            }
            command = commands.recv() => match command {
                Some(command) => command,
                None => break,
            },
        };
        let result = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
                continue;
            }
            result = async {
                writer.write_all(&command.frame).await?;
                writer.flush().await?;
                Ok(())
            } => result,
        };
        if let Some(completed) = command.completed {
            let _ignored = completed.send(result.clone());
        }
        if let Err(error) = result {
            terminate_transport(&shared, error);
            break;
        }
    }
}

async fn read_loop<R>(
    mut reader: R,
    reverse_responses: mpsc::Sender<Request>,
    events: broadcast::Sender<Event>,
    reverse_requests: broadcast::Sender<Request>,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut decoder = FrameDecoder::default();
    let mut buffer = [0_u8; 8192];
    let terminal_error = 'transport: loop {
        let count = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            result = reader.read(&mut buffer) => match result {
                Ok(0) => break DapError::TransportClosed,
                Ok(count) => count,
                Err(error) => break error.into(),
            },
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
                Message::Response(response) => handle_response(&shared.pending, response),
                Message::Event(event) => {
                    if frame.len() <= MAX_RETAINED_EVENT_SIZE {
                        let _ignored = events.send(event);
                    } else {
                        shared.status.send_modify(|status| {
                            if event.event == "output" {
                                status.dropped_output_events =
                                    status.dropped_output_events.saturating_add(1);
                            } else {
                                status.dropped_non_output_events =
                                    status.dropped_non_output_events.saturating_add(1);
                            }
                        });
                    }
                }
                Message::Request(request) => {
                    let _ignored = reverse_requests.send(request.clone());
                    if reverse_responses.try_send(request).is_err() {
                        break 'transport DapError::TransportClosed;
                    }
                }
            }
        }
    };
    terminate_transport(&shared, terminal_error);
}

async fn reverse_response_loop(
    writer: mpsc::Sender<WriteCommand>,
    mut requests: mpsc::Receiver<Request>,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let request = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            request = requests.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };
        let enqueue = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            enqueue = shared.enqueue.lock() => enqueue,
        };
        if shared.closed.load(Ordering::Acquire) {
            return;
        }
        let seq = match next_sequence(&shared.next_seq) {
            Ok(seq) => seq,
            Err(error) => {
                terminate_transport(&shared, error);
                return;
            }
        };
        let response = Response::error(
            seq,
            request.seq,
            request.command,
            "reverse requests are not supported",
        );
        let frame = match encode_message(&response) {
            Ok(frame) => frame,
            Err(error) => {
                terminate_transport(&shared, error);
                return;
            }
        };
        let result = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            result = writer.send(WriteCommand {
                frame,
                completed: None,
            }) => result,
        };
        drop(enqueue);
        if result.is_err() {
            terminate_transport(&shared, DapError::TransportClosed);
            return;
        }
    }
}

fn handle_response(pending: &Pending, response: Response) {
    if let Some(pending_request) = lock_pending(pending).remove(&response.request_seq) {
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

fn terminate_transport(shared: &Shared, error: DapError) {
    if !shared.closed.swap(true, Ordering::AcqRel) {
        shared.status.send_modify(|status| status.closed = true);
        fail_pending(&shared.pending, error);
        let _ignored = shared.shutdown.send(true);
    }
}

fn fail_pending(pending: &Pending, error: DapError) {
    let senders = lock_pending(pending)
        .drain()
        .map(|(_, pending)| pending.sender)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ignored = sender.send(Err(error.clone()));
    }
}

fn lock_pending(pending: &Pending) -> MutexGuard<'_, HashMap<i64, PendingRequest>> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_writer(
    writer: &Mutex<Option<mpsc::Sender<WriteCommand>>>,
) -> MutexGuard<'_, Option<mpsc::Sender<WriteCommand>>> {
    writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_tasks(tasks: &Mutex<Vec<JoinHandle<()>>>) -> MutexGuard<'_, Vec<JoinHandle<()>>> {
    tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
