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
    password,
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
    /// Username and password sign-in.
    username: Option<String>,
    password: Option<String>,
    /// Development sign-in by email, used only when no username is given.
    email: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    username: String,
    email: String,
    password: String,
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
        .route("/auth/users", post(register))
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

/// Signs in with a username and password. Without a username, development sign-in by email is
/// used, and only when the gateway runs with `GATEWAY_DEV_LOGIN`.
async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    if let Some(username) = req.username {
        let password = req.password.unwrap_or_default();
        return sign_in_with_password(&state, &username, &password).await;
    }
    if !state.dev_login {
        return Err(ApiError::NotFound);
    }
    let email = validate::email(req.email.as_deref().unwrap_or_default())?;
    let display_name = req
        .display_name
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_string());
    validate::length("display_name", &display_name, 1, 100)?;

    // An existing user keeps their display name; the no-op update makes RETURNING work for both cases.
    let user = sqlx::query_as::<_, UserView>(
        "INSERT INTO users (email, display_name) VALUES ($1, $2)
         ON CONFLICT (email) DO UPDATE SET updated_at = now()
         RETURNING id, email, display_name",
    )
    .bind(&email)
    .bind(&display_name)
    .fetch_one(&state.pool)
    .await?;
    start_session(&state, user).await
}

async fn sign_in_with_password(
    state: &AppState,
    username: &str,
    password: &str,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    let username = username.trim().to_lowercase();
    let row: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email, display_name, password_hash FROM users WHERE username = $1",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await?;
    // Unknown usernames and wrong passwords get the same answer.
    let (stored_hash, user) = match row {
        Some((id, email, display_name, hash)) => (
            hash,
            Some(UserView {
                id,
                email,
                display_name,
            }),
        ),
        None => (None, None),
    };
    let password_ok = password::verify(password, stored_hash.as_deref());
    match user {
        Some(user) if password_ok => start_session(state, user).await,
        _ => Err(ApiError::Unauthorized),
    }
}

/// Creates an account with a username, email, and password, then signs it in.
async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    let username = req.username.trim().to_lowercase();
    let valid_username = (3..=32).contains(&username.len())
        && username
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    if !valid_username {
        return Err(ApiError::BadRequest(
            "username must be 3-32 characters: letters, digits, _, . or -".to_string(),
        ));
    }
    let email = validate::email(&req.email)?;
    if !(8..=200).contains(&req.password.len()) {
        return Err(ApiError::BadRequest(
            "password must be 8-200 characters".to_string(),
        ));
    }
    let display_name = req.display_name.unwrap_or_else(|| username.clone());
    validate::length("display_name", &display_name, 1, 100)?;

    let hash = password::hash(&req.password).map_err(|err| {
        tracing::error!(%err, "could not hash password");
        ApiError::Internal
    })?;
    let user = sqlx::query_as::<_, UserView>(
        "INSERT INTO users (email, display_name, username, password_hash) VALUES ($1, $2, $3, $4)
         RETURNING id, email, display_name",
    )
    .bind(&email)
    .bind(&display_name)
    .bind(&username)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    .map_err(|err| match &err {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            ApiError::Conflict("that username or email is already in use".to_string())
        }
        _ => err.into(),
    })?;
    start_session(&state, user).await
}

/// Issues a session token for a signed-in user.
async fn start_session(
    state: &AppState,
    user: UserView,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    let token = new_token();
    let expires_at = Utc::now() + TimeDelta::days(SESSION_TTL_DAYS);
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user.id)
        .bind(hash_token(&token))
        .bind(expires_at)
        .execute(&state.pool)
        .await?;
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
