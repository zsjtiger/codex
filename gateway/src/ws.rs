use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use tracing::warn;

/// Placeholder: echoes text frames. It will be replaced by JSON-RPC forwarding
/// to a per-session `codex app-server`, authenticated against the session table.
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    while let Some(message) = socket.recv().await {
        match message {
            Ok(Message::Text(text)) => {
                if socket.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(err) => {
                warn!(%err, "websocket receive failed");
                break;
            }
        }
    }
}
