//! A turn can apply skills chosen with "/name". Their instructions reach codex; the stored message does not.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn chosen_skill_instructions_reach_codex_but_not_the_stored_message(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = codex_gateway::AppState::new(
        pool.clone(),
        true,
        codex_gateway::runtime::RuntimeConfig::default(),
    );
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (status, skill) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/skills"),
        &owner,
        Some(json!({ "name": "diagram", "instructions": "Draw diagrams as Mermaid blocks." })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{skill}");
    let skill_id = skill["id"].as_str().unwrap().to_string();

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "/diagram the login flow", "skill_ids": [skill_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    assert_eq!(turn["input"], "/diagram the login flow");
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().find(|(m, _)| m == "turn/start").unwrap();
    let text = params["input"][0]["text"].as_str().unwrap();
    assert!(text.contains("## Skill: diagram"), "{text}");
    assert!(text.contains("Draw diagrams as Mermaid blocks."), "{text}");
    assert!(text.ends_with("the login flow"), "{text}");
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_skills_are_rejected(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = codex_gateway::AppState::new(
        pool.clone(),
        true,
        codex_gateway::runtime::RuntimeConfig::default(),
    );
    let owner = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (status, body) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello", "skill_ids": [Uuid::new_v4()] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
