//! Username and password sign-in: registration, sign-in, and the rules for each.

mod common;

use axum::http::StatusCode;
use codex_gateway::{AppState, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn state_for(pool: &PgPool) -> AppState {
    AppState::new(pool.clone(), false, RuntimeConfig::default())
}

#[sqlx::test(migrations = "./migrations")]
async fn accounts_register_and_sign_in_with_username_and_password(pool: PgPool) {
    let state = state_for(&pool);

    let (status, registered) = call(
        &state,
        "POST",
        "/api/v1/auth/users",
        "",
        Some(json!({ "username": "Ada_L", "email": "ada@example.com", "password": "correct horse", "display_name": "Ada" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{registered}");
    let token = registered["token"].as_str().unwrap().to_string();
    let (status, me) = call(&state, "GET", "/api/v1/me", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "ada@example.com");

    // Usernames are case-insensitive.
    let (status, session) = call(
        &state,
        "POST",
        "/api/v1/auth/sessions",
        "",
        Some(json!({ "username": "ada_l", "password": "correct horse" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    assert!(session["token"].as_str().is_some());

    // Wrong passwords and unknown usernames get the same answer.
    for body in [
        json!({ "username": "ada_l", "password": "wrong password" }),
        json!({ "username": "nobody", "password": "correct horse" }),
    ] {
        let (status, body) = call(&state, "POST", "/api/v1/auth/sessions", "", Some(body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn registration_rules_are_enforced(pool: PgPool) {
    let state = state_for(&pool);
    let valid =
        json!({ "username": "grace", "email": "grace@example.com", "password": "long enough" });
    let (status, _) = call(
        &state,
        "POST",
        "/api/v1/auth/users",
        "",
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The same username or email cannot be registered twice.
    let (status, _) = call(&state, "POST", "/api/v1/auth/users", "", Some(valid)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &state,
        "POST",
        "/api/v1/auth/users",
        "",
        Some(
            json!({ "username": "other", "email": "grace@example.com", "password": "long enough" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    for bad in [
        json!({ "username": "ab", "email": "a@example.com", "password": "long enough" }),
        json!({ "username": "has space", "email": "a@example.com", "password": "long enough" }),
        json!({ "username": "fine_name", "email": "a@example.com", "password": "short" }),
    ] {
        let (status, body) = call(&state, "POST", "/api/v1/auth/users", "", Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn email_sign_in_is_development_only(pool: PgPool) {
    let state = state_for(&pool);
    let (status, _) = call(
        &state,
        "POST",
        "/api/v1/auth/sessions",
        "",
        Some(json!({ "email": "dev@example.com" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "email sign-in is off without GATEWAY_DEV_LOGIN"
    );
}
