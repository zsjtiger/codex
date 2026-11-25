//! Plugins: a bundle of skills and connectors that installs into a workspace in one step and comes
//! out again with it.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn manifest() -> serde_json::Value {
    json!({
        "skills": [{ "name": "论文写作", "description": "写章节", "instructions": "使用学术语气。" }],
        "connectors": [{ "name": "alphaXiv", "url": "https://api.alphaxiv.org/mcp" }]
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn a_plugin_installs_its_skills_and_connectors_and_uninstalls_them(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "plugins@example.com").await;
    let (ws_id, _) = setup(&state, &token, None).await;

    let (status, plugin) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/plugins"), &token, Some(json!({ "name": "学术套件", "description": "论文相关", "manifest": manifest() }))).await;
    assert_eq!(status, StatusCode::CREATED, "{plugin}");
    assert_eq!(plugin["installed"], false);
    let plugin_id = plugin["id"].as_str().unwrap().to_string();

    // Installing adds the skill and the connector to the workspace.
    let (status, installed) = call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/install"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{installed}");
    assert_eq!(installed["installed"], true);
    let (_, skills) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/skills?kind=skill"), &token, None).await;
    assert!(skills.as_array().unwrap().iter().any(|s| s["name"] == "论文写作" && s["instructions"] == "使用学术语气。"), "{skills}");
    let (_, connectors) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/connectors"), &token, None).await;
    assert!(connectors.as_array().unwrap().iter().any(|c| c["key"] == "alphaxiv" && c["enabled"] == true), "{connectors}");

    // Installing again is refused, so nothing is added twice.
    let (status, _) = call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/install"), &token, None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Uninstalling takes out exactly what the plugin added.
    let (status, removed) = call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/uninstall"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    assert_eq!(removed["installed"], false);
    let (_, skills) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/skills?kind=skill"), &token, None).await;
    assert!(skills.as_array().unwrap().is_empty(), "{skills}");
    let (_, connectors) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/connectors"), &token, None).await;
    assert!(connectors.as_array().unwrap().is_empty(), "{connectors}");

    // Deleting the plugin removes it from the list.
    let (status, _) = call(&state, "DELETE", &format!("/api/v1/plugins/{plugin_id}"), &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, plugins) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/plugins"), &token, None).await;
    assert!(plugins.as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn an_install_that_clashes_with_an_existing_skill_adds_nothing(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "clash@example.com").await;
    let (ws_id, _) = setup(&state, &token, None).await;
    call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/skills"), &token, Some(json!({ "kind": "skill", "name": "论文写作", "instructions": "已有的技能。" }))).await;

    let (_, plugin) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/plugins"), &token, Some(json!({ "name": "学术套件", "manifest": manifest() }))).await;
    let plugin_id = plugin["id"].as_str().unwrap().to_string();
    let (status, body) = call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/install"), &token, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("论文写作"), "{body}");

    // The connector that came before the clashing skill was not added either.
    let (_, connectors) = call(&state, "GET", &format!("/api/v1/workspaces/{ws_id}/connectors"), &token, None).await;
    assert!(connectors.as_array().unwrap().is_empty(), "{connectors}");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_plugin_with_a_bad_address_or_no_contents_is_refused(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "badplugin@example.com").await;
    let (ws_id, _) = setup(&state, &token, None).await;
    let path = format!("/api/v1/workspaces/{ws_id}/plugins");

    let (status, _) = call(&state, "POST", &path, &token, Some(json!({ "name": "坏地址", "manifest": { "connectors": [{ "name": "x", "url": "ftp://nope" }] } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&state, "POST", &path, &token, Some(json!({ "name": "空", "manifest": {} }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn only_admins_install_plugins(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner-plugins@example.com").await;
    let (ws_id, _) = setup(&state, &owner, None).await;
    let (_, plugin) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/plugins"), &owner, Some(json!({ "name": "学术套件", "manifest": manifest() }))).await;
    let plugin_id = plugin["id"].as_str().unwrap().to_string();

    let outsider = login(&state, "outsider-plugins@example.com").await;
    let (status, _) = call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/install"), &outsider, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "outsiders do not learn the plugin exists");
}
