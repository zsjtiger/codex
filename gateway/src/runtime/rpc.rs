//! JSON-RPC 2.0 client over a WebSocket connection to `codex app-server`.
//!
//! Requests are matched to responses by id. Server notifications and
//! server-initiated requests (approvals and similar) are broadcast to subscribers,
//! who answer with `respond` or `respond_error`.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};

use futures_util::{SinkExt, StreamExt, stream::SplitStream};
use serde_json::{Value, json};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const EVENT_BUFFER: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: Option<i64>,
    pub message: String,
}

impl RpcError {
    pub(crate) fn transport(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "{} (code {code})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for RpcError {}

/// A server notification (no id).
#[derive(Debug, Clone)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

/// A server-initiated request that expects a response. `id` is echoed back as-is.
#[derive(Debug, Clone)]
pub struct ServerRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, RpcError>>>>>;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct RpcConnection {
    outgoing: mpsc::Sender<String>,
    pending: Pending,
    next_id: AtomicI64,
    closed: Arc<AtomicBool>,
    notifications: broadcast::Sender<Notification>,
    server_requests: broadcast::Sender<ServerRequest>,
}

impl RpcConnection {
    /// Connects and performs the `initialize` handshake.
    pub async fn connect(url: &str) -> Result<Arc<Self>, RpcError> {
        let (socket, _) = connect_async(url)
            .await
            .map_err(|err| RpcError::transport(format!("connect to {url} failed: {err}")))?;
        let (mut sink, stream) = socket.split();

        let (outgoing, mut outgoing_rx) = mpsc::channel::<String>(256);
        tokio::spawn(async move {
            while let Some(frame) = outgoing_rx.recv().await {
                if sink.send(Message::Text(frame.into())).await.is_err() {
                    break;
                }
            }
        });

        let (notifications, _) = broadcast::channel(EVENT_BUFFER);
        let (server_requests, _) = broadcast::channel(EVENT_BUFFER);
        let connection = Arc::new(Self {
            outgoing,
            pending: Arc::default(),
            next_id: AtomicI64::new(1),
            closed: Arc::new(AtomicBool::new(false)),
            notifications: notifications.clone(),
            server_requests: server_requests.clone(),
        });
        tokio::spawn(read_loop(
            stream,
            Arc::clone(&connection.pending),
            notifications,
            server_requests,
            Arc::clone(&connection.closed),
        ));

        connection
            .request(
                "initialize",
                json!({ "clientInfo": { "name": "codex-gateway", "version": env!("CARGO_PKG_VERSION") } }),
            )
            .await?;
        connection
            .send_frame(json!({ "method": "initialized" }))
            .await?;
        Ok(connection)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.outgoing.is_closed()
    }

    /// Subscribe before sending the request that triggers the events, so none are missed.
    pub fn subscribe(&self) -> broadcast::Receiver<Notification> {
        self.notifications.subscribe()
    }

    pub fn subscribe_server_requests(&self) -> broadcast::Receiver<ServerRequest> {
        self.server_requests.subscribe()
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(id, tx);

        let frame = json!({ "id": id, "method": method, "params": params });
        if let Err(err) = self.send_frame(frame).await {
            self.pending
                .lock()
                .expect("pending lock poisoned")
                .remove(&id);
            return Err(err);
        }
        rx.await
            .map_err(|_| RpcError::transport("connection closed before a response arrived"))?
    }

    /// Answers a server-initiated request with a result.
    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.send_frame(json!({ "id": id, "result": result })).await
    }

    /// Answers a server-initiated request with an error.
    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), RpcError> {
        self.send_frame(json!({ "id": id, "error": { "code": code, "message": message } }))
            .await
    }

    async fn send_frame(&self, frame: Value) -> Result<(), RpcError> {
        self.outgoing
            .send(frame.to_string())
            .await
            .map_err(|_| RpcError::transport("connection closed"))
    }
}

async fn read_loop(
    mut stream: SplitStream<Socket>,
    pending: Pending,
    notifications: broadcast::Sender<Notification>,
    server_requests: broadcast::Sender<ServerRequest>,
    closed: Arc<AtomicBool>,
) {
    while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };

        match (value.get("method").and_then(Value::as_str), value.get("id")) {
            (Some(method), Some(id)) => {
                let _ = server_requests.send(ServerRequest {
                    id: id.clone(),
                    method: method.to_string(),
                    params: value.get("params").cloned().unwrap_or(Value::Null),
                });
            }
            (Some(method), None) => {
                let _ = notifications.send(Notification {
                    method: method.to_string(),
                    params: value.get("params").cloned().unwrap_or(Value::Null),
                });
            }
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else {
                    continue;
                };
                let waiter = pending.lock().expect("pending lock poisoned").remove(&id);
                if let Some(waiter) = waiter {
                    let _ = waiter.send(parse_response(&value));
                }
            }
            (None, None) => {}
        }
    }
    // The socket is gone: mark the connection closed and fail every waiting request.
    closed.store(true, Ordering::Release);
    pending.lock().expect("pending lock poisoned").clear();
}

fn parse_response(value: &Value) -> Result<Value, RpcError> {
    match value.get("error") {
        Some(error) => Err(RpcError {
            code: error.get("code").and_then(Value::as_i64),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
        }),
        None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
    }
}
