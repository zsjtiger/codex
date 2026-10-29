//! Usage reports: a member sees only their own tokens, and admins see the workspace total.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, routes::usage::record_usage};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn users_see_their_own_usage_and_admins_see_the_workspace(pool: PgPool) {
    let state = AppState::new(
        pool.clone(),
        true,
        codex_gateway::runtime::RuntimeConfig::default(),
    );
    let owner = login(&state, "owner@example.com").await;
    let member = login(&state, "member@example.com").await;
    let (_, me) = call(&state, "GET", "/api/v1/me", &member, None).await;
    let member_id = Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();
    let (_, me) = call(&state, "GET", "/api/v1/me", &owner, None).await;
    let owner_id = Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let (_, ws) = call(
        &state,
        "POST",
        "/api/v1/workspaces",
        &owner,
        Some(json!({ "slug": "usage", "name": "Usage" })),
    )
    .await;
    let ws_id = Uuid::parse_str(ws["id"].as_str().unwrap()).unwrap();
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/members"),
        &owner,
        Some(json!({ "email": "member@example.com" })),
    )
    .await;

    record_usage(&pool, ws_id, Some(owner_id), None, None, 100, 40)
        .await
        .unwrap();
    record_usage(&pool, ws_id, Some(member_id), None, None, 7, 3)
        .await
        .unwrap();

    let (status, mine) = call(&state, "GET", "/api/v1/me/usage", &member, None).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(
        mine["totals"],
        json!({ "input_tokens": 7, "output_tokens": 3 })
    );

    let (status, total) = call(
        &state,
        "GET",
        &format!("/api/v1/workspaces/{ws_id}/usage"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{total}");
    assert_eq!(
        total["totals"],
        json!({ "input_tokens": 107, "output_tokens": 43 })
    );

    let (status, _) = call(
        &state,
        "GET",
        &format!("/api/v1/workspaces/{ws_id}/usage"),
        &member,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "members cannot read the workspace report"
    );

    let (status, _) = call(
        &state,
        "GET",
        "/api/v1/me/usage?from=2030-01-02T00:00:00Z&to=2030-01-01T00:00:00Z",
        &member,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
