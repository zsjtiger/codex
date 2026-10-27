//! Tool, search, and file actions Codex completes are stored per thread and listed for the Used panel.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn state_for(pool: &PgPool) -> AppState {
    AppState::new(pool.clone(), true, RuntimeConfig::default())
}

#[sqlx::test(migrations = "./migrations")]
async fn completed_tool_uses_are_stored_and_listed_in_order(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "TOOL find weights" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let (status, uses) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}/tool-uses"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{uses}");
    // The agent message is ordinary conversation and must not appear.
    assert_eq!(
        uses,
        json!([{
            "id": uses[0]["id"],
            "item_id": "ws_1",
            "kind": "Web search",
            "label": "alphafold weights",
            "created_at": uses[0]["created_at"],
        }])
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tool_uses_are_visible_to_workspace_members_only(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let outsider = login(&state, "outsider@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (status, _) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}/tool-uses"),
        &outsider,
        None,
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "non-members must not read tool uses"
    );
}
