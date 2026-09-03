use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, broadcast, oneshot};

use crate::{
    FrameDecoder, IncomingMessage, LspError, NotificationMessage, RequestId, RequestMessage,
    ResponseError, ResponseMessage, Result, decode_message, encode_message,
};

type BoxWriter = Pin<Box<dyn AsyncWrite + Send>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Result<Value>>>>>;

#[derive(Clone)]
pub struct LspClient {
    writer: Arc<Mutex<BoxWriter>>,
    pending: PendingRequests,
    notifications: broadcast::Sender<NotificationMessage>,
    next_request_id: Arc<AtomicI64>,
}

impl LspClient {
    pub fn start<T>(transport: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (reader, writer) = tokio::io::split(transport);
        let writer: Arc<Mutex<BoxWriter>> = Arc::new(Mutex::new(Box::pin(writer)));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (notifications, _) = broadcast::channel(128);

        tokio::spawn(read_loop(
            reader,
            Arc::clone(&writer),
            Arc::clone(&pending),
            notifications.clone(),
        ));

        Self {
            writer,
            pending,
            notifications,
            next_request_id: Arc::new(AtomicI64::new(1)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NotificationMessage> {
        self.notifications.subscribe()
    }

    pub async fn notify(&self, method: impl Into<String>, params: Option<Value>) -> Result<()> {
        self.write(&NotificationMessage::new(method, params)).await
    }

    pub async fn request(
        &self,
        method: impl Into<String>,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let method = method.into();
        let id = RequestId::Number(self.next_request_id.fetch_add(1, Ordering::Relaxed));
        let request = RequestMessage::new(id.clone(), method.clone(), params);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);

        if let Err(error) = self.write(&request).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(LspError::TransportClosed),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                let _ = self
                    .notify("$/cancelRequest", Some(json!({ "id": id })))
                    .await;
                Err(LspError::RequestTimeout { method })
            }
        }
    }

    async fn write(&self, message: &impl serde::Serialize) -> Result<()> {
        write_message(&self.writer, message).await
    }
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
    pending: PendingRequests,
    notifications: broadcast::Sender<NotificationMessage>,
) where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut decoder = FrameDecoder::default();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let frames = match decoder.push(&buffer[..count]) {
            Ok(frames) => frames,
            Err(_) => break,
        };
        for frame in frames {
            let Ok(message) = decode_message(&frame) else {
                continue;
            };
            match message {
                IncomingMessage::Response(response) => {
                    if let Some(sender) = pending.lock().await.remove(&response.id) {
                        let result = match response.error {
                            Some(error) => Err(LspError::Response {
                                code: error.code,
                                message: error.message,
                            }),
                            None => Ok(response.result.unwrap_or(Value::Null)),
                        };
                        let _ = sender.send(result);
                    }
                }
                IncomingMessage::Notification(notification) => {
                    let _ = notifications.send(notification);
                }
                IncomingMessage::Request(request) => {
                    let response = ResponseMessage {
                        jsonrpc: crate::JSON_RPC_VERSION.to_owned(),
                        id: request.id,
                        result: None,
                        error: Some(ResponseError {
                            code: -32601,
                            message: format!("unsupported server request: {}", request.method),
                            data: None,
                        }),
                    };
                    if write_message(&writer, &response).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    let senders = pending
        .lock()
        .await
        .drain()
        .map(|(_, sender)| sender)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ = sender.send(Err(LspError::TransportClosed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeLspServer;

    #[tokio::test]
    async fn matches_out_of_order_responses_to_their_requests() {
        let (client, mut server) = FakeLspServer::pair(4096);
        let first = tokio::spawn({
            let client = client.clone();
            async move { client.request("first", None, Duration::from_secs(1)).await }
        });
        let second = tokio::spawn({
            let client = client.clone();
            async move { client.request("second", None, Duration::from_secs(1)).await }
        });

        let one = server.recv().await.unwrap();
        let two = server.recv().await.unwrap();
        let (IncomingMessage::Request(one), IncomingMessage::Request(two)) = (one, two) else {
            panic!("expected requests");
        };
        server
            .respond_ok(two.id, json!("second-result"))
            .await
            .unwrap();
        server
            .respond_ok(one.id, json!("first-result"))
            .await
            .unwrap();

        assert_eq!(first.await.unwrap().unwrap(), json!("first-result"));
        assert_eq!(second.await.unwrap().unwrap(), json!("second-result"));
    }

    #[tokio::test]
    async fn publishes_notifications_to_subscribers() {
        let (client, mut server) = FakeLspServer::pair(4096);
        let mut notifications = client.subscribe();
        server
            .notify(
                "textDocument/publishDiagnostics",
                Some(json!({ "uri": "file:///x" })),
            )
            .await
            .unwrap();
        let notification = notifications.recv().await.unwrap();
        assert_eq!(notification.method, "textDocument/publishDiagnostics");
    }

    #[tokio::test]
    async fn timeout_sends_cancel_request() {
        let (client, mut server) = FakeLspServer::pair(4096);
        let request = tokio::spawn(async move {
            client
                .request("slow", None, Duration::from_millis(20))
                .await
        });
        let original = server.recv().await.unwrap();
        let IncomingMessage::Request(original) = original else {
            panic!("expected request");
        };
        assert!(matches!(
            request.await.unwrap(),
            Err(LspError::RequestTimeout { .. })
        ));
        let cancel = server.recv().await.unwrap();
        let IncomingMessage::Notification(cancel) = cancel else {
            panic!("expected cancellation notification");
        };
        assert_eq!(cancel.method, "$/cancelRequest");
        assert_eq!(cancel.params, Some(json!({ "id": original.id })));
    }

    #[tokio::test]
    async fn unsupported_server_request_receives_method_not_found() {
        let (_client, mut server) = FakeLspServer::pair(4096);
        server
            .request(RequestId::Number(99), "workspace/configuration", None)
            .await
            .unwrap();
        let response = server.recv().await.unwrap();
        let IncomingMessage::Response(response) = response else {
            panic!("expected response");
        };
        assert_eq!(response.id, RequestId::Number(99));
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn transport_exit_fails_pending_requests() {
        let (client, mut server) = FakeLspServer::pair(4096);
        let request = tokio::spawn(async move {
            client
                .request("pending", None, Duration::from_secs(5))
                .await
        });
        let _ = server.recv().await.unwrap();
        drop(server);
        assert!(matches!(
            request.await.unwrap(),
            Err(LspError::TransportClosed)
        ));
    }
}
