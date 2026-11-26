//! Codex's configuration and runtime switches (admins only), and the chat's settings, hooks, and MCP calls (members).

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// An owner (admin) with a workspace and a chat, and a member of that workspace.
async fn owner_member_and_chat(state: &AppState, mock_url: &str, owner_email: &str) -> (String, String, String) {
    let owner = login(state, owner_email).await;
    let member = login(state, "member-config@example.com").await;
    let (ws, chat) = setup(state, &owner, Some(mock_url)).await;
    let (status, _) = call(state, "POST", &format!("/api/v1/workspaces/{ws}/members"), &owner, Some(json!({ "email": "member-config@example.com" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    (owner, member, chat)
}

/// The chat's environment: the workspace's first one, which `setup` made for the chat.
async fn environment_of(state: &AppState, owner: &str, chat: &str) -> String {
    let (_, chat_view) = call(state, "GET", &format!("/api/v1/threads/{chat}/turns"), owner, None).await;
    assert!(chat_view.is_array());
    let (_, ws) = call(state, "GET", "/api/v1/workspaces", owner, None).await;
    let ws_id = ws[0]["id"].as_str().unwrap().to_string();
    let (_, envs) = call(state, "GET", &format!("/api/v1/workspaces/{ws_id}/environments"), owner, None).await;
    envs[0]["id"].as_str().expect("the workspace has an environment").to_string()
}

async fn audit_rows(pool: &PgPool, action: &str, target: &str) -> Vec<Value> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT metadata::text FROM audit_logs WHERE action = $1 AND target_id = $2")
        .bind(action)
        .bind(target)
        .fetch_all(pool)
        .await
        .unwrap();
    rows.iter().map(|row| serde_json::from_str(row).unwrap()).collect()
}

fn asked(mock: &MockServer, method: &str) -> Vec<Value> {
    mock.requests.lock().unwrap().iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn config_changes_are_admin_only_and_the_audit_log_names_keys_not_values(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let (owner, member, chat) = owner_member_and_chat(&state, &mock.url, "owner-config@example.com").await;
    let env = environment_of(&state, &owner, &chat).await;

    let body = json!({ "key_path": "model", "value": "a-very-specific-model-name", "merge_strategy": "replace", "expected_version": "v6" });
    let (status, _) = call(&state, "PUT", &format!("/api/v1/environments/{env}/config/value"), &member, Some(body.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, written) = call(&state, "PUT", &format!("/api/v1/environments/{env}/config/value"), &owner, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{written}");
    assert_eq!(written["version"], "v7");

    let sent = asked(&mock, "config/value/write");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], json!({ "keyPath": "model", "value": "a-very-specific-model-name", "mergeStrategy": "replace", "expectedVersion": "v6" }));

    // The value is not written to the audit log; the key path and strategy are.
    let logs = audit_rows(&pool, "environment.config_write", &env).await;
    assert_eq!(logs, vec![json!({ "key_path": "model", "merge_strategy": "replace" })]);

    // Reading the requirements and the diagnostics is admin-only too.
    for path in ["config/requirements", "diagnostics", "experimental-features"] {
        let (status, _) = call(&state, "GET", &format!("/api/v1/environments/{env}/{path}"), &member, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        let (status, _) = call(&state, "GET", &format!("/api/v1/environments/{env}/{path}"), &owner, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn keys_that_look_like_secrets_are_refused_before_codex_is_asked(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let (owner, _member, chat) = owner_member_and_chat(&state, &mock.url, "owner-secrets@example.com").await;
    let env = environment_of(&state, &owner, &chat).await;

    for key in ["model_providers.custom.api_key", "mcp_servers.x.env.GITHUB_TOKEN", "auth.password", "../escape", "a b", ""] {
        let body = json!({ "key_path": key, "value": "x", "merge_strategy": "replace" });
        let (status, _) = call(&state, "PUT", &format!("/api/v1/environments/{env}/config/value"), &owner, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key:?} should be refused");
    }
    // One bad edit spoils the whole batch: nothing is written.
    let batch = json!({ "edits": [
        { "key_path": "model", "value": "m", "merge_strategy": "replace" },
        { "key_path": "auth.api_key", "value": "k", "merge_strategy": "upsert" }
    ] });
    let (status, _) = call(&state, "POST", &format!("/api/v1/environments/{env}/config/batch"), &owner, Some(batch)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&state, "POST", &format!("/api/v1/environments/{env}/config/batch"), &owner, Some(json!({ "edits": [] }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert!(asked(&mock, "config/value/write").is_empty());
    assert!(asked(&mock, "config/batchWrite").is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn batches_reloads_features_and_skill_switches_reach_codex_and_are_audited(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let (owner, member, chat) = owner_member_and_chat(&state, &mock.url, "owner-batch@example.com").await;
    let env = environment_of(&state, &owner, &chat).await;

    let batch = json!({ "edits": [
        { "key_path": "features.goals", "value": true, "merge_strategy": "upsert" },
        { "key_path": "model", "value": "m", "merge_strategy": "replace" }
    ], "reload_user_config": true });
    let (status, _) = call(&state, "POST", &format!("/api/v1/environments/{env}/config/batch"), &owner, Some(batch)).await;
    assert_eq!(status, StatusCode::OK);
    let sent = asked(&mock, "config/batchWrite");
    assert_eq!(sent[0]["edits"].as_array().unwrap().len(), 2);
    assert_eq!(sent[0]["reloadUserConfig"], true);
    let logs = audit_rows(&pool, "environment.config_batch_write", &env).await;
    assert_eq!(logs[0]["edits"], json!([{ "key_path": "features.goals", "merge_strategy": "upsert" }, { "key_path": "model", "merge_strategy": "replace" }]));

    let (status, _) = call(&state, "POST", &format!("/api/v1/environments/{env}/config/mcp-reload"), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(asked(&mock, "config/mcpServer/reload").len(), 1);

    let (status, features) = call(&state, "PUT", &format!("/api/v1/environments/{env}/experimental-features"), &owner, Some(json!({ "enablement": { "goals": true } }))).await;
    assert_eq!(status, StatusCode::OK, "{features}");
    assert_eq!(asked(&mock, "experimentalFeature/enablement/set")[0], json!({ "enablement": { "goals": true } }));
    assert_eq!(audit_rows(&pool, "environment.features_set", &env).await[0], json!({ "features": { "goals": true } }));
    let (status, _) = call(&state, "PUT", &format!("/api/v1/environments/{env}/experimental-features"), &owner, Some(json!({ "enablement": {} }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, switched) = call(&state, "POST", &format!("/api/v1/environments/{env}/codex-skills/config"), &owner, Some(json!({ "name": "alpha", "enabled": false }))).await;
    assert_eq!(status, StatusCode::OK, "{switched}");
    assert_eq!(switched["effectiveEnabled"], false);
    assert_eq!(asked(&mock, "skills/config/write")[0], json!({ "name": "alpha", "enabled": false }));
    let (status, _) = call(&state, "POST", &format!("/api/v1/environments/{env}/codex-skills/config"), &member, Some(json!({ "name": "alpha", "enabled": true }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "./migrations")]
async fn members_change_only_the_model_and_effort_and_their_calls_are_audited_without_arguments(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let mock = mock_app_server().await;
    let (owner, member, chat) = owner_member_and_chat(&state, &mock.url, "owner-chat@example.com").await;
    // Nothing can be set on a chat codex has not started.
    let (status, _) = call(&state, "PUT", &format!("/api/v1/threads/{chat}/settings"), &member, Some(json!({ "model": "gpt-5" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    post_turn(&state, &owner, &chat, "start").await;
    assert_eq!(wait_for_turn(&state, &owner, &chat, 0).await["status"], "completed");

    let (status, _) = call(&state, "PUT", &format!("/api/v1/threads/{chat}/settings"), &member, Some(json!({ "model": "gpt-5", "effort": "high" }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(asked(&mock, "thread/settings/update")[0], json!({ "threadId": "thr_mock", "model": "gpt-5", "effort": "high" }));
    let (status, _) = call(&state, "PUT", &format!("/api/v1/threads/{chat}/settings"), &member, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Other settings are not accepted at all.
    let (status, _) = call(&state, "PUT", &format!("/api/v1/threads/{chat}/settings"), &member, Some(json!({ "sandbox_policy": { "type": "dangerFullAccess" } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown fields are refused");

    let (status, hooks) = call(&state, "GET", &format!("/api/v1/threads/{chat}/hooks"), &member, None).await;
    assert_eq!(status, StatusCode::OK, "{hooks}");
    assert_eq!(hooks["data"], json!([]));

    let secret_query = "private question about the merger";
    let (status, result) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{chat}/mcp/tool-call"),
        &member,
        Some(json!({ "server": "alphaxiv", "tool": "search_papers", "arguments": { "query": secret_query } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["content"][0]["text"], "3 papers found");
    assert_eq!(asked(&mock, "mcpServer/tool/call")[0]["arguments"]["query"], secret_query);

    let (status, resource) = call(&state, "POST", &format!("/api/v1/threads/{chat}/mcp/resource-read"), &member, Some(json!({ "server": "alphaxiv", "uri": "paper://private/1" }))).await;
    assert_eq!(status, StatusCode::OK, "{resource}");
    assert_eq!(resource["contents"][0]["text"], "resource body");

    let tool_logs = audit_rows(&pool, "thread.mcp_tool_call", &chat).await;
    assert_eq!(tool_logs, vec![json!({ "server": "alphaxiv", "tool": "search_papers" })]);
    let resource_logs = audit_rows(&pool, "thread.mcp_resource_read", &chat).await;
    assert_eq!(resource_logs, vec![json!({ "server": "alphaxiv" })]);
    let settings_logs = audit_rows(&pool, "thread.settings_update", &chat).await;
    assert_eq!(settings_logs[0], json!({ "model": "gpt-5", "effort": "high" }));
    let logs = serde_json::to_string(&(tool_logs, resource_logs, settings_logs)).unwrap();
    assert!(!logs.contains(secret_query) && !logs.contains("paper://private"), "the audit log holds no arguments or addresses");
}
