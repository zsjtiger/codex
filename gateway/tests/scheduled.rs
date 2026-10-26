//! Scheduled tasks: validation, Run now, and runs that come due.

mod common;

use axum::http::StatusCode;
use codex_gateway::{
    AppState,
    runtime::{RuntimeConfig, schedule},
};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn scheduled_tasks_validate_their_schedule(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (_, ws) = call(
        &state,
        "POST",
        "/api/v1/workspaces",
        &owner,
        Some(json!({ "slug": "sched", "name": "S" })),
    )
    .await;
    let tasks = format!(
        "/api/v1/workspaces/{}/scheduled-tasks",
        ws["id"].as_str().unwrap()
    );

    let base = json!({ "name": "Briefing", "prompt": "Summarise the news.", "frequency": "daily", "time_of_day": "08:00" });
    let (status, task) = call(&state, "POST", &tasks, &owner, Some(base.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert!(task["next_run_at"].is_string());

    let (status, _) = call(
        &state,
        "POST",
        &tasks,
        &owner,
        Some(json!({ "name": "W", "prompt": "p", "frequency": "weekly", "time_of_day": "16:00" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "weekly needs a weekday");
    let (status, _) = call(
        &state,
        "POST",
        &tasks,
        &owner,
        Some(json!({ "name": "W", "prompt": "p", "frequency": "monthly", "time_of_day": "16:00" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &state,
        "POST",
        &tasks,
        &owner,
        Some(json!({ "name": "W", "prompt": "p", "frequency": "daily", "time_of_day": "25:00" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn run_now_starts_a_thread_and_keeps_the_schedule(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, _) = setup(&state, &owner, Some(&mock.url)).await;
    let (_, task) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/scheduled-tasks"),
        &owner,
        Some(json!({ "name": "Briefing", "prompt": "Say hi.", "frequency": "weekdays", "time_of_day": "08:00" })),
    )
    .await;
    let task_id = task["id"].as_str().unwrap().to_string();
    let next_before = task["next_run_at"].clone();

    let (status, run) = call(
        &state,
        "POST",
        &format!("/api/v1/scheduled-tasks/{task_id}/run"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let thread_id = run["thread_id"].as_str().unwrap().to_string();

    let turn = wait_for_turn(&state, &owner, &thread_id, 0).await;
    assert_eq!(turn["input"], "Say hi.");
    assert_eq!(turn["status"], "completed", "{turn}");

    let (_, after) = call(
        &state,
        "GET",
        &format!("/api/v1/scheduled-tasks/{task_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        after["next_run_at"], next_before,
        "run now does not move the schedule"
    );
    assert_eq!(after["last_thread_id"], thread_id);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_due_task_starts_a_run_and_moves_on(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, _) = setup(&state, &owner, Some(&mock.url)).await;
    let (_, task) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/scheduled-tasks"),
        &owner,
        Some(json!({ "name": "Hourly check", "prompt": "Say hi.", "frequency": "hourly", "time_of_day": "00:30" })),
    )
    .await;
    let task_id = task["id"].as_str().unwrap().to_string();

    // Make it due now, as if the scheduler had been waiting for its time.
    sqlx::query(
        "UPDATE scheduled_tasks SET next_run_at = now() - interval '1 minute' WHERE id = $1::uuid",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(schedule::run_due(&state).await.unwrap(), 1);

    let (_, after) = call(
        &state,
        "GET",
        &format!("/api/v1/scheduled-tasks/{task_id}"),
        &owner,
        None,
    )
    .await;
    let thread_id = after["last_thread_id"].as_str().unwrap().to_string();
    assert!(
        after["next_run_at"].as_str().unwrap() > "2000-01-01",
        "next run is in the future"
    );
    let (_, thread) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(thread["title"], "Hourly check");
    let turn = wait_for_turn(&state, &owner, &thread_id, 0).await;
    assert_eq!(turn["status"], "completed", "{turn}");

    // Nothing else is due until the next run time.
    assert_eq!(schedule::run_due(&state).await.unwrap(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn disabled_tasks_do_not_run(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (_, ws) = call(
        &state,
        "POST",
        "/api/v1/workspaces",
        &owner,
        Some(json!({ "slug": "off", "name": "O" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (_, task) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/scheduled-tasks"),
        &owner,
        Some(json!({ "name": "Paused", "prompt": "p", "frequency": "daily", "time_of_day": "08:00" })),
    )
    .await;
    let task_id = task["id"].as_str().unwrap().to_string();
    let (status, paused) = call(
        &state,
        "PATCH",
        &format!("/api/v1/scheduled-tasks/{task_id}"),
        &owner,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(paused["enabled"], false);
    sqlx::query(
        "UPDATE scheduled_tasks SET next_run_at = now() - interval '1 minute' WHERE id = $1::uuid",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(schedule::run_due(&state).await.unwrap(), 0);
}
