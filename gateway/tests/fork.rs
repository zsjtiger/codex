//! Forking a chat from one of its replies, through app-server's thread fork.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn a_chat_forks_from_a_finished_reply_with_its_history_up_to_it(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "fork@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "first question" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "second question" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 1).await["status"], "completed");
    let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), &token, None).await;
    let first = turns[0]["id"].as_str().unwrap().to_string();
    let first_codex = turns[0]["codex_turn_id"].as_str().unwrap().to_string();

    let (status, forked) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/fork"), &token, Some(json!({ "turn_id": first }))).await;
    assert_eq!(status, StatusCode::CREATED, "{forked}");
    assert_eq!(forked["title"], "Untitled (fork)");
    let new_id = forked["id"].as_str().unwrap().to_string();

    // app-server forked through the chosen turn, and the new chat holds only the history up to it.
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().find(|(m, _)| m == "thread/fork").expect("thread/fork was sent");
    assert_eq!(params["lastTurnId"], first_codex);
    assert_eq!(params["threadId"], "thr_mock");
    let (_, copied) = call(&state, "GET", &format!("/api/v1/threads/{new_id}/turns"), &token, None).await;
    assert_eq!(copied.as_array().unwrap().len(), 1, "{copied}");
    assert_eq!(copied[0]["input"], "first question");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_reply_that_is_still_running_cannot_be_forked_from(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "fork-running@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "SLOW think" }))).await;
    let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), &token, None).await;
    let running = turns[0]["id"].as_str().unwrap().to_string();
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/fork"), &token, Some(json!({ "turn_id": running }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
}
