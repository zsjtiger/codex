//! A chat changed through the gateway reaches every browser watching it, so the other tabs can reload.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The next change the chat reports, skipping any other events.
async fn next_change(ws: &mut Ws) -> String {
    loop {
        let frame = next_of_type(ws, "event").await;
        if frame["method"] == "baize/thread/changed" {
            return frame["params"]["change"].as_str().unwrap().to_string();
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn each_change_to_a_chat_reaches_the_browsers_watching_it(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let token = login(&state, "chat-sync@example.com").await;
    let (_, chat) = setup(&state, &token, Some(&mock.url)).await;
    post_turn(&state, &token, &chat, "start").await;
    assert_eq!(wait_for_turn(&state, &token, &chat, 0).await["status"], "completed");
    let (_, turns) = call(&state, "GET", &format!("/api/v1/threads/{chat}/turns"), &token, None).await;
    let turn = turns[0]["id"].as_str().unwrap().to_string();

    let addr = spawn_gateway(state.clone()).await;
    let mut other_tab = connect(&state, addr, &token).await;
    send(&mut other_tab, json!({ "type": "subscribe", "threadId": chat })).await;
    next_of_type(&mut other_tab, "subscribed").await;

    let path = format!("/api/v1/threads/{chat}");
    let (status, _) = call(&state, "PATCH", &path, &token, Some(json!({ "title": "Renamed" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(next_change(&mut other_tab).await, "title");

    let (status, _) = call(&state, "PATCH", &path, &token, Some(json!({ "archived": true }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(next_change(&mut other_tab).await, "archived");

    let (status, _) = call(&state, "PUT", &format!("/api/v1/turns/{turn}/feedback"), &token, Some(json!({ "rating": "up" }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(next_change(&mut other_tab).await, "feedback");

    let (status, _) = call(&state, "POST", &format!("/api/v1/turns/{turn}/revert"), &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(next_change(&mut other_tab).await, "reverted");

    let (status, _) = call(&state, "DELETE", &path, &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(next_change(&mut other_tab).await, "deleted");
}

/// Next frame of a given type, skipping the rest. Workspace frames carry the workspace, not a chat.
async fn next_workspace_change(ws: &mut Ws) -> Value {
    next_of_type(ws, "workspaceChanged").await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_workspace_chat_list_hears_about_new_renamed_and_removed_chats(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let owner = login(&state, "workspace-sync@example.com").await;
    let (ws_id, chat) = setup(&state, &owner, Some(&mock.url)).await;
    let (_, envs) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/environments"), &owner, None).await;
    let env = envs[0]["id"].as_str().unwrap().to_string();
    let addr = spawn_gateway(state.clone()).await;

    // A sidebar that is not watching any particular chat still hears about changes in the workspace.
    let mut sidebar = connect(&state, addr, &owner).await;
    send(&mut sidebar, json!({ "type": "subscribeWorkspace", "workspaceId": ws_id })).await;
    next_of_type(&mut sidebar, "subscribedWorkspace").await;

    let (status, created) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/threads"), &owner, Some(json!({ "title": "New", "environment_id": env }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let frame = next_workspace_change(&mut sidebar).await;
    assert_eq!(frame["change"], "created");
    assert_eq!(frame["threadId"], created["id"]);
    assert_eq!(frame["workspaceId"], ws_id);

    let (status, _) = call(&state, "PATCH", &format!("/api/v1/threads/{chat}"), &owner, Some(json!({ "title": "Renamed" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(next_workspace_change(&mut sidebar).await["change"], "title");

    let (status, _) = call(&state, "DELETE", &format!("/api/v1/threads/{chat}"), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let frame = next_workspace_change(&mut sidebar).await;
    assert_eq!(frame["change"], "deleted");
    assert_eq!(frame["threadId"], chat);

    // Someone outside the workspace cannot follow its chat list.
    let outsider = login(&state, "outsider-sync@example.com").await;
    let mut stranger = connect(&state, addr, &outsider).await;
    send(&mut stranger, json!({ "type": "subscribeWorkspace", "workspaceId": ws_id })).await;
    assert_eq!(next_of_type(&mut stranger, "error").await["message"], "workspace not found");
}
