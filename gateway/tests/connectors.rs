//! Connectors (MCP servers) and plugins (skill-style bundles).

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn connectors_are_admin_managed_and_validated(pool: PgPool) {
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let member = login(&state, "member@example.com").await;
    let (_, ws) = call(
        &state,
        "POST",
        "/api/v1/workspaces",
        &owner,
        Some(json!({ "slug": "conn", "name": "C" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/members"),
        &owner,
        Some(json!({ "email": "member@example.com" })),
    )
    .await;
    let connectors = format!("/api/v1/workspaces/{ws_id}/connectors");

    let (status, _) = call(
        &state,
        "POST",
        &connectors,
        &member,
        Some(json!({ "name": "Docs", "url": "https://docs.example.com/mcp" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "members cannot add connectors"
    );

    let (status, bad) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Docs", "url": "file:///etc/passwd" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    let (status, _) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "!!!", "url": "https://x.example" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a name with no usable characters is refused"
    );

    let (status, docs) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Docs Search", "url": "https://docs.example.com/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{docs}");
    assert_eq!(docs["key"], "docs_search");

    // Names that normalise to the same key clash.
    let (status, _) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "docs-search", "url": "https://other.example/mcp" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "'-' becomes '_', so this shares the key"
    );
    let (status, _) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Docs Search", "url": "https://dup.example/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (_, list) = call(&state, "GET", &connectors, &member, None).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "members can read connectors"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn enabled_connectors_are_passed_to_new_codex_threads(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    let connectors = format!("/api/v1/workspaces/{ws}/connectors");
    call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Docs", "url": "https://docs.example.com/mcp" })),
    )
    .await;
    let (_, off) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Off", "url": "https://off.example.com/mcp", "enabled": false })),
    )
    .await;
    assert_eq!(off["enabled"], false);

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello" })),
    )
    .await;
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let requests = mock.requests.lock().unwrap().clone();
    let (_, start) = requests.iter().find(|(m, _)| m == "thread/start").unwrap();
    assert_eq!(
        start["config"]["mcp_servers"]["docs"]["url"],
        "https://docs.example.com/mcp"
    );
    assert!(
        start["config"]["mcp_servers"].get("off").is_none(),
        "disabled connectors are left out"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn plugins_are_skill_bundles_with_their_own_list(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    let skills = format!("/api/v1/workspaces/{ws}/skills");

    let (status, plugin) = call(&state, "POST", &skills, &owner, Some(json!({ "kind": "plugin", "name": "Engineering", "instructions": "Review pull requests carefully." }))).await;
    assert_eq!(status, StatusCode::CREATED, "{plugin}");
    assert_eq!(plugin["kind"], "plugin");
    call(
        &state,
        "POST",
        &skills,
        &owner,
        Some(json!({ "name": "Tests", "instructions": "Run the tests." })),
    )
    .await;

    let (status, _) = call(
        &state,
        "GET",
        &format!("{skills}?kind=widget"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, plugins) = call(
        &state,
        "GET",
        &format!("{skills}?kind=plugin"),
        &owner,
        None,
    )
    .await;
    assert_eq!(plugins.as_array().unwrap().len(), 1);

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello" })),
    )
    .await;
    wait_for_turn(&state, &owner, &thread_id, 0).await;
    let requests = mock.requests.lock().unwrap().clone();
    let (_, start) = requests.iter().find(|(m, _)| m == "thread/start").unwrap();
    let instructions = start["developerInstructions"].as_str().unwrap();
    assert!(
        instructions.contains("Review pull requests carefully."),
        "{instructions}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn no_connectors_means_no_config_is_sent(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello" })),
    )
    .await;
    wait_for_turn(&state, &owner, &thread_id, 0).await;
    let requests = mock.requests.lock().unwrap().clone();
    let (_, start) = requests.iter().find(|(m, _)| m == "thread/start").unwrap();
    assert!(start.get("config").is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn connector_status_follows_codex_startup_reports(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default());
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    let connectors = format!("/api/v1/workspaces/{ws}/connectors");
    let (_, good) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Good", "url": "https://ok.example.com/mcp" })),
    )
    .await;
    let (_, bad) = call(
        &state,
        "POST",
        &connectors,
        &owner,
        Some(json!({ "name": "Bad", "url": "https://down.example.com/mcp" })),
    )
    .await;
    assert!(good["status"].is_null(), "unchecked until codex reports");

    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello" })),
    )
    .await;
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let mut statuses = std::collections::HashMap::new();
    for _ in 0..200 {
        let (_, list) = call(&state, "GET", &connectors, &owner, None).await;
        statuses.clear();
        for c in list.as_array().unwrap() {
            statuses.insert(
                c["key"].as_str().unwrap().to_string(),
                c["status"].as_str().map(str::to_string),
            );
        }
        if statuses.get("good") == Some(&Some("ready".to_string()))
            && statuses.get("bad") == Some(&Some("failed".to_string()))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(statuses.get("good"), Some(&Some("ready".to_string())));
    assert_eq!(statuses.get("bad"), Some(&Some("failed".to_string())));

    let (_, failed) = call(
        &state,
        "GET",
        &format!("/api/v1/connectors/{}", bad["id"].as_str().unwrap()),
        &owner,
        None,
    )
    .await;
    assert_eq!(failed["status_error"], "connection refused");

    // Changing the URL clears the old status until codex reports again.
    let (_, changed) = call(
        &state,
        "PATCH",
        &format!("/api/v1/connectors/{}", bad["id"].as_str().unwrap()),
        &owner,
        Some(json!({ "url": "https://down2.example.com/mcp" })),
    )
    .await;
    assert!(changed["status"].is_null());
    assert!(changed["status_error"].is_null());
}
