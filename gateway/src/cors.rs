//! Lets the desktop app (and other front ends on other origins) call the gateway.
//!
//! Origins are listed in `GATEWAY_ALLOWED_ORIGINS` as a comma-separated list, for example
//! `tauri://localhost,http://tauri.localhost`. The web app is served from the gateway's own
//! origin, so it needs no entry. Requests use bearer tokens rather than cookies, so no
//! credentials are allowed.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};

/// Origins allowed from `GATEWAY_ALLOWED_ORIGINS`, empty when unset.
pub fn allowed_origins_from_env() -> Arc<Vec<String>> {
    let list = std::env::var("GATEWAY_ALLOWED_ORIGINS").unwrap_or_default();
    Arc::new(
        list.split(',')
            .map(|origin| origin.trim().trim_end_matches('/').to_string())
            .filter(|origin| !origin.is_empty())
            .collect(),
    )
}

/// The request's origin when it is on the allow list.
fn allowed(origin: Option<&HeaderValue>, allowed: &[String]) -> Option<HeaderValue> {
    let origin = origin?.to_str().ok()?;
    allowed.iter().any(|item| item == origin).then(|| HeaderValue::from_str(origin).ok()).flatten()
}

/// Answers preflight requests and adds the allow headers to responses from allowed origins.
pub async fn cors(State(origins): State<Arc<Vec<String>>>, request: Request, next: Next) -> Response {
    let origin = allowed(request.headers().get(header::ORIGIN), &origins);
    if request.method() == Method::OPTIONS {
        if let Some(origin) = origin {
            return preflight(origin);
        }
    }
    let mut response = next.run(request).await;
    if let Some(origin) = origin {
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(header::VARY, HeaderValue::from_static("origin"));
    }
    response
}

fn preflight(origin: HeaderValue) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    headers.insert(header::VARY, HeaderValue::from_static("origin"));
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    *response.headers_mut() = headers;
    response
}

/// Whether a WebSocket upgrade may come from this origin. Browsers send `Origin` on WebSocket
/// requests. Same-host origins are allowed, as are origins on the allow list. A missing
/// `Origin` (non-browser clients) is also allowed, since the connection still needs a ticket.
pub fn websocket_origin_allowed(headers: &HeaderMap, origins: &[String]) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if origins.iter().any(|item| item == origin) {
        return true;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    origin.split_once("://").is_some_and(|(_, authority)| authority == host)
}
