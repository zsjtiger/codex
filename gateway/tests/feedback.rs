//! Users rate replies with thumbs up or down; the rating is stored on the turn.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn ratings_are_stored_on_the_turn_and_only_members_can_rate(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let outsider = login(&state, "outsider@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    let turn_id = turn["id"].as_str().unwrap().to_string();
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let rate = format!("/api/v1/turns/{turn_id}/feedback");
    let (status, _) = call(
        &state,
        "PUT",
        &rate,
        &owner,
        Some(json!({ "rating": "up" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, turns) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        None,
    )
    .await;
    assert_eq!(turns[0]["feedback"], "up");

    let (status, _) = call(
        &state,
        "PUT",
        &rate,
        &owner,
        Some(json!({ "rating": null })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, turns) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        None,
    )
    .await;
    assert!(turns[0]["feedback"].is_null());

    let (status, _) = call(
        &state,
        "PUT",
        &rate,
        &owner,
        Some(json!({ "rating": "meh" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &state,
        "PUT",
        &rate,
        &outsider,
        Some(json!({ "rating": "down" })),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::NO_CONTENT,
        "non-members cannot rate replies"
    );
}
