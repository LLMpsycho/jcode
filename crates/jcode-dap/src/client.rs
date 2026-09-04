use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Sleep};

use crate::{
    DapError, Event, FrameDecoder, Message, Request, Response, Result, decode_message,
    encode_message,
};

#[cfg(test)]
mod tests;
mod transaction;

use self::transaction::{
    AdmissionObserver, AdmissionPhase, ClientInstance, RequestTransaction, Settlement,
    SettlementReason, TransactionSnapshot,
};

const MAX_PENDING_REQUESTS: usize = 1024;
const WRITER_QUEUE_CAPACITY: usize = 32;
const REVERSE_RESPONSE_QUEUE_CAPACITY: usize = 32;
pub const EVENT_CHANNEL_CAPACITY: usize = 128;
pub const MAX_RETAINED_EVENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RETAINED_EVENT_SIZE: usize = MAX_RETAINED_EVENT_BYTES / EVENT_CHANNEL_CAPACITY;

struct PendingRequest {
    client_instance: ClientInstance,
    command: String,
    sender: oneshot::Sender<Result<Response>>,
    transaction: Arc<RequestTransaction>,
}
type Pending = Arc<Mutex<HashMap<i64, PendingRequest>>>;

struct WriteCommand {
    frame: Vec<u8>,
    completed: Option<oneshot::Sender<Result<()>>>,
}

struct Shared {
    client_instance: ClientInstance,
    pending: Pending,
    writer: mpsc::Sender<WriteCommand>,
    closed: AtomicBool,
    shutdown: watch::Sender<bool>,
    next_seq: AtomicI64,
    enqueue: Mutex<()>,
    serializer: Arc<Semaphore>,
    decoder: Arc<Semaphore>,
    status: watch::Sender<DapClientStatus>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DapClientStatus {
    pub closed: bool,
    pub close_cause: Option<ClientCloseCause>,
    pub dropped_output_events: u64,
    pub dropped_non_output_events: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientCloseCause {
    ExplicitClose,
    ReaderEof,
    ReadFailure,
    WriteFailure,
    ProtocolFailure,
    SequenceExhausted,
}

struct ClientInner {
    shared: Arc<Shared>,
    writer: Mutex<Option<mpsc::Sender<WriteCommand>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    events: broadcast::Sender<Event>,
    #[allow(dead_code)]
    reverse_requests: broadcast::Sender<Request>,
    supports_cancel: AtomicBool,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        close_inner(self);
    }
}

#[derive(Clone)]
pub(crate) struct DapClient {
    inner: Arc<ClientInner>,
}

pub(crate) struct TrackedRequest {
    shared: Arc<Shared>,
    transaction: Arc<RequestTransaction>,
    command: String,
    driver: Pin<Box<dyn Future<Output = Result<Response>> + Send>>,
    deadline: Pin<Box<Sleep>>,
    completed: bool,
}

pub(crate) type AdmissionGate = Box<dyn FnOnce() -> bool + Send + 'static>;

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct TrackedRequestInvalidator {
    shared: Arc<Shared>,
    transaction: Arc<RequestTransaction>,
}

impl TrackedRequest {
    #[allow(dead_code)]
    pub(crate) fn admission_observer(&self) -> AdmissionObserver {
        self.transaction.observer()
    }

    #[allow(dead_code)]
    pub(crate) fn invalidator(&self) -> TrackedRequestInvalidator {
        TrackedRequestInvalidator {
            shared: Arc::clone(&self.shared),
            transaction: Arc::clone(&self.transaction),
        }
    }
}

impl TrackedRequestInvalidator {
    #[allow(dead_code)]
    pub(crate) fn invalidate(&self) -> TransactionSnapshot {
        settle_transaction(
            &self.shared,
            &self.transaction,
            SettlementReason::Invalidated,
        );
        self.transaction.snapshot()
    }

