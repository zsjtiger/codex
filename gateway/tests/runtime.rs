//! Turn dispatch against a mock `codex app-server`.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn turn_runs_to_completion_and_records_usage(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let endpoint = mock.url.clone();
    let seen = mock.seen.clone();
    let token = login(&state, "owner@example.com").await;
    let (ws_id, thread_id) = setup(&state, &token, Some(&endpoint)).await;

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "hello" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(turn["status"], "queued");

    let done = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert!(done["completed_at"].is_string());

    let (_, thread) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(thread["codex_thread_id"], "thr_mock");
    assert_eq!(thread["state"], "idle");

    let (_, report) = call(
        &state,
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
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let endpoint = mock.url.clone();
    let seen = mock.seen.clone();
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&endpoint)).await;
    let turns = format!("/api/v1/threads/{thread_id}/turns");

    call(
        &state,
        "POST",
        &turns,
        &token,
        Some(json!({ "input": "first" })),
    )
    .await;
    wait_for_turn(&state, &token, &thread_id, 0).await;
    call(
        &state,
        "POST",
        &turns,
        &token,
        Some(json!({ "input": "second" })),
    )
    .await;
    let second = wait_for_turn(&state, &token, &thread_id, 1).await;
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
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let endpoint = mock.url.clone();
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&endpoint)).await;

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "FAIL_START please" })),
    )
    .await;
    let turn = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert!(
        turn["error"]
            .as_str()
            .unwrap()
            .contains("model unavailable"),
        "{turn}"
    );

    let (_, thread) = call(
        &state,
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
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let endpoint = mock.url.clone();
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, Some(&endpoint)).await;

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "FAIL_TURN" })),
    )
    .await;
    let turn = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert_eq!(turn["error"], "boom");
}

#[sqlx::test(migrations = "./migrations")]
async fn environment_without_endpoint_fails_the_turn(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, None).await;

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &token,
        Some(json!({ "input": "hello" })),
    )
    .await;
    let turn = wait_for_turn(&state, &token, &thread_id, 0).await;
    assert_eq!(turn["status"], "failed");
    assert!(
        turn["error"].as_str().unwrap().contains("no endpoint"),
        "{turn}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn turn_creation_is_rate_limited_per_user(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_turn_limit(2);
    let token = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &token, None).await;
    let turns = format!("/api/v1/threads/{thread_id}/turns");

    assert_eq!(
        call(
            &state,
            "POST",
            &turns,
            &token,
            Some(json!({ "input": "one" }))
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &state,
            "POST",
            &turns,
            &token,
            Some(json!({ "input": "two" }))
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (status, body) = call(
        &state,
        "POST",
        &turns,
        &token,
        Some(json!({ "input": "three" })),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"]["code"], "rate_limited");
}
