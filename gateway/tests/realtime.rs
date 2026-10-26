//! Browser WebSocket, approvals, turn ordering, restart recovery, and local launching.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::StatusCode;
use codex_gateway::{
    AppState,
    runtime::{LocalLaunch, RuntimeConfig, dispatch},
};
use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect(addr: SocketAddr, token: &str) -> Ws {
    connect_async(format!("ws://{addr}/ws?token={token}"))
        .await
        .expect("websocket connect")
        .0
}

async fn send(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn next_json(ws: &mut Ws) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("timed out waiting for a frame")
        .expect("socket closed")
        .expect("socket error");
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

/// Reads frames until one has the given `type`.
async fn next_of_type(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let frame = next_json(ws).await;
        if frame["type"] == kind {
            return frame;
        }
    }
}

async fn post_turn(state: &AppState, token: &str, thread_id: &str, input: &str) -> StatusCode {
    let (status, body) = call(
        state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        token,
        Some(json!({ "input": input })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    status
}

#[sqlx::test(migrations = "./migrations")]
async fn websocket_requires_a_valid_session(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let addr = spawn_gateway(state.clone()).await;

    assert!(connect_async(format!("ws://{addr}/ws")).await.is_err());
    assert!(
        connect_async(format!("ws://{addr}/ws?token=cdx_bogus"))
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn subscribed_browser_receives_turn_events(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;
    let addr = spawn_gateway(state.clone()).await;

    let mut ws = connect(addr, &token).await;
    send(
        &mut ws,
        json!({ "type": "subscribe", "threadId": thread_id }),
    )
    .await;
    assert_eq!(
        next_of_type(&mut ws, "subscribed").await["threadId"],
        thread_id
    );

    post_turn(&state, &token, &thread_id, "hello").await;
    let mut methods = Vec::new();
    loop {
        let frame = next_of_type(&mut ws, "event").await;
        assert_eq!(
            frame["threadId"], thread_id,
            "events carry the gateway thread id"
        );
        let method = frame["method"].as_str().unwrap().to_string();
        methods.push(method.clone());
        if method == "turn/completed" {
            break;
        }
    }
    assert_eq!(methods, vec!["thread/tokenUsage/updated", "turn/completed"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn approval_is_routed_to_the_browser_and_answered(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;
    let addr = spawn_gateway(state.clone()).await;

    let mut ws = connect(addr, &token).await;
    send(
        &mut ws,
        json!({ "type": "subscribe", "threadId": thread_id }),
    )
    .await;
    next_of_type(&mut ws, "subscribed").await;

    post_turn(&state, &token, &thread_id, "APPROVAL run it").await;
    let request = next_of_type(&mut ws, "approvalRequest").await;
    assert_eq!(request["method"], "item/commandExecution/requestApproval");
    assert_eq!(request["threadId"], thread_id);

    send(
        &mut ws,
        json!({ "type": "approvalResponse", "requestId": request["requestId"], "result": { "decision": "accept" } }),
    )
    .await;
    assert_eq!(
        next_of_type(&mut ws, "approvalAccepted").await["requestId"],
        request["requestId"]
    );

    let turn = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "completed", "{turn}");
    let responses = mock.responses.lock().unwrap().clone();
    assert!(
        responses
            .iter()
            .any(|r| r["id"] == APPROVAL_ID && r["result"]["decision"] == "accept"),
        "{responses:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn approval_is_declined_when_nobody_is_watching(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;

    post_turn(&state, &token, &thread_id, "APPROVAL run it").await;
    let turn = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "completed", "{turn}");
    let responses = mock.responses.lock().unwrap().clone();
    assert!(
        responses
            .iter()
            .any(|r| r["id"] == APPROVAL_ID && r["result"]["decision"] == "decline"),
        "{responses:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn approval_answers_must_come_from_a_subscribed_session(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;
    let addr = spawn_gateway(state.clone()).await;

    let mut watcher = connect(addr, &token).await;
    send(
        &mut watcher,
        json!({ "type": "subscribe", "threadId": thread_id }),
    )
    .await;
    next_of_type(&mut watcher, "subscribed").await;
    let mut other = connect(addr, &token).await;

    post_turn(&state, &token, &thread_id, "APPROVAL run it").await;
    let request = next_of_type(&mut watcher, "approvalRequest").await;

    // A session that is not subscribed to the thread cannot answer for it.
    send(
        &mut other,
        json!({ "type": "approvalResponse", "requestId": request["requestId"], "result": { "decision": "accept" } }),
    )
    .await;
    assert_eq!(
        next_of_type(&mut other, "error").await["message"],
        "not subscribed to the thread of this approval"
    );

    // Unknown request ids are rejected too.
    send(
        &mut watcher,
        json!({ "type": "approvalResponse", "requestId": Uuid::new_v4(), "result": { "decision": "accept" } }),
    )
    .await;
    assert_eq!(
        next_of_type(&mut watcher, "error").await["message"],
        "unknown or already answered approval request"
    );

    send(
        &mut watcher,
        json!({ "type": "approvalResponse", "requestId": request["requestId"], "result": { "decision": "accept" } }),
    )
    .await;
    next_of_type(&mut watcher, "approvalAccepted").await;
    assert_eq!(
        wait_for_turn(&state, &token, &thread_id, 0).await["status"],
        "completed"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn outsiders_cannot_subscribe_to_a_thread(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let outsider = login(&state, "outsider@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    let addr = spawn_gateway(state.clone()).await;

    let mut ws = connect(addr, &outsider).await;
    send(
        &mut ws,
        json!({ "type": "subscribe", "threadId": thread_id }),
    )
    .await;
    assert_eq!(
        next_of_type(&mut ws, "error").await["message"],
        "thread not found"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn turns_on_one_thread_run_one_at_a_time(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;

    post_turn(&state, &token, &thread_id, "SLOW first").await;
    post_turn(&state, &token, &thread_id, "SLOW second").await;
    assert_eq!(
        wait_for_turn(&state, &token, &thread_id, 0).await["status"],
        "completed"
    );
    assert_eq!(
        wait_for_turn(&state, &token, &thread_id, 1).await["status"],
        "completed"
    );
    assert_eq!(
        mock.max_in_flight.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn restart_fails_interrupted_turns_and_resumes_queued_ones(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&mock.url)).await;
    let thread: Uuid = thread_id.parse().unwrap();

    // What a crash leaves behind: one turn mid-flight, one still queued.
    sqlx::query(
        "INSERT INTO turns (thread_id, seq, status, input) VALUES ($1, 1, 'running', 'before'), ($1, 2, 'queued', 'after')",
    )
    .bind(thread)
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(dispatch::recover(&state).await.unwrap(), 1);
    let interrupted = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(interrupted["status"], "failed");
    assert!(interrupted["error"].as_str().unwrap().contains("restarted"));
    let resumed = wait_for_turn(&state, &token, &thread_id, 1).await;
    assert_eq!(resumed["status"], "completed", "{resumed}");
}

/// Needs a real `codex` binary: set `GATEWAY_TEST_CODEX_BIN` and run with `--ignored`.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs GATEWAY_TEST_CODEX_BIN"]
async fn local_launch_starts_and_reaps_a_real_app_server(pool: PgPool) {
    let codex_bin = std::env::var("GATEWAY_TEST_CODEX_BIN").expect("GATEWAY_TEST_CODEX_BIN");
    let home_root = std::env::temp_dir().join(format!("gateway-test-{}", Uuid::new_v4()));
    let config = RuntimeConfig {
        local: Some(LocalLaunch {
            codex_bin: codex_bin.into(),
            home_root: home_root.clone(),
        }),
        idle_timeout: Duration::ZERO,
    };
    let state = AppState::new(pool.clone(), true, config);

    let environment = Uuid::new_v4();
    let lease = state
        .runtime
        .lease(environment, None)
        .await
        .expect("launch");
    let started = lease
        .request("thread/start", json!({}))
        .await
        .expect("thread/start");
    assert!(
        started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .is_some(),
        "{started}"
    );
    assert!(
        home_root
            .join(environment.to_string())
            .join("app-server.log")
            .exists()
    );
    assert_eq!(state.runtime.live_connections().await, 1);

    drop(lease);
    assert_eq!(state.runtime.reap_idle().await, 1);
    assert_eq!(state.runtime.live_connections().await, 0);
    let _ = std::fs::remove_dir_all(&home_root);
}