    #[allow(dead_code)]
    pub(crate) fn admission_phase(&self) -> AdmissionPhase {
        self.transaction.snapshot().admission_phase
    }
}

impl Future for TrackedRequest {
    type Output = Result<Response>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(result) = self.driver.as_mut().poll(cx) {
            if result.is_err() {
                settle_transaction(
                    &self.shared,
                    &self.transaction,
                    SettlementReason::QueueFailure,
                );
            }
            self.completed = true;
            return Poll::Ready(result);
        }
        if self.deadline.as_mut().poll(cx).is_ready() {
            let settlement =
                settle_transaction(&self.shared, &self.transaction, SettlementReason::Deadline);
            if settlement.won {
                self.completed = true;
                return Poll::Ready(Err(DapError::RequestTimeout {
                    command: self.command.clone(),
                }));
            }
            return Poll::Pending;
        }
        Poll::Pending
    }
}

impl Drop for TrackedRequest {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        settle_transaction(
            &self.shared,
            &self.transaction,
            SettlementReason::CallerDrop,
        );
    }
}

impl DapClient {
    pub(crate) fn is_exact(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    #[cfg(test)]
    pub(crate) fn start<T>(transport: T) -> Self
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
            client_instance: ClientInstance::new(),
            pending,
            writer: writer_tx.clone(),
            closed: AtomicBool::new(false),
            shutdown,
            next_seq: AtomicI64::new(1),
            enqueue: Mutex::new(()),
            serializer: Arc::new(Semaphore::new(1)),
            decoder: Arc::new(Semaphore::new(1)),
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

    #[cfg(test)]
    pub(crate) fn subscribe_reverse_requests(&self) -> broadcast::Receiver<Request> {
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

    pub(crate) fn status(&self) -> DapClientStatus {
        *self.inner.shared.status.borrow()
    }

    pub(crate) fn synchronize_admission<T>(&self, action: impl FnOnce() -> T) -> T {
        let _enqueue = lock_enqueue(&self.inner.shared.enqueue);
        action()
    }

    pub async fn request(
        &self,
        command: impl Into<String>,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let command = command.into();
        self.tracked_request(command, arguments, timeout)?.await
    }

    pub(crate) fn tracked_request(
        &self,
        command: impl Into<String>,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<TrackedRequest> {
        self.tracked_request_with_admission_gate(command, arguments, timeout, Box::new(|| true))
    }

    pub(crate) fn tracked_request_with_admission_gate(
        &self,
        command: impl Into<String>,
        arguments: Option<Value>,
        timeout: Duration,
        admission_gate: AdmissionGate,
    ) -> Result<TrackedRequest> {
        let command = command.into();
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(DapError::InvalidRequestTimeout)?;
        let shared = Arc::clone(&self.inner.shared);
        let writer = lock_writer(&self.inner.writer)
            .clone()
            .ok_or(DapError::TransportClosed)?;
        self.ensure_open()?;
        let transaction = RequestTransaction::new(
            shared.client_instance.clone(),
            self.inner.supports_cancel.load(Ordering::Acquire),
        );
        let driver_transaction = Arc::clone(&transaction);
        let driver_shared = Arc::clone(&shared);
        let driver_command = command.clone();
        let driver = Box::pin(async move {
            let serializer = Arc::clone(&driver_shared.serializer)
                .acquire_owned()
                .await
                .map_err(|_| DapError::TransportClosed)?;
            ensure_shared_open(&driver_shared)?;
            let seq = allocate_sequence(&driver_shared)?;
            let message = Request::new(seq, driver_command.clone(), arguments);
            let (frame, serializer) = tokio::task::spawn_blocking(move || {
                encode_message(&message).map(|frame| (frame, serializer))
            })
            .await
            .map_err(|_| {
                DapError::InvalidMessage("DAP request serialization task failed".to_owned())
            })??;
            let permit = match writer.reserve_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    settle_transaction(
                        &driver_shared,
                        &driver_transaction,
                        SettlementReason::QueueFailure,
                    );
                    return Err(DapError::TransportClosed);
                }
            };
            let (sender, receiver) = oneshot::channel();
            let mut admission_gate = Some(admission_gate);
            let mut frame = Some(frame);
            let mut permit = Some(permit);
            let mut sender = Some(sender);
            let mut admission_error = None;
            let mut admission_reason = SettlementReason::QueueFailure;
            if !driver_transaction.commit_admission(seq, || {
                let _enqueue = lock_enqueue(&driver_shared.enqueue);
                if let Err(error) = ensure_shared_open(&driver_shared) {
                    admission_error = Some(error);
                    return false;
                }
                if lock_pending(&driver_shared.pending).len() >= MAX_PENDING_REQUESTS {
                    admission_error = Some(DapError::InvalidMessage(
                        "too many pending DAP requests".to_owned(),
                    ));
                    return false;
                }
                if !admission_gate.take().is_some_and(|gate| gate()) {
                    admission_error = Some(DapError::TransportClosed);
                    admission_reason = SettlementReason::Invalidated;
                    return false;
                }
                let (Some(sender), Some(permit), Some(frame)) =
                    (sender.take(), permit.take(), frame.take())
                else {
                    admission_error = Some(DapError::InvalidMessage(
                        "DAP request admission state unavailable".to_owned(),
                    ));
                    return false;
                };
                let mut pending = lock_pending(&driver_shared.pending);
                pending.insert(
                    seq,
                    PendingRequest {
                        client_instance: driver_shared.client_instance.clone(),
                        command: driver_command.clone(),
                        sender,
                        transaction: Arc::clone(&driver_transaction),
                    },
                );
                permit.send(WriteCommand {
                    frame,
                    completed: None,
                });
                true
            }) {
                settle_transaction(&driver_shared, &driver_transaction, admission_reason);
                return Err(admission_error.unwrap_or(DapError::TransportClosed));
            }
            drop(serializer);
            receiver.await.unwrap_or(Err(DapError::TransportClosed))
        });
        Ok(TrackedRequest {
            shared,
            transaction,
            command,
            driver,
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
            completed: false,
        })
    }

    fn ensure_open(&self) -> Result<()> {
        if self.inner.shared.closed.load(Ordering::Acquire) {
            Err(DapError::TransportClosed)
        } else {
            Ok(())
        }
    }
}

fn close_inner(inner: &ClientInner) {
    if let Some(pending) =
        begin_transport_termination(&inner.shared, ClientCloseCause::ExplicitClose)
    {
        fail_pending(pending, DapError::TransportClosed);
    }
    lock_writer(&inner.writer).take();
    for task in lock_tasks(&inner.tasks).drain(..) {
        task.abort();
    }
}

fn next_sequence(counter: &AtomicI64) -> Result<i64> {
    let maximum = i64::from(i32::MAX);
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            if !(1..=maximum).contains(&current) {
                return None;
            }
            Some(if current == maximum { 0 } else { current + 1 })
        })
        .map_err(|_| DapError::InvalidMessage("client sequence exhausted".to_owned()))
}

