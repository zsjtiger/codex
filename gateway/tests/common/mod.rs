//! Shared test helpers: a mock codex app-server and HTTP/WebSocket helpers.

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tower::ServiceExt;

use codex_gateway::{AppState, app};

/// Request id the mock uses for its approval requests.
pub const APPROVAL_ID: i64 = 900;

pub struct MockServer {
    pub url: String,
    /// Methods the gateway has sent, in order.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// Every request the gateway has sent, with its params.
    pub requests: Arc<Mutex<Vec<(String, Value)>>>,
    /// Responses the gateway has sent back to the mock's server requests.
    pub responses: Arc<Mutex<Vec<Value>>>,
    /// Largest number of turns in flight at once.
    pub max_in_flight: Arc<AtomicUsize>,
}

/// Starts a mock app-server. Turn behaviour depends on the input text:
/// `FAIL_START` returns an error, `FAIL_TURN` completes as failed,
/// `APPROVAL` sends an approval request first, and `SLOW` delays completion.
pub async fn mock_app_server() -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let responses = Arc::new(Mutex::new(Vec::new()));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let turn_counter = Arc::new(AtomicU64::new(1));

    let requests_c = Arc::clone(&requests);
    let (seen_c, responses_c, max_c) = (
        Arc::clone(&seen),
        Arc::clone(&responses),
        Arc::clone(&max_in_flight),
    );
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let socket = accept_async(stream).await.unwrap();
            let (mut sink, mut stream) = socket.split();
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                while let Some(frame) = out_rx.recv().await {
                    if sink.send(Message::Text(frame.into())).await.is_err() {
                        break;
                    }
                }
            });

            let requests = Arc::clone(&requests_c);
            let (seen, responses, max_in_flight, in_flight, turn_counter) = (
                Arc::clone(&seen_c),
                Arc::clone(&responses_c),
                Arc::clone(&max_c),
                Arc::clone(&in_flight),
                Arc::clone(&turn_counter),
            );
            tokio::spawn(async move {
                while let Some(Ok(Message::Text(text))) = stream.next().await {
                    let msg: Value = serde_json::from_str(text.as_str()).unwrap();
                    let Some(method) = msg
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    else {
                        // A response from the gateway to one of our server requests.
                        responses.lock().unwrap().push(msg);
                        continue;
                    };
                    seen.lock().unwrap().push(method.clone());
                    requests
                        .lock()
                        .unwrap()
                        .push((method.clone(), msg["params"].clone()));
                    let Some(id) = msg.get("id").cloned() else {
                        continue; // notification such as "initialized"
                    };
                    let reply = |result: Value| json!({ "id": id, "result": result }).to_string();
                    match method.as_str() {
                        "initialize" => out_tx.send(reply(json!({ "userAgent": "mock" }))).unwrap(),
                        "thread/start" => out_tx
                            .send(reply(json!({ "thread": { "id": "thr_mock" } })))
                            .unwrap(),
                        "turn/start" => {
                            let input = msg["params"]["input"][0]["text"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            if input.contains("FAIL_START") {
                                out_tx
                                    .send(json!({ "id": id, "error": { "code": -32000, "message": "model unavailable" } }).to_string())
                                    .unwrap();
                                continue;
                            }
                            let turn_id =
                                format!("turn_{}", turn_counter.fetch_add(1, Ordering::Relaxed));
                            let now_in_flight = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                            max_in_flight.fetch_max(now_in_flight, Ordering::SeqCst);
                            out_tx
                                .send(reply(json!({ "turn": { "id": turn_id } })))
                                .unwrap();

                            let out = out_tx.clone();
                            let in_flight = Arc::clone(&in_flight);
                            let responses = Arc::clone(&responses);
                            tokio::spawn(async move {
                                if input.contains("APPROVAL") {
                                    out.send(
                                        json!({
                                            "id": APPROVAL_ID,
                                            "method": "item/commandExecution/requestApproval",
                                            "params": {
                                                "threadId": "thr_mock",
                                                "turnId": turn_id,
                                                "itemId": "item_1",
                                                "startedAtMs": 0
                                            }
                                        })
                                        .to_string(),
                                    )
                                    .unwrap();
                                    wait_for_response(&responses, APPROVAL_ID).await;
                                }
                                if input.contains("SLOW") {
                                    tokio::time::sleep(Duration::from_millis(400)).await;
                                }
                                let status = if input.contains("FAIL_TURN") {
                                    "failed"
                                } else {
                                    "completed"
                                };
                                let error = if status == "failed" {
                                    json!({ "message": "boom" })
                                } else {
                                    Value::Null
                                };
                                out.send(
                                    json!({
                                        "method": "thread/tokenUsage/updated",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turnId": turn_id,
                                            "tokenUsage": {
                                                "last": { "inputTokens": 10, "outputTokens": 5 },
                                                "total": { "inputTokens": 10, "outputTokens": 5 }
                                            }
                                        }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                                in_flight.fetch_sub(1, Ordering::SeqCst);
                                out.send(
                                    json!({
                                        "method": "turn/completed",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turn": { "id": turn_id, "status": status, "items": [], "error": error }
                                        }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                            });
                        }
                        _ => out_tx.send(reply(json!({}))).unwrap(),
                    }
                }
            });
        }
    });

    MockServer {
        url,
        seen,
        requests,
        responses,
        max_in_flight,
    }
}

async fn wait_for_response(responses: &Mutex<Vec<Value>>, id: i64) {
    for _ in 0..500 {
        if responses.lock().unwrap().iter().any(|r| r["id"] == id) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = app(state.clone());
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn login(state: &AppState, email: &str) -> String {
    let (_, body) = call(
        state,
        "POST",
        "/api/v1/auth/sessions",
        "",
        Some(json!({ "email": email })),
    )
    .await;
    body["token"].as_str().unwrap().to_string()
}

/// Creates a workspace, an environment with the given endpoint, and a thread bound to it.
/// Returns `(workspace_id, thread_id)`.
pub async fn setup(state: &AppState, token: &str, endpoint: Option<&str>) -> (String, String) {
    let (_, ws) = call(
        state,
        "POST",
        "/api/v1/workspaces",
        token,
        Some(json!({ "slug": "runtime", "name": "Runtime" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (_, env) = call(
        state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/environments"),
        token,
        Some(json!({ "name": "env", "endpoint": endpoint })),
    )
    .await;
    let (_, thread) = call(
        state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        token,
        Some(json!({ "environment_id": env["id"] })),
    )
    .await;
    (ws_id, thread["id"].as_str().unwrap().to_string())
}

/// Polls the turn list until the turn at `index` leaves `queued` / `running`.
pub async fn wait_for_turn(state: &AppState, token: &str, thread_id: &str, index: usize) -> Value {
    for _ in 0..400 {
        let (_, turns) = call(
            state,
            "GET",
            &format!("/api/v1/threads/{thread_id}/turns"),
            token,
            None,
        )
        .await;
        if let Some(turn) = turns.as_array().and_then(|t| t.get(index))
            && !matches!(turn["status"].as_str(), Some("queued" | "running"))
        {
            return turn.clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("turn did not finish in time");
}

/// Serves the gateway on a real TCP port, for WebSocket tests. Returns its address.
pub async fn spawn_gateway(state: AppState) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    addr
}

/// Sends a request with a raw body and returns the status, content type and bytes.
pub async fn call_raw(
    state: &AppState,
    method: &str,
    path: &str,
    token: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let router = app(state.clone());
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, content_type, bytes)
}
