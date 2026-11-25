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

#[sqlx::test(migrations = "./migrations")]
async fn the_diff_of_a_turn_is_kept_as_codex_last_reported_it(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "diff@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    let (status, turn) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "DIFF write the notes" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");

    let (status, diffs) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/diffs"), &token, None).await;
    assert_eq!(status, StatusCode::OK);
    let diffs = diffs.as_array().expect("a list");
    assert_eq!(diffs.len(), 1, "{diffs:?}");
    assert_eq!(diffs[0]["diff"], "--- a/notes.md\n+++ b/notes.md\n+second\n");
    assert_eq!(diffs[0]["input"], "DIFF write the notes");
    assert_eq!(diffs[0]["seq"], 1);
    assert_eq!(diffs[0]["turn_id"], turn["id"]);

    // A turn with no changes has no diff.
    let (_, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "just talk" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 1).await["status"], "completed");
    let (_, diffs) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/diffs"), &token, None).await;
    assert_eq!(diffs.as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn an_at_mention_searches_the_checkout_the_code_chat_runs_in(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("file-search-{}", uuid::Uuid::new_v4()));
    let config = codex_gateway::code::CodeConfig { root: root.join("data"), ..Default::default() };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let mock = mock_app_server().await;
    let token = login(&state, "files@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    // A plain chat has no checkout to search.
    let (status, found) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files?q=cache"), &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found, json!([]));

    // Make it a code chat with a checkout on disk.
    let repo_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO code_repos (id, workspace_id, full_name) VALUES ($1, $2::uuid, 'acme/demo')")
        .bind(repo_id)
        .bind(&ws_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE threads SET repo_id = $2 WHERE id = $1::uuid").bind(&chat_id).bind(repo_id).execute(&pool).await.unwrap();
    let worktree = root.join("data").join("worktrees").join(&chat_id);
    std::fs::create_dir_all(worktree.join(".git")).unwrap();

    let (status, found) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files?q=cache"), &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found, json!([{ "path": "src/cache.rs", "file_name": "cache.rs" }]));
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().rev().find(|(m, _)| m == "fuzzyFileSearch").expect("the search was sent");
    assert_eq!(params["roots"], json!([worktree.to_string_lossy()]));

    // An empty query searches nothing.
    let (_, found) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files?q=%20"), &token, None).await;
    assert_eq!(found, json!([]));
    let _ = std::fs::remove_dir_all(&root);
}
