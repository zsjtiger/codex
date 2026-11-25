//! The codex account's rate limits for an environment, read through codex.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn an_environments_rate_limits_are_read_from_its_codex(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "limits@example.com").await;
    let (ws_id, _) = setup(&state, &token, Some(&mock.url)).await;
    let (_, envs) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/environments"), &token, None).await;
    let env_id = envs[0]["id"].as_str().unwrap().to_string();

    let (status, limits) = call(&state, "GET", &format!("/api/v1/environments/{env_id}/rate-limits"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{limits}");
    assert_eq!(
        limits,
        json!({
            "limit_name": null,
            "plan_type": "plus",
            "primary": { "used_percent": 42, "window_minutes": 300, "resets_at": 1800000000 },
            "secondary": { "used_percent": 7, "window_minutes": 10080, "resets_at": null },
            "credits": { "has_credits": true, "unlimited": false, "balance": "12.50" },
            "reached_type": null
        })
    );
    let requests = mock.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|(m, _)| m == "account/rateLimits/read"));

    // Someone outside the workspace cannot see them.
    let outsider = login(&state, "outside-limits@example.com").await;
    let (status, _) = call(&state, "GET", &format!("/api/v1/environments/{env_id}/rate-limits"), &outsider, None).await;
    assert_ne!(status, StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn an_environment_without_codex_reports_that(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "nocodex@example.com").await;
    let (ws_id, _) = setup(&state, &token, None).await;
    let (_, envs) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/environments"), &token, None).await;
    let env_id = envs[0]["id"].as_str().unwrap().to_string();
    let (status, body) = call(&state, "GET", &format!("/api/v1/environments/{env_id}/rate-limits"), &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
