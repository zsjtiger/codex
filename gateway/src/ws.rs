//! Browser WebSocket: authenticated, subscription-based event and approval channel.
//!
//! Get a ticket with `POST /api/v1/ws-tickets`, then connect to `/ws?ticket=<ticket>`.
//! Tickets last 60 seconds and work once. Then send JSON messages:
//!
//! - `{"type":"subscribe","threadId":"..."}`: start receiving the thread's events.
//! - `{"type":"unsubscribe","threadId":"..."}`
//! - `{"type":"approvalResponse","requestId":"...","result":{...}}`: answer an
//!   `approvalRequest`. `result` is the app-server response for that request kind.
//!
//! The server sends `event` frames (app-server notifications for the thread),
//! `approvalRequest` frames, and `subscribed` / `error` acknowledgements.

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    runtime::{Lease, hub::ResolveError},
    state::AppState,
};

#[derive(Deserialize)]
pub struct WsQuery {
    ticket: Option<String>,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ClientMessage {
    Subscribe { thread_id: Uuid },
    Unsubscribe { thread_id: Uuid },
    ApprovalResponse { request_id: Uuid, result: Value },
}

struct Subscription {
    thread_id: Uuid,
    handle: u64,
    /// Keeps the environment's app-server alive while the browser is watching it.
    _lease: Lease,
}

pub async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let ticket = query.ticket.ok_or(ApiError::Unauthorized)?;
    let user = state
        .tickets
        .redeem(&ticket)
        .ok_or(ApiError::Unauthorized)?;
    Ok(ws
        .on_upgrade(move |socket| session(socket, state, user))
        .into_response())
}

async fn session(socket: WebSocket, state: AppState, user: CurrentUser) {
    let (mut sink, mut stream) = socket.split();
    let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<Value>();
    let mut subscriptions: Vec<Subscription> = Vec::new();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else {
                    break;
                };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break; }
                    continue;
                };
                let reply = match serde_json::from_str::<ClientMessage>(text.as_str()) {
                    Ok(message) => handle(&state, &user, &outbound, &mut subscriptions, message).await,
                    Err(err) => error_frame(&format!("invalid message: {err}")),
                };
                if sink.send(Message::Text(reply.to_string().into())).await.is_err() {
                    break;
                }
            }
            frame = outbound_rx.recv() => {
                let Some(frame) = frame else {
                    break;
                };
                if sink.send(Message::Text(frame.to_string().into())).await.is_err() {
                    break;
                }
            }
        }
    }

    for subscription in subscriptions {
        state
            .runtime
            .hub()
            .unsubscribe(subscription.thread_id, subscription.handle);
    }
}

async fn handle(
    state: &AppState,
    user: &CurrentUser,
    outbound: &mpsc::UnboundedSender<Value>,
    subscriptions: &mut Vec<Subscription>,
    message: ClientMessage,
) -> Value {
    match message {
        ClientMessage::Subscribe { thread_id } => {
            match subscribe(state, user, outbound, subscriptions, thread_id).await {
                Ok(()) => json!({ "type": "subscribed", "threadId": thread_id }),
                Err(message) => error_frame(&message),
            }
        }
        ClientMessage::Unsubscribe { thread_id } => {
            subscriptions.retain(|sub| {
                if sub.thread_id == thread_id {
                    state.runtime.hub().unsubscribe(sub.thread_id, sub.handle);
                    false
                } else {
                    true
                }
            });
            json!({ "type": "unsubscribed", "threadId": thread_id })
        }
        ClientMessage::ApprovalResponse { request_id, result } => {
            let subscribed: Vec<Uuid> = subscriptions.iter().map(|sub| sub.thread_id).collect();
            match state
                .runtime
                .hub()
                .resolve(request_id, &subscribed, result)
                .await
            {
                Ok(()) => json!({ "type": "approvalAccepted", "requestId": request_id }),
                Err(ResolveError::Unknown) => {
                    error_frame("unknown or already answered approval request")
                }
                Err(ResolveError::NotSubscribed) => {
                    error_frame("not subscribed to the thread of this approval")
                }
            }
        }
    }
}

async fn subscribe(
    state: &AppState,
    user: &CurrentUser,
    outbound: &mpsc::UnboundedSender<Value>,
    subscriptions: &mut Vec<Subscription>,
    thread_id: Uuid,
) -> Result<(), String> {
    if subscriptions.iter().any(|sub| sub.thread_id == thread_id) {
        return Ok(());
    }
    let row: Option<(Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT th.workspace_id, th.environment_id, e.endpoint
         FROM threads th
         LEFT JOIN environments e ON e.id = th.environment_id
         WHERE th.id = $1",
    )
    .bind(thread_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| "internal error".to_string())?;
    let Some((workspace_id, environment_id, endpoint)) = row else {
        return Err("thread not found".to_string());
    };
    require_role(&state.pool, workspace_id, user.id, Role::Member)
        .await
        .map_err(|_| "thread not found".to_string())?;
    let environment_id = environment_id.ok_or_else(|| "thread has no environment".to_string())?;

    let lease = state
        .runtime
        .lease(environment_id, endpoint.as_deref())
        .await
        .map_err(|err| err.to_string())?;
    let handle = state.runtime.hub().subscribe(thread_id, outbound.clone());
    subscriptions.push(Subscription {
        thread_id,
        handle,
        _lease: lease,
    });
    Ok(())
}

fn error_frame(message: &str) -> Value {
    json!({ "type": "error", "message": message })
}
