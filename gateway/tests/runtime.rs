//! Turn dispatch against a mock `codex app-server` that speaks the same JSON-RPC shapes.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
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
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tower::ServiceExt;

use codex_gateway::{AppState, app};

/// Starts a mock app-server and returns its URL plus the methods it has received.
async fn mock_app_server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_clone = Arc::clone(&seen);
    // Real codex turn ids are unique (UUIDv7), so the mock issues a fresh id per turn.
    let turn_counter = Arc::new(AtomicU64::new(1));

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let seen = Arc::clone(&seen_clone);
            let turn_counter = Arc::clone(&turn_counter);
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let request: Value = serde_json::from_str(text.as_str()).unwrap();
                    let method = request["method"].as_str().unwrap_or_default().to_string();
                    seen.lock().unwrap().push(method.clone());
                    let Some(id) = request.get("id").cloned() else {
                        continue; // notification, e.g. "initialized"
                    };
                    let reply = match method.as_str() {
                        "initialize" => json!({ "id": id, "result": { "userAgent": "mock" } }),
                        "thread/start" => {
                            json!({ "id": id, "result": { "thread": { "id": "thr_mock" } } })
                        }
                        "turn/start" => {
                            let input = request["params"]["input"][0]["text"]
                                .as_str()
                                .unwrap_or_default();
                            if input.contains("FAIL_START") {
                                json!({ "id": id, "error": { "code": -32000, "message": "model unavailable" } })
                            } else {
                                // Accept the turn, then report its progress and outcome.
                                let turn_id = format!(
                                    "turn_{}",
                                    turn_counter.fetch_add(1, Ordering::Relaxed)
                                );
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
                                let events = [
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
                                    }),
                                    json!({
                                        "method": "turn/completed",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turn": { "id": turn_id, "status": status, "items": [], "error": error }
                                        }
                                    }),
                                ];
                                socket.send(Message::Text(json!({ "id": id, "result": { "turn": { "id": turn_id } } }).to_string().into())).await.unwrap();
                                for event in events {
                                    socket
                                        .send(Message::Text(event.to_string().into()))
                                        .await
                                        .unwrap();
                                }
                                continue;
                            }
                        }
                        _ => json!({ "id": id, "result": {} }),
                    };
                    socket
                        .send(Message::Text(reply.to_string().into()))
                        .await
                        .unwrap();
                }
            });
        }
    });
    (url, seen)
}