fn allocate_sequence(shared: &Shared) -> Result<i64> {
    match next_sequence(&shared.next_seq) {
        Ok(sequence) => Ok(sequence),
        Err(error) => {
            terminate_transport(shared, error.clone(), ClientCloseCause::SequenceExhausted);
            Err(error)
        }
    }
}

fn ensure_shared_open(shared: &Shared) -> Result<()> {
    if shared.closed.load(Ordering::Acquire) {
        Err(DapError::TransportClosed)
    } else {
        Ok(())
    }
}

fn settle_transaction(
    shared: &Shared,
    transaction: &Arc<RequestTransaction>,
    reason: SettlementReason,
) -> Settlement {
    let mut terminal_error = None;
    let settlement = transaction.settle_with(reason, |pending_seq, cancel_request_seq| {
        let cancel = cancel_request_seq.and_then(|request_id| {
            let serializer = match Arc::clone(&shared.serializer).try_acquire_owned() {
                Ok(serializer) => serializer,
                Err(_) => return None,
            };
            let permit = match shared.writer.clone().try_reserve_owned() {
                Ok(permit) => permit,
                Err(_) => return None,
            };
            Some((request_id, serializer, permit))
        });
        let _enqueue = lock_enqueue(&shared.enqueue);
        if let Some(sequence) = pending_seq {
            let mut pending = lock_pending(&shared.pending);
            if pending.get(&sequence).is_some_and(|request| {
                request.client_instance.is_exact(transaction.instance())
                    && Arc::ptr_eq(&request.transaction, transaction)
            }) {
                pending.remove(&sequence);
            }
        }
        let Some((request_id, serializer, permit)) = cancel else {
            return;
        };
        if ensure_shared_open(shared).is_err() {
            return;
        }
        let sequence = match next_sequence(&shared.next_seq) {
            Ok(sequence) => sequence,
            Err(error) => {
                terminal_error = Some(error);
                return;
            }
        };
        let frame = match encode_message(&Request::new(
            sequence,
            "cancel",
            Some(json!({ "requestId": request_id })),
        )) {
            Ok(frame) => frame,
            Err(error) => {
                terminal_error = Some(error);
                return;
            }
        };
        permit.send(WriteCommand {
            frame,
            completed: None,
        });
        drop(serializer);
    });
    if let Some(error) = terminal_error {
        terminate_transport(shared, error, ClientCloseCause::SequenceExhausted);
    }
    settlement
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
            terminate_transport(&shared, error, ClientCloseCause::WriteFailure);
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
    let mut inbound_sequence = 0_i64;
    let mut buffer = [0_u8; 8192];
    let mut deferred_terminal = None;
    let (terminal_error, terminal_cause) = 'transport: loop {
        if let Some(terminal) = deferred_terminal.take() {
            break terminal;
        }
        let count = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            result = reader.read(&mut buffer) => match result {
                Ok(0) => break (DapError::TransportClosed, ClientCloseCause::ReaderEof),
                Ok(count) => count,
                Err(error) => break (error.into(), ClientCloseCause::ReadFailure),
            },
        };
        let mut frames = match decoder.push(&buffer[..count]) {
            Ok(frames) => frames,
            Err(error) => break (error, ClientCloseCause::ProtocolFailure),
        };
        for _ in 0..32 {
            match tokio::time::timeout(Duration::from_micros(100), reader.read(&mut buffer)).await {
                Ok(Ok(0)) => {
                    deferred_terminal =
                        Some((DapError::TransportClosed, ClientCloseCause::ReaderEof));
                    break;
                }
                Ok(Ok(count)) => match decoder.push(&buffer[..count]) {
                    Ok(more) => frames.extend(more),
                    Err(error) => {
                        deferred_terminal = Some((error, ClientCloseCause::ProtocolFailure));
                        break;
                    }
                },
                Ok(Err(error)) => {
                    deferred_terminal = Some((error.into(), ClientCloseCause::ReadFailure));
                    break;
                }
                Err(_) => break,
            }
        }
        if frames.is_empty() {
            continue;
        }
        let permit = match Arc::clone(&shared.decoder).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                break 'transport (DapError::TransportClosed, ClientCloseCause::ProtocolFailure);
            }
        };
        let messages = match tokio::task::spawn_blocking(move || {
            let _permit = permit;
            frames
                .into_iter()
                .map(|frame| decode_message(&frame).map(|message| (frame.len(), message)))
                .collect::<Result<Vec<_>>>()
        })
        .await
        {
            Ok(Ok(messages)) => messages,
            Ok(Err(error)) => break 'transport (error, ClientCloseCause::ProtocolFailure),
            Err(_) => {
                break 'transport (
                    DapError::InvalidMessage("DAP decode task failed".to_owned()),
                    ClientCloseCause::ProtocolFailure,
                );
            }
        };
        for (frame_len, mut message) in messages {
            if let Err(error) = normalize_inbound_sequence(&mut message, &mut inbound_sequence) {
                break 'transport (error, ClientCloseCause::ProtocolFailure);
            }
            match message {
                Message::Response(response) => handle_response(&shared, response),
                Message::Event(event) => {
                    if frame_len <= MAX_RETAINED_EVENT_SIZE {
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
                        break 'transport (
                            DapError::TransportClosed,
                            ClientCloseCause::ProtocolFailure,
                        );
                    }
                }
            }
        }
    };
    terminate_transport(&shared, terminal_error, terminal_cause);
}

