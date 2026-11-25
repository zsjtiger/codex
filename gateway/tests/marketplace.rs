//! The built-in marketplace: its entries, adding one to a workspace, and the "most used" counts.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn entry(list: &serde_json::Value, key: &str) -> serde_json::Value {
    list.as_array().unwrap().iter().find(|e| e["key"] == key).cloned().unwrap_or_else(|| panic!("no entry {key}: {list}"))
}

#[sqlx::test(migrations = "./migrations")]
async fn the_catalog_lists_entries_and_counts_installs_across_workspaces(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let token = login(&state, "market@example.com").await;
    let (ws_id, _) = setup(&state, &token, None).await;

    let (status, list) = call(&state, "GET", "/api/v1/marketplace", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(list.as_array().unwrap().len() >= 2);
    assert_eq!(entry(&list, "academic-suite")["installs"], 0);

    // Adding an entry makes a plugin in the workspace, which can then be installed.
    let add = format!("/api/v1/workspaces/{ws_id}/plugins/from-catalog");
    let (status, plugin) = call(&state, "POST", &add, &token, Some(json!({ "key": "academic-suite" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{plugin}");
    assert_eq!(plugin["catalog_key"], "academic-suite");
    assert_eq!(plugin["installed"], false);
    let (status, _) = call(&state, "POST", &add, &token, Some(json!({ "key": "academic-suite" }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "the same entry is not added twice");

    // Installed plugins count as used; plugins that are only added do not.
    let plugin_id = plugin["id"].as_str().unwrap();
    call(&state, "POST", &format!("/api/v1/plugins/{plugin_id}/install"), &token, None).await;
    let (_, list) = call(&state, "GET", "/api/v1/marketplace", &token, None).await;
    assert_eq!(entry(&list, "academic-suite")["installs"], 1);
    assert_eq!(list[0]["key"], "academic-suite", "most used comes first");

    // Another workspace's install counts too, but its own name is not shown.
    let token2 = login(&state, "market2@example.com").await;
    // A second workspace of its own (setup() always uses the same slug, so it is made here).
    let (_, ws2_row) = call(&state, "POST", "/api/v1/workspaces", &token2, Some(json!({ "slug": "second-market", "name": "Second" }))).await;
    let ws2 = ws2_row["id"].as_str().unwrap().to_string();
    let (_, plugin2) = call(&state, "POST", &format!("/api/v1/workspaces/{ws2}/plugins/from-catalog"), &token2, Some(json!({ "key": "academic-suite" }))).await;
    call(&state, "POST", &format!("/api/v1/plugins/{}/install", plugin2["id"].as_str().unwrap()), &token2, None).await;
    let (_, list) = call(&state, "GET", "/api/v1/marketplace", &token, None).await;
    assert_eq!(entry(&list, "academic-suite")["installs"], 2);
    assert!(!list.to_string().contains(&ws2), "workspaces are not listed");

    // An unknown entry is not found.
    let (status, _) = call(&state, "POST", &add, &token, Some(json!({ "key": "nope" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
