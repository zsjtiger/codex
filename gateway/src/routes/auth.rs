use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    auth::{CurrentUser, SESSION_TTL_DAYS, hash_token, new_token},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    display_name: Option<String>,
}

#[derive(Serialize)]
pub struct LoginResponse {
    token: String,
    expires_at: DateTime<Utc>,
    user: UserView,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/auth/sessions",
            post(create_session).delete(delete_session),
        )
        .route("/me", get(me))
        .route("/ws-tickets", post(create_ws_ticket))
}

#[derive(Serialize)]
pub struct WsTicket {
    ticket: String,
    expires_at: DateTime<Utc>,
}

/// Issues a single-use ticket for opening `/ws`.
async fn create_ws_ticket(
    State(state): State<AppState>,
    user: CurrentUser,
) -> (StatusCode, Json<WsTicket>) {
    let (ticket, expires_at) = state.tickets.issue(user);
    (StatusCode::CREATED, Json(WsTicket { ticket, expires_at }))
}

/// Development-only login: creates the user if needed and issues a session token.
async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    if !state.dev_login {
        return Err(ApiError::NotFound);
    }
    let email = validate::email(&req.email)?;
    let display_name = req
        .display_name
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_string());
    validate::length("display_name", &display_name, 1, 100)?;

    let mut tx = state.pool.begin().await?;
    // An existing user keeps their display name; the no-op update makes RETURNING work for both cases.
    let user = sqlx::query_as::<_, UserView>(
        "INSERT INTO users (email, display_name) VALUES ($1, $2)
         ON CONFLICT (email) DO UPDATE SET updated_at = now()
         RETURNING id, email, display_name",
    )
    .bind(&email)
    .bind(&display_name)
    .fetch_one(&mut *tx)
    .await?;

    let token = new_token();
    let expires_at = Utc::now() + TimeDelta::days(SESSION_TTL_DAYS);
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user.id)
        .bind(hash_token(&token))
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(LoginResponse {
            token,
            expires_at,
            user,
        }),
    ))
}

/// Revokes the session behind the bearer token.
async fn delete_session(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
        .bind(user.session_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(user: CurrentUser) -> Json<UserView> {
    Json(UserView {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
    })
}
