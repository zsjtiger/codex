//! Projects, skills, artifacts, search, attachment content, and turn options.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn state_for(pool: &PgPool) -> AppState {
    AppState::new(pool.clone(), true, RuntimeConfig::default())
}

/// Creates a workspace owned by `owner` and adds `member` to it. Returns the workspace id.
async fn workspace_with_member(state: &AppState, owner: &str, member: &str) -> String {
    // Members are added by email, so the account must exist first.
    login(state, member).await;
    let (_, ws) = call(
        state,
        "POST",
        "/api/v1/workspaces",
        owner,
        Some(json!({ "slug": "features", "name": "Features" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (status, body) = call(
        state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/members"),
        owner,
        Some(json!({ "email": member })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    ws_id
}

#[sqlx::test(migrations = "./migrations")]
async fn projects_are_editable_by_creator_and_admins(pool: PgPool) {
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let member = login(&state, "member@example.com").await;
    let other = login(&state, "other@example.com").await;
    let ws = workspace_with_member(&state, &owner, "member@example.com").await;
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/members"),
        &owner,
        Some(json!({ "email": "other@example.com" })),
    )
    .await;

    let (status, project) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/projects"),
        &member,
        Some(json!({ "name": "Research", "instructions": "Answer with citations." })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();

    let (status, updated) = call(
        &state,
        "PATCH",
        &format!("/api/v1/projects/{project_id}"),
        &member,
        Some(json!({ "instructions": "Use short answers." })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["instructions"], "Use short answers.");

    // Another member can see the project but cannot change it.
    let (status, _) = call(
        &state,
        "PATCH",
        &format!("/api/v1/projects/{project_id}"),
        &other,
        Some(json!({ "name": "Mine" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, list) = call(
        &state,
        "GET",
        &format!("/api/v1/workspaces/{ws}/projects"),
        &other,
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // The owner (an admin) can delete it.
    let (status, _) = call(
        &state,
        "DELETE",
        &format!("/api/v1/projects/{project_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
async fn skills_are_admin_managed_and_names_are_unique(pool: PgPool) {
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let member = login(&state, "member@example.com").await;
    let ws = workspace_with_member(&state, &owner, "member@example.com").await;
    let skills = format!("/api/v1/workspaces/{ws}/skills");

    let (status, _) = call(
        &state,
        "POST",
        &skills,
        &member,
        Some(json!({ "name": "Review", "instructions": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "members cannot add skills");

    let (status, skill) = call(
        &state,
        "POST",
        &skills,
        &owner,
        Some(json!({ "name": "Review", "instructions": "Check tests." })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{skill}");
    let skill_id = skill["id"].as_str().unwrap().to_string();

    let (status, _) = call(
        &state,
        "POST",
        &skills,
        &owner,
        Some(json!({ "name": "Review", "instructions": "again" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (_, list) = call(&state, "GET", &skills, &member, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, toggled) = call(
        &state,
        "PATCH",
        &format!("/api/v1/skills/{skill_id}"),
        &owner,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(toggled["enabled"], false);
}

#[sqlx::test(migrations = "./migrations")]
async fn artifacts_validate_kind_and_can_move_between_projects(pool: PgPool) {
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let ws = workspace_with_member(&state, &owner, "member@example.com").await;
    let artifacts = format!("/api/v1/workspaces/{ws}/artifacts");

    let (status, _) = call(
        &state,
        "POST",
        &artifacts,
        &owner,
        Some(json!({ "title": "Bad", "kind": "video" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, project) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/projects"),
        &owner,
        Some(json!({ "name": "P" })),
    )
    .await;
    let project_id = project["id"].as_str().unwrap().to_string();

    let (status, artifact) = call(
        &state,
        "POST",
        &artifacts,
        &owner,
        Some(json!({ "title": "Plan", "kind": "document", "content": "# Plan" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{artifact}");
    let artifact_id = artifact["id"].as_str().unwrap().to_string();

    let (status, moved) = call(
        &state,
        "PATCH",
        &format!("/api/v1/artifacts/{artifact_id}"),
        &owner,
        Some(json!({ "project_id": project_id, "content": "# Plan v2" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["project_id"], project_id);
    assert_eq!(moved["content"], "# Plan v2");

    // Absent project_id keeps the project; explicit null removes it.
    let (_, kept) = call(
        &state,
        "PATCH",
        &format!("/api/v1/artifacts/{artifact_id}"),
        &owner,
        Some(json!({ "title": "Plan 2" })),
    )
    .await;
    assert_eq!(kept["project_id"], project_id);
    let (_, cleared) = call(
        &state,
        "PATCH",
        &format!("/api/v1/artifacts/{artifact_id}"),
        &owner,
        Some(json!({ "project_id": null })),
    )
    .await;
    assert!(cleared["project_id"].is_null());

    let (_, filtered) = call(
        &state,
        "GET",
        &format!("{artifacts}?kind=slides"),
        &owner,
        None,
    )
    .await;
    assert!(filtered.as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn search_covers_chats_messages_artifacts_and_projects(pool: PgPool) {
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let ws = workspace_with_member(&state, &owner, "member@example.com").await;

    let (_, thread) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/threads"),
        &owner,
        Some(json!({ "title": "Binary search notes" })),
    )
    .await;
    call(
        &state,
        "POST",
        &format!("/api/v1/threads/{}/turns", thread["id"].as_str().unwrap()),
        &owner,
        Some(json!({ "input": "why is 100% of the array scanned?" })),
    )
    .await;
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/artifacts"),
        &owner,
        Some(json!({ "title": "Search report", "kind": "document" })),
    )
    .await;
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/projects"),
        &owner,
        Some(json!({ "name": "Search project" })),
    )
    .await;

    let search = format!("/api/v1/workspaces/{ws}/search?q=search");
    let (status, results) = call(&state, "GET", &search, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{results}");
    assert_eq!(results["threads"].as_array().unwrap().len(), 1);
    assert_eq!(results["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(results["projects"].as_array().unwrap().len(), 1);

    // Message text is searched too, and % is matched literally rather than as a wildcard.
    let (_, messages) = call(
        &state,
        "GET",
        &format!("/api/v1/workspaces/{ws}/search?q=100%25"),
        &owner,
        None,
    )
    .await;
    assert_eq!(messages["turns"].as_array().unwrap().len(), 1);
    let (_, none) = call(
        &state,
        "GET",
        &format!("/api/v1/workspaces/{ws}/search?q=%25"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        none["turns"].as_array().unwrap().len(),
        1,
        "a bare % matches only the literal percent sign"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn attachment_bytes_round_trip_and_are_checked(pool: PgPool) {
    let storage = std::env::temp_dir().join(format!("gateway-attachments-{}", Uuid::new_v4()));
    let state = state_for(&pool).with_storage(storage.clone());
    let owner = login(&state, "owner@example.com").await;
    let ws = workspace_with_member(&state, &owner, "member@example.com").await;
    let attachments = format!("/api/v1/workspaces/{ws}/attachments");

    let bytes = b"hello, attachment".to_vec();
    let (_, created) = call(
        &state,
        "POST",
        &attachments,
        &owner,
        Some(json!({ "filename": "note.txt", "content_type": "text/plain", "size_bytes": bytes.len() })),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let content = format!("/api/v1/attachments/{id}/content");

    // Nothing to download before the upload.
    let (status, _, _) = call_raw(&state, "GET", &content, &owner, "text/plain", vec![]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = call_raw(
        &state,
        "PUT",
        &content,
        &owner,
        "text/plain",
        b"short".to_vec(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "size must match the metadata"
    );

    let (status, _, _) =
        call_raw(&state, "PUT", &content, &owner, "text/plain", bytes.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) =
        call_raw(&state, "PUT", &content, &owner, "text/plain", bytes.clone()).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an attachment is uploaded once"
    );

    let (status, content_type, downloaded) =
        call_raw(&state, "GET", &content, &owner, "text/plain", vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("text/plain"));
    assert_eq!(downloaded, bytes);

    let _ = std::fs::remove_dir_all(storage);
}

#[sqlx::test(migrations = "./migrations")]
async fn turn_options_and_project_instructions_reach_app_server(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let (ws, thread_id) = setup(&state, &owner, Some(&mock.url)).await;
    call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/skills"),
        &owner,
        Some(json!({ "name": "Tests", "instructions": "Run the tests." })),
    )
    .await;
    let (_, project) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws}/projects"),
        &owner,
        Some(json!({ "name": "P", "instructions": "Be brief." })),
    )
    .await;
    call(
        &state,
        "PATCH",
        &format!("/api/v1/threads/{thread_id}"),
        &owner,
        Some(json!({ "project_id": project["id"] })),
    )
    .await;

    let (status, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "hello", "model": "gpt-test", "effort": "high" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    assert_eq!(turn["model"], "gpt-test");
    assert_eq!(turn["effort"], "high");
    wait_for_turn(&state, &owner, &thread_id, 0).await;

    let requests = mock.requests.lock().unwrap().clone();
    let (_, thread_start) = requests.iter().find(|(m, _)| m == "thread/start").unwrap();
    let instructions = thread_start["developerInstructions"].as_str().unwrap();
    assert!(
        instructions.contains("Be brief.") && instructions.contains("Run the tests."),
        "{instructions}"
    );
    let (_, turn_start) = requests.iter().find(|(m, _)| m == "turn/start").unwrap();
    assert_eq!(turn_start["model"], "gpt-test");
    assert_eq!(turn_start["effort"], "high");
}

#[sqlx::test(migrations = "./migrations")]
async fn running_turn_can_be_interrupted(pool: PgPool) {
    let mock = mock_app_server().await;
    let state = state_for(&pool);
    let owner = login(&state, "owner@example.com").await;
    let (_, thread_id) = setup(&state, &owner, Some(&mock.url)).await;

    let (_, turn) = call(
        &state,
        "POST",
        &format!("/api/v1/threads/{thread_id}/turns"),
        &owner,
        Some(json!({ "input": "SLOW long task" })),
    )
    .await;
    let turn_id = turn["id"].as_str().unwrap().to_string();
    for _ in 0..200 {
        let (_, turns) = call(
            &state,
            "GET",
            &format!("/api/v1/threads/{thread_id}/turns"),
            &owner,
            None,
        )
        .await;
        if turns[0]["status"] == "running" && turns[0]["codex_turn_id"].is_string() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let (status, _) = call(
        &state,
        "POST",
        &format!("/api/v1/turns/{turn_id}/interrupt"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let requests = mock.requests.lock().unwrap().clone();
    let (_, interrupt) = requests
        .iter()
        .find(|(m, _)| m == "turn/interrupt")
        .expect("turn/interrupt sent");
    assert_eq!(interrupt["threadId"], "thr_mock");

    // Interrupting a finished turn is refused.
    wait_for_turn(&state, &owner, &thread_id, 0).await;
    let (status, _) = call(
        &state,
        "POST",
        &format!("/api/v1/turns/{turn_id}/interrupt"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}