async fn call(
    pool: &PgPool,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = app(AppState::new(pool.clone(), true));
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

async fn login(pool: &PgPool, email: &str) -> String {
    let (_, body) = call(
        pool,
        "POST",
        "/api/v1/auth/sessions",
        "",
        Some(json!({ "email": email })),
    )
    .await;
    body["token"].as_str().unwrap().to_string()
}

/// Sets up a workspace, an environment with the given endpoint, and a thread bound to it.
async fn setup(pool: &PgPool, token: &str, endpoint: Option<&str>) -> (String, String) {
    let (_, ws) = call(
        pool,
        "POST",
        "/api/v1/workspaces",
        token,
        Some(json!({ "slug": "runtime", "name": "Runtime" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (_, env) = call(
        pool,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/environments"),
        token,
        Some(json!({ "name": "env", "endpoint": endpoint })),
    )
    .await;
    let (_, thread) = call(
        pool,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        token,
        Some(json!({ "environment_id": env["id"] })),
    )
    .await;
    (ws_id, thread["id"].as_str().unwrap().to_string())
}

/// Polls the turn list until the first turn leaves `queued` / `running`.
async fn wait_for_turn(pool: &PgPool, token: &str, thread_id: &str, seq_index: usize) -> Value {
    for _ in 0..200 {
        let (_, turns) = call(
            pool,
            "GET",
            &format!("/api/v1/threads/{thread_id}/turns"),
            token,
            None,
        )
        .await;
        if let Some(turn) = turns.as_array().and_then(|t| t.get(seq_index))
            && !matches!(turn["status"].as_str(), Some("queued" | "running"))
        {
            return turn.clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("turn did not finish in time");
}

#[sqlx::test(migrations = "./migrations")]
async fn turn_runs_to_completion_and_records_usage(pool: PgPool) {
    let (endpoint, seen) = mock_app_server().await;
    let token = login(&pool, "owner@example.com").await;
    let (ws_id, thread_id) = setup(&pool, &token, Some(&endpoint)).await;

    let (status, turn) = call(
        &pool,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "hello" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(turn["status"], "queued");

    let done = wait_for_turn(&pool, &token, &thread_id, 0).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert!(done["completed_at"].is_string());

    let (_, thread) = call(
        &pool,
        "GET",
        &format!("/api/v1/threads/{thread_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(thread["codex_thread_id"], "thr_mock");
    assert_eq!(thread["state"], "idle");

    let (_, report) = call(
        &pool,
        "GET",
        &format!("/api/v1/workspaces/{ws_id}/usage"),
        &token,
        None,
    )
    .await;
    assert_eq!(report["totals"]["input_tokens"], 10);
    assert_eq!(report["totals"]["output_tokens"], 5);

    let methods = seen.lock().unwrap().clone();
    assert_eq!(
        methods,
        vec!["initialize", "initialized", "thread/start", "turn/start"]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn second_turn_reuses_the_codex_thread(pool: PgPool) {
    let (endpoint, seen) = mock_app_server().await;
    let token = login(&pool, "owner@example.com").await;
    let (_, thread_id) = setup(&pool, &token, Some(&endpoint)).await;
    let turns = format!("/api/v1/threads/{thread_id}/turns");

    call(
        &pool,
        "POST",
        &turns,
        &token,
        Some(json!({ "input": "first" })),
    )
    .await;
    wait_for_turn(&pool, &token, &thread_id, 0).await;
    call(
        &pool,
        "POST",
        &turns,
        &token,
        Some(json!({ "input": "second" })),
    )
    .await;
    let second = wait_for_turn(&pool, &token, &thread_id, 1).await;
    assert_eq!(second["status"], "completed", "{second}");

    let starts = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|m| *m == "thread/start")
        .count();
    assert_eq!(starts, 1, "codex thread should be started once per thread");
}

#[sqlx::test(migrations = "./migrations")]
async fn start_error_fails_the_turn(pool: PgPool) {
    let (endpoint, _) = mock_app_server().await;
    let token = login(&pool, "owner@example.com").await;
    let (_, thread_id) = setup(&pool, &token, Some(&endpoint)).await;

    call(
        &pool,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "FAIL_START please" })),
    )
    .await;
    let turn = wait_for_turn(&pool, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert!(
        turn["error"]
            .as_str()
            .unwrap()
            .contains("model unavailable"),
        "{turn}"
    );

    let (_, thread) = call(
        &pool,
        "GET",
        &format!("/api/v1/threads/{thread_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(thread["state"], "failed");
}

#[sqlx::test(migrations = "./migrations")]
async fn failed_turn_notification_is_recorded(pool: PgPool) {
    let (endpoint, _) = mock_app_server().await;
    let token = login(&pool, "owner@example.com").await;
    let (_, thread_id) = setup(&pool, &token, Some(&endpoint)).await;

    call(
        &pool,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "FAIL_TURN" })),
    )
    .await;
    let turn = wait_for_turn(&pool, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert_eq!(turn["error"], "boom");
}

#[sqlx::test(migrations = "./migrations")]
async fn environment_without_endpoint_fails_the_turn(pool: PgPool) {
    let token = login(&pool, "owner@example.com").await;
    let (_, thread_id) = setup(&pool, &token, None).await;

    call(
        &pool,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "hello" })),
    )
    .await;
    let turn = wait_for_turn(&pool, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert!(turn["error"].as_str().unwrap().contains("no environment"));
}
