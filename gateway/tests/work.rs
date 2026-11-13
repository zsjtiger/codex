//! A thread's finished commands and file changes, returned grouped by turn for the code view.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn finished_items_come_back_grouped_by_turn_in_order(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "work-owner@example.com").await;
    let (ws_id, thread_id) = setup(&state, &owner, None).await;

    let rows = [
        ("turn-1", "c1", json!({"id": "c1", "type": "commandExecution", "command": "make docs", "status": "completed", "exitCode": 0})),
        ("turn-1", "f1", json!({"id": "f1", "type": "fileChange", "status": "completed", "changes": [{"path": "README.md", "kind": {"type": "update"}, "diff": "+hi\n"}]})),
        ("turn-2", "c2", json!({"id": "c2", "type": "commandExecution", "command": "cargo test", "status": "failed", "exitCode": 1})),
    ];
    for (turn, item_id, item) in rows {
        sqlx::query("INSERT INTO turn_work (thread_id, codex_turn_id, item_id, item) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::parse_str(&thread_id).unwrap())
            .bind(turn)
            .bind(item_id)
            .bind(item)
            .execute(&pool)
            .await
            .unwrap();
    }

    let (status, body) = call(&state, "GET", &format!("/api/v1/threads/{thread_id}/work"), &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[0]["codex_turn_id"], "turn-1");
    assert_eq!(body[0]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body[0]["items"][0]["command"], "make docs");
    assert_eq!(body[0]["items"][1]["changes"][0]["path"], "README.md");
    assert_eq!(body[1]["codex_turn_id"], "turn-2");
    assert_eq!(body[1]["items"][0]["exitCode"], 1);

    // Someone outside the workspace cannot read it.
    let stranger = login(&state, "work-stranger@example.com").await;
    let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{thread_id}/work"), &stranger, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = ws_id;
}