fn normalize_inbound_sequence(message: &mut Message, last_sequence: &mut i64) -> Result<()> {
    let sequence = match message {
        Message::Request(request) => &mut request.seq,
        Message::Response(response) => &mut response.seq,
        Message::Event(event) => &mut event.seq,
    };
    if *sequence == 0 {
        *last_sequence = last_sequence
            .checked_add(1)
            .filter(|value| *value <= i64::from(i32::MAX))
            .ok_or_else(|| DapError::InvalidMessage("adapter sequence exhausted".to_owned()))?;
        *sequence = *last_sequence;
    } else {
        *last_sequence = (*last_sequence).max(*sequence);
    }
    Ok(())
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
        let serializer = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            serializer = Arc::clone(&shared.serializer).acquire_owned() => match serializer {
                Ok(serializer) => serializer,
                Err(_) => return,
            },
        };
        if shared.closed.load(Ordering::Acquire) {
            return;
        }
        let permit = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            permit = writer.clone().reserve_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        let seq = match allocate_sequence(&shared) {
            Ok(seq) => seq,
            Err(_) => return,
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
                terminate_transport(&shared, error, ClientCloseCause::SequenceExhausted);
                return;
            }
        };
        let _enqueue = lock_enqueue(&shared.enqueue);
        if shared.closed.load(Ordering::Acquire) {
            return;
        }
        permit.send(WriteCommand {
            frame,
            completed: None,
        });
        drop(serializer);
    }
}

