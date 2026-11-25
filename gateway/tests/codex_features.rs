//! Codex's own conversation features, used through the gateway: a review of the working copy, a
//! compaction of a long chat, and a note added to a turn that is running.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn a_review_runs_as_a_turn_of_the_chat(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "review@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    // Only known targets are accepted.
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/review"), &token, Some(json!({ "target": { "type": "everything" } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/review"), &token, Some(json!({ "target": { "type": "baseBranch", "branch": "--upload-pack=x" } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, turn) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/review"), &token, Some(json!({ "target": { "type": "uncommittedChanges" } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    assert_eq!(turn["input"], "Review uncommitted changes");
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");

    // app-server was asked for a review of the working copy, delivered inline on the chat's thread.
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().find(|(m, _)| m == "review/start").expect("review/start was sent");
    assert_eq!(params["target"], json!({ "type": "uncommittedChanges" }));
    assert_eq!(params["delivery"], "inline");
    assert_eq!(params["threadId"], "thr_mock");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_chat_is_compacted_and_the_turn_completes_when_app_server_reports_it(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "compact@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    let (status, turn) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/compact"), &token, None).await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let requests = mock.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|(m, p)| m == "thread/compact/start" && p["threadId"] == "thr_mock"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_note_is_added_to_the_running_turn_and_refused_when_none_runs(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "steer@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    // Nothing runs yet: a note is refused.
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/steer"), &token, Some(json!({ "input": "also cover X" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A slow turn is running; the note goes to it.
    call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "SLOW explain it" }))).await;
    for _ in 0..200 {
        let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), &token, None).await;
        if turns[0]["status"] == "running" && turns[0]["codex_turn_id"].is_string() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (status, note) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/steer"), &token, Some(json!({ "input": "also cover X" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{note}");
    assert_eq!(note["status"], "completed");
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().find(|(m, _)| m == "turn/steer").expect("turn/steer was sent");
    assert_eq!(params["input"][0]["text"], "also cover X");
    assert_eq!(params["threadId"], "thr_mock");
    assert_eq!(params["expectedTurnId"], turns_codex_id(&state, &token, &chat_id).await);

    // Once the turn has finished, a note is refused again.
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/steer"), &token, Some(json!({ "input": "late" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

async fn turns_codex_id(state: &AppState, token: &str, chat_id: &str) -> String {
    let (_, turns) = call(state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), token, None).await;
    turns[0]["codex_turn_id"].as_str().unwrap().to_string()
}
