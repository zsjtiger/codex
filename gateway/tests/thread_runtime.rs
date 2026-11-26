//! A chat's goal, and the background terminals codex runs for it.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn a_chat_has_a_goal_once_codex_has_started_it(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "goal@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    let base = format!("/api/v1/threads/{chat_id}/goal");

    // A chat codex has not started yet has no goal, and cannot be given one.
    let (status, goal) = call(&state, "GET", &base, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(goal.is_null());
    let (status, _) = call(&state, "PUT", &base, &token, Some(json!({ "objective": "x" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    post_turn(&state, &token, &chat_id, "start").await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let (status, goal) = call(&state, "GET", &base, &token, None).await;
    assert_eq!(status, StatusCode::OK, "{goal}");
    assert_eq!(goal, json!({ "objective": "ship it", "status": "active", "token_budget": null, "tokens_used": 120, "time_used_seconds": 30 }));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_goal_is_set_in_part_or_whole_and_cleared_and_each_change_is_audited(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "goal-set@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    post_turn(&state, &token, &chat_id, "start").await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let base = format!("/api/v1/threads/{chat_id}/goal");

    let (status, goal) = call(&state, "PUT", &base, &token, Some(json!({ "objective": " ship the cache ", "status": "paused", "token_budget": 5000 }))).await;
    assert_eq!(status, StatusCode::OK, "{goal}");
    assert_eq!(goal["objective"], "ship the cache");
    assert_eq!(goal["status"], "paused");
    assert_eq!(goal["token_budget"], 5000);
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().rev().find(|(m, _)| m == "thread/goal/set").expect("codex was asked");
    assert_eq!(params, &json!({ "threadId": "thr_mock", "origin": "user", "objective": "ship the cache", "status": "paused", "tokenBudget": 5000 }));

    // A change of only the status leaves the rest alone; null removes the budget.
    let (status, _) = call(&state, "PUT", &base, &token, Some(json!({ "status": "active", "token_budget": null }))).await;
    assert_eq!(status, StatusCode::OK);
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().rev().find(|(m, _)| m == "thread/goal/set").unwrap();
    assert!(params.get("objective").is_none(), "an unchanged objective is not sent");
    assert_eq!(params["tokenBudget"], serde_json::Value::Null);

    // Bad values are refused before codex is asked.
    for body in [json!({ "objective": "   " }), json!({ "status": "sleeping" }), json!({ "token_budget": -1 }), json!({ "token_budget": "lots" })] {
        let (status, _) = call(&state, "PUT", &base, &token, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} should be refused");
    }

    let (status, _) = call(&state, "DELETE", &base, &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let requests = mock.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|(m, p)| m == "thread/goal/clear" && p["origin"] == "user"));

    let set: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'thread.goal_set' AND target_id = $1")
        .bind(&chat_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let cleared: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'thread.goal_cleared' AND target_id = $1")
        .bind(&chat_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((set, cleared), (2, 1));

    // Someone outside the workspace cannot read or change the goal.
    let outsider = login(&state, "goal-outsider@example.com").await;
    let (status, _) = call(&state, "GET", &base, &outsider, None).await;
    assert_ne!(status, StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn background_terminals_are_listed_stopped_and_cleaned(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "terminals@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    let base = format!("/api/v1/threads/{chat_id}/background-terminals");

    // Before codex has started, there is nothing running.
    let (status, empty) = call(&state, "GET", &base, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty, json!({ "terminals": [], "next_cursor": null }));

    post_turn(&state, &token, &chat_id, "start").await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let (status, listed) = call(&state, "GET", &base, &token, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["terminals"],
        json!([{ "process_id": "p1", "command": "npm run dev", "cwd": "/work", "os_pid": 4242, "cpu_percent": 1.5, "rss_kb": 900 }])
    );

    let (status, _) = call(&state, "POST", &format!("{base}/p1/terminate"), &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&state, "POST", &format!("{base}/clean"), &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let requests = mock.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|(m, p)| m == "thread/backgroundTerminals/terminate" && p["processId"] == "p1"));
    assert!(requests.iter().any(|(m, _)| m == "thread/backgroundTerminals/clean"));

    let stopped: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action IN ('thread.terminal_stop', 'thread.terminals_clean') AND target_id = $1")
        .bind(&chat_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stopped, 2);
}