fn handle_response(shared: &Shared, response: Response) {
    let transaction = {
        let pending = lock_pending(&shared.pending);
        let Some(request) = pending.get(&response.request_seq) else {
            return;
        };
        if !request.client_instance.is_exact(&shared.client_instance) {
            return;
        }
        Arc::clone(&request.transaction)
    };
    let mut pending_request = None;
    if !transaction.route_response(&shared.client_instance, response.request_seq, || {
        let mut pending = lock_pending(&shared.pending);
        if pending.get(&response.request_seq).is_some_and(|request| {
            request.client_instance.is_exact(&shared.client_instance)
                && Arc::ptr_eq(&request.transaction, &transaction)
        }) {
            pending_request = pending.remove(&response.request_seq);
            true
        } else {
            false
        }
    }) {
        return;
    };
    if let Some(pending_request) = pending_request {
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
        pending_request.transaction.settle_response();
        let _ignored = pending_request.sender.send(result);
    }
}

fn terminate_transport(shared: &Shared, error: DapError, cause: ClientCloseCause) {
    if let Some(pending) = begin_transport_termination(shared, cause) {
        fail_pending(pending, error);
    }
}

fn begin_transport_termination(
    shared: &Shared,
    cause: ClientCloseCause,
) -> Option<Vec<PendingRequest>> {
    begin_transport_termination_after_contention(shared, cause, || {})
}

fn begin_transport_termination_after_contention(
    shared: &Shared,
    cause: ClientCloseCause,
    on_contention: impl FnOnce(),
) -> Option<Vec<PendingRequest>> {
    let _enqueue = match shared.enqueue.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            on_contention();
            lock_enqueue(&shared.enqueue)
        }
    };
    if !shared.closed.swap(true, Ordering::AcqRel) {
        shared.status.send_modify(|status| {
            status.closed = true;
            status.close_cause = Some(cause);
        });
        let pending = lock_pending(&shared.pending)
            .drain()
            .map(|(_, pending)| pending)
            .collect();
        let _ignored = shared.shutdown.send(true);
        Some(pending)
    } else {
        None
    }
}

fn fail_pending(pending_requests: Vec<PendingRequest>, error: DapError) {
    for pending in pending_requests {
        pending.transaction.settle_transport_failure();
        let _ignored = pending.sender.send(Err(error.clone()));
    }
}

fn lock_pending(pending: &Pending) -> MutexGuard<'_, HashMap<i64, PendingRequest>> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_enqueue(enqueue: &Mutex<()>) -> MutexGuard<'_, ()> {
    enqueue
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
