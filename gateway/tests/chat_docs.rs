//! Documents saved from a chat: they belong to the chat, list on their own, and can be edited.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn a_chat_document_lists_with_its_chat_and_can_be_edited(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "docs@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, None).await;
    let (_, other_chat) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/threads"), &token, Some(json!({ "title": "Other" }))).await;
    let other_id = other_chat["id"].as_str().unwrap().to_string();

    let (status, doc) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/artifacts"),
        &token,
        Some(json!({ "title": "Summary", "kind": "document", "content": "# Summary\n\nFirst draft.", "thread_id": chat_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{doc}");
    let doc_id = doc["id"].as_str().unwrap().to_string();

    // The chat lists its own document; the other chat does not.
    let (_, mine) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/artifacts?kind=document&thread_id={chat_id}"), &token, None).await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], doc_id.as_str());
    let (_, theirs) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/artifacts?thread_id={other_id}"), &token, None).await;
    assert!(theirs.as_array().unwrap().is_empty());

    // An edit changes the content and keeps the chat it belongs to.
    let (status, edited) = call(
        &state,
        "PATCH",
        &format!("/api/v1/artifacts/{doc_id}"),
        &token,
        Some(json!({ "title": "Summary (final)", "content": "# Summary\n\nFinal text." })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["title"], "Summary (final)");
    assert_eq!(edited["content"], "# Summary\n\nFinal text.");
    assert_eq!(edited["thread_id"], chat_id.as_str());
}
