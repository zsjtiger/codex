//! End-to-end API tests. Each test gets a fresh database via `#[sqlx::test]`,
//! which needs DATABASE_URL pointing at a Postgres server that can create databases.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use codex_gateway::{AppState, app};

async fn call(
    pool: &PgPool,
    dev_login: bool,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = app(AppState::new(pool.clone(), dev_login));
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// Logs in with the dev endpoint and returns `(token, user_id)`.
async fn login(pool: &PgPool, email: &str) -> (String, String) {
    let (status, body) = call(
        pool,
        true,
        "POST",
        "/api/v1/auth/sessions",
        None,
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "login failed: {body}");
    (
        body["token"].as_str().unwrap().to_string(),
        body["user"]["id"].as_str().unwrap().to_string(),
    )
}

async fn create_workspace(pool: &PgPool, token: &str, slug: &str) -> String {
    let (status, body) = call(
        pool,
        true,
        "POST",
        "/api/v1/workspaces",
        Some(token),
        Some(json!({ "slug": slug, "name": "Team" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create workspace failed: {body}"
    );
    body["id"].as_str().unwrap().to_string()
}

#[sqlx::test(migrations = "./migrations")]
async fn login_issues_session_and_me_resolves_it(pool: PgPool) {
    let (token, user_id) = login(&pool, "Alice@Example.com").await;

    let (status, body) = call(&pool, true, "GET", "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], user_id);
    assert_eq!(body["email"], "alice@example.com");

    let (status, _) = call(&pool, true, "GET", "/api/v1/me", Some("cdx_bogus"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_revokes_the_session(pool: PgPool) {
    let (token, _) = login(&pool, "bob@example.com").await;

    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        "/api/v1/auth/sessions",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call(&pool, true, "GET", "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn dev_login_is_hidden_when_disabled(pool: PgPool) {
    let (status, _) = call(
        &pool,
        false,
        "POST",
        "/api/v1/auth/sessions",
        None,
        Some(json!({ "email": "carol@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn workspace_creation_and_visibility(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let (outsider, _) = login(&pool, "outsider@example.com").await;
    let ws = create_workspace(&pool, &owner, "acme").await;

    let (status, body) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], "owner");

    // Non-members get 404, not 403, so workspace existence is not revealed.
    let (status, _) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = call(
        &pool,
        true,
        "POST",
        "/api/v1/workspaces",
        Some(&outsider),
        Some(json!({ "slug": "acme", "name": "Dup" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "duplicate slug: {body}");

    let (status, body) = call(
        &pool,
        true,
        "POST",
        "/api/v1/workspaces",
        Some(&outsider),
        Some(json!({ "slug": "Bad Slug", "name": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid slug: {body}");
}

#[sqlx::test(migrations = "./migrations")]
async fn membership_rules(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let (admin, admin_id) = login(&pool, "admin@example.com").await;
    let (member, member_id) = login(&pool, "member@example.com").await;
    let ws = create_workspace(&pool, &owner, "team").await;
    let members = format!("/api/v1/workspaces/{ws}/members");

    for (email, role) in [
        ("admin@example.com", "admin"),
        ("member@example.com", "member"),
    ] {
        let (status, body) = call(
            &pool,
            true,
            "POST",
            &members,
            Some(&owner),
            Some(json!({ "email": email, "role": role })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "add {email}: {body}");
    }

    // Ownership cannot be assigned through the API.
    let (status, _) = call(
        &pool,
        true,
        "POST",
        &members,
        Some(&owner),
        Some(json!({ "email": "member@example.com", "role": "owner" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Plain members cannot add people.
    let (status, _) = call(
        &pool,
        true,
        "POST",
        &members,
        Some(&member),
        Some(json!({ "email": "admin@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admins cannot change roles; only the owner can.
    let (status, _) = call(
        &pool,
        true,
        "PATCH",
        &format!("{members}/{member_id}"),
        Some(&admin),
        Some(json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(
        &pool,
        true,
        "PATCH",
        &format!("{members}/{member_id}"),
        Some(&owner),
        Some(json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "promote: {body}");

    // Admins cannot remove other admins; the owner can.
    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("{members}/{admin_id}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The promoted member is now an admin too, so the other admin cannot remove them.
    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("{members}/{member_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("{members}/{member_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Members can leave, but the owner cannot.
    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("{members}/{admin_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let owner_id: String = sqlx::query_scalar("SELECT user_id::text FROM workspace_members WHERE role = 'owner' AND workspace_id = $1::uuid")
        .bind(&ws)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("{members}/{owner_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn thread_permissions_and_archiving(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let (alice, _) = login(&pool, "alice@example.com").await;
    let (bob, _) = login(&pool, "bob@example.com").await;
    let ws = create_workspace(&pool, &owner, "dev").await;
    for email in ["alice@example.com", "bob@example.com"] {
        let (status, _) = call(
            &pool,
            true,
            "POST",
            &format!("/api/v1/workspaces/{ws}/members"),
            Some(&owner),
            Some(json!({ "email": email })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, thread) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/threads"),
        Some(&alice),
        Some(json!({ "title": "Refactor" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{thread}");
    let thread_id = thread["id"].as_str().unwrap().to_string();
    assert!(thread["created_by"].is_string());

    // Bob can read the thread but not rename it.
    let (status, _) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/threads/{thread_id}"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &pool,
        true,
        "PATCH",
        &format!("/api/v1/threads/{thread_id}"),
        Some(&bob),
        Some(json!({ "title": "Mine" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Alice (the creator) can archive; archived threads drop out of the default list.
    let (status, body) = call(
        &pool,
        true,
        "PATCH",
        &format!("/api/v1/threads/{thread_id}"),
        Some(&alice),
        Some(json!({ "archived": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["archived_at"].is_string());

    let (_, active) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/threads?archived=false"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(active.as_array().unwrap().len(), 0);
    let (_, archived) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/threads?archived=true"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(archived.as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn turns_get_sequential_numbers_and_reject_archived_threads(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let ws = create_workspace(&pool, &owner, "turns").await;
    let (_, thread) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/threads"),
        Some(&owner),
        Some(json!({})),
    )
    .await;
    let thread_id = thread["id"].as_str().unwrap().to_string();
    let turns = format!("/api/v1/threads/{thread_id}/turns");

    let mut handles = Vec::new();
    for i in 0..5 {
        let pool = pool.clone();
        let token = owner.clone();
        let turns = turns.clone();
        handles.push(tokio::spawn(async move {
            call(
                &pool,
                true,
                "POST",
                &turns,
                Some(&token),
                Some(json!({ "input": format!("message {i}") })),
            )
            .await
        }));
    }
    for handle in handles {
        let (status, body) = handle.await.unwrap();
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["status"], "queued");
    }

    let (status, list) = call(&pool, true, "GET", &turns, Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let seqs: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);

    call(
        &pool,
        true,
        "PATCH",
        &format!("/api/v1/threads/{thread_id}"),
        Some(&owner),
        Some(json!({ "archived": true })),
    )
    .await;
    let (status, _) = call(
        &pool,
        true,
        "POST",
        &turns,
        Some(&owner),
        Some(json!({ "input": "late" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn environments_and_thread_binding(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let ws = create_workspace(&pool, &owner, "envs").await;
    let other_ws = create_workspace(&pool, &owner, "other").await;

    let (status, env) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/environments"),
        Some(&owner),
        Some(json!({ "name": "linux-1", "endpoint": "ws://10.0.0.5:9000" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{env}");
    let env_id = env["id"].as_str().unwrap().to_string();
    assert_eq!(env["status"], "provisioning");

    let (status, _) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/environments"),
        Some(&owner),
        Some(json!({ "name": "bad", "endpoint": "http://nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The runtime marks the environment ready.
    let (status, env) = call(
        &pool,
        true,
        "PATCH",
        &format!("/api/v1/environments/{env_id}"),
        Some(&owner),
        Some(json!({ "status": "ready" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env["status"], "ready");

    // A thread can only bind to an environment in its own workspace.
    let (status, _) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{other_ws}/threads"),
        Some(&owner),
        Some(json!({ "environment_id": env_id })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/threads"),
        Some(&owner),
        Some(json!({ "environment_id": env_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrations = "./migrations")]
async fn attachments_validate_and_filter_by_thread(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let ws = create_workspace(&pool, &owner, "files").await;
    let (_, thread) = call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/threads"),
        Some(&owner),
        Some(json!({})),
    )
    .await;
    let thread_id = thread["id"].as_str().unwrap().to_string();
    let attachments = format!("/api/v1/workspaces/{ws}/attachments");

    let (status, _) = call(&pool, true, "POST", &attachments, Some(&owner), Some(json!({ "filename": "a.png", "content_type": "image/png", "size_bytes": 60 * 1024 * 1024_i64 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "too large");
    let (status, _) = call(&pool, true, "POST", &attachments, Some(&owner), Some(json!({ "filename": "a.png", "content_type": "image/png", "size_bytes": 10, "sha256": "XYZ" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad sha256");

    let sha = "a".repeat(64);
    let (status, created) = call(&pool, true, "POST", &attachments, Some(&owner), Some(json!({ "filename": "notes.md", "content_type": "text/markdown", "size_bytes": 1234, "sha256": sha, "thread_id": thread_id }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(
        created["storage_key"]
            .as_str()
            .unwrap()
            .starts_with(&format!("workspaces/{ws}/attachments/"))
    );
    let attachment_id = created["id"].as_str().unwrap().to_string();

    call(
        &pool,
        true,
        "POST",
        &attachments,
        Some(&owner),
        Some(json!({ "filename": "loose.txt", "content_type": "text/plain", "size_bytes": 5 })),
    )
    .await;

    let (_, in_thread) = call(
        &pool,
        true,
        "GET",
        &format!("{attachments}?thread_id={thread_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(in_thread.as_array().unwrap().len(), 1);
    let (_, all) = call(&pool, true, "GET", &attachments, Some(&owner), None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);

    let (status, _) = call(
        &pool,
        true,
        "DELETE",
        &format!("/api/v1/attachments/{attachment_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
async fn usage_and_audit_are_admin_only(pool: PgPool) {
    let (owner, _) = login(&pool, "owner@example.com").await;
    let (member, _) = login(&pool, "member@example.com").await;
    let ws = create_workspace(&pool, &owner, "ops").await;
    call(
        &pool,
        true,
        "POST",
        &format!("/api/v1/workspaces/{ws}/members"),
        Some(&owner),
        Some(json!({ "email": "member@example.com" })),
    )
    .await;

    let (status, _) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/usage"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/audit-logs"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Usage is recorded by the runtime; insert a row directly to exercise the report.
    let user_id: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email = 'owner@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let ws_uuid: uuid::Uuid = ws.parse().unwrap();
    codex_gateway::routes::usage::record_usage(&pool, ws_uuid, Some(user_id), None, None, 120, 80)
        .await
        .unwrap();

    let (status, report) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/usage"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["totals"]["input_tokens"], 120);
    assert_eq!(report["totals"]["output_tokens"], 80);
    assert_eq!(report["daily"].as_array().unwrap().len(), 1);

    let (status, logs) = call(
        &pool,
        true,
        "GET",
        &format!("/api/v1/workspaces/{ws}/audit-logs"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let actions: Vec<&str> = logs
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["action"].as_str().unwrap())
        .collect();
    assert!(actions.contains(&"workspace.created"));
    assert!(actions.contains(&"member.added"));
}
