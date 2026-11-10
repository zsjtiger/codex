//! The signed-in user's saved memory: notes that are added to their new turns when memory is on.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{auth::CurrentUser, error::ApiError, state::AppState, validate};

/// Most memory notes kept per user.
const MAX_NOTES: i64 = 200;

#[derive(Serialize, FromRow)]
pub struct MemoryNote {
    pub id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct MemoryView {
    pub enabled: bool,
    pub notes: Vec<MemoryNote>,
}

#[derive(Deserialize)]
pub struct NoteInput {
    content: String,
}

#[derive(Deserialize)]
pub struct EnabledInput {
    enabled: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/memory", get(get_memory).post(add_note))
        .route("/me/memory/enabled", put(set_enabled))
        .route("/me/memory/{id}", axum::routing::patch(edit_note).delete(delete_note))
}

async fn get_memory(State(state): State<AppState>, user: CurrentUser) -> Result<Json<MemoryView>, ApiError> {
    let enabled = memory_enabled(&state, user.id).await?;
    let notes = sqlx::query_as::<_, MemoryNote>(
        "SELECT id, content, created_at, updated_at FROM user_memories WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(MemoryView { enabled, notes }))
}

async fn add_note(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<NoteInput>,
) -> Result<(StatusCode, Json<MemoryNote>), ApiError> {
    let content = clean(&req.content)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_memories WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    if count >= MAX_NOTES {
        return Err(ApiError::BadRequest(format!("memory is limited to {MAX_NOTES} notes")));
    }
    let note = sqlx::query_as::<_, MemoryNote>(
        "INSERT INTO user_memories (user_id, content) VALUES ($1, $2)
         RETURNING id, content, created_at, updated_at",
    )
    .bind(user.id)
    .bind(content)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(note)))
}

async fn edit_note(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<NoteInput>,
) -> Result<Json<MemoryNote>, ApiError> {
    let content = clean(&req.content)?;
    let note = sqlx::query_as::<_, MemoryNote>(
        "UPDATE user_memories SET content = $3, updated_at = now()
         WHERE id = $1 AND user_id = $2
         RETURNING id, content, created_at, updated_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(content)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(note))
}

async fn delete_note(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM user_memories WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_enabled(State(state): State<AppState>, user: CurrentUser, Json(req): Json<EnabledInput>) -> Result<StatusCode, ApiError> {
    sqlx::query(
        "INSERT INTO user_settings (user_id, memory_enabled) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET memory_enabled = EXCLUDED.memory_enabled, updated_at = now()",
    )
    .bind(user.id)
    .bind(req.enabled)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether the user's saved memory is used in their chats. On by default.
pub async fn memory_enabled(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    let value: Option<bool> = sqlx::query_scalar("SELECT memory_enabled FROM user_settings WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(value.unwrap_or(true))
}

fn clean(content: &str) -> Result<String, ApiError> {
    let trimmed = content.trim();
    validate::length("memory", trimmed, 1, 2000)?;
    Ok(trimmed.to_string())
}
