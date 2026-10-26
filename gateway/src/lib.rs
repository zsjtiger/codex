//! Web gateway for the cloud Codex service.
//!
//! `app` builds the HTTP router: the REST control plane under `/api/v1`,
//! and the browser WebSocket at `/ws`, which is still an echo placeholder.

pub mod audit;
pub mod auth;
pub mod authz;
pub mod config;
pub mod error;
pub mod ratelimit;
pub mod routes;
pub mod runtime;
pub mod state;
pub mod tickets;
pub mod validate;
pub mod ws;

use axum::{Router, routing::get};

pub use state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(routes::router())
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws::ws_handler))
        .with_state(state)
}
