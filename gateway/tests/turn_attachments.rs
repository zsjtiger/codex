//! Files sent with a message: binary files are shown but not sent to the model; text files are inlined.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn binary_attachments_are_listed_on_the_turn_but_not_sent_to_the_model(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let pdf = b"%PDF-1.4 secret bytes".to_vec();
    let (status, created) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/attachments"),
        &owner,
        Some(json!({ "filename": "paper.pdf", "content_type": "application/pdf", "size_bytes": pdf.len() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let file_id = created["id"].as_str().unwrap().to_string();
    let (status, _, _) = call_raw(
        &state,
        "PUT",
        &format!("/api/v1/attachments/{file_id}/content"),
        &owner,
        "application/pdf",
        pdf,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "Summarise it", "attachment_ids": [file_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let (_, turns) = call(
        &state,
        "GET",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        turns[0]["attachments"][0]["filename"], "paper.pdf",
        "{turns}"
    );
    assert_eq!(
        turns[0]["attachments"][0]["content_type"],
        "application/pdf"
    );
    assert!(
        turns[0].get("attachment_ids").is_none(),
        "ids are not returned"
    );

    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().find(|(m, _)| m == "turn/start").unwrap();
    let text = params["input"][0]["text"].as_str().unwrap();
    assert!(text.contains("Attached file: paper.pdf"), "{text}");
    assert!(text.contains("not included in this message"), "{text}");
    assert!(
        !text.contains("secret bytes"),
        "file bytes must not reach the model"
    );
}
