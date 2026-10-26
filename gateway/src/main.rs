//! Web gateway for the cloud Codex service.
//!
//! Currently a skeleton: it serves a health check and a WebSocket endpoint that
//! echoes frames back. Authentication and forwarding to `codex app-server`
//! are not implemented yet.

use axum::{
    Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codex_gateway=info,tower_http=info".into()),
        )
        .init();

    let addr = std::env::var("GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let app = router();

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws_handler))
}

async fn healthz() -> &'static str {
    "ok"
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

/// Placeholder: echoes text frames. Replace with JSON-RPC forwarding to a
/// per-session `codex app-server` once the runtime layer exists.
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
