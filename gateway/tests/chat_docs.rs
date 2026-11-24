//! Documents saved from a chat: they belong to the chat, list on their own, and can be edited.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use uuid::Uuid;
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

    // Each change keeps what it replaced: the first draft, listed newest first.
    let (status, versions) = call(&state, "GET", &format!("/api/v1/artifacts/{doc_id}/versions"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions.as_array().unwrap().len(), 1);
    assert_eq!(versions[0]["content"], "# Summary\n\nFirst draft.");
    assert_eq!(versions[0]["title"], "Summary");

    // A change that alters nothing keeps no version.
    call(&state, "PATCH", &format!("/api/v1/artifacts/{doc_id}"), &token, Some(json!({ "content": "# Summary\n\nRevised." }))).await;
    let (_, versions) = call(&state, "GET", &format!("/api/v1/artifacts/{doc_id}/versions"), &token, None).await;
    assert_eq!(versions.as_array().unwrap().len(), 2, "one more version for the second real change");
    call(&state, "PATCH", &format!("/api/v1/artifacts/{doc_id}"), &token, Some(json!({ "content": "# Summary\n\nRevised." }))).await;
    let (_, versions) = call(&state, "GET", &format!("/api/v1/artifacts/{doc_id}/versions"), &token, None).await;
    assert_eq!(versions.as_array().unwrap().len(), 2, "an unchanged save keeps no version");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_summary_turn_saves_its_reply_as_a_document_of_the_chat(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "summary@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    // An ordinary turn to summarize.
    call(&state, "POST", &format!("/api/v1/threads/{chat_id}/turns"), &token, Some(json!({ "input": "TOOL explain it" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 0).await["status"], "completed");
    let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), &token, None).await;
    let source = turns[0]["id"].as_str().unwrap().to_string();

    // The summary asks the model; its reply becomes a document of the same chat.
    let (status, summary) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{chat_id}/summaries"),
        &token,
        Some(json!({ "turn_id": source })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{summary}");
    assert_eq!(wait_for_turn(&state, &token, &chat_id, 1).await["status"], "completed");

    let (_, docs) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/artifacts?kind=document&thread_id={chat_id}"), &token, None).await;
    assert_eq!(docs.as_array().unwrap().len(), 1, "{docs}");
    let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/turns"), &token, None).await;
    assert_eq!(docs[0]["content"], turns[1]["reply"], "the document is the summary turn's reply");

    // A reply that is not complete cannot be summarized.
    let (status, _) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{chat_id}/summaries"),
        &token,
        Some(json!({ "turn_id": Uuid::new_v4().to_string() })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
