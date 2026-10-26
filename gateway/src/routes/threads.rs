use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct ThreadView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub environment_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub title: String,
    pub state: String,
    pub codex_thread_id: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
}

/// Only the fields needed for authorization.
#[derive(FromRow)]
struct ThreadAccess {
    workspace_id: Uuid,
    created_by: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct CreateThread {
    title: Option<String>,
    environment_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct UpdateThread {
    title: Option<String>,
    /// `true` archives the thread, `false` restores it.
    archived: Option<bool>,
}

#[derive(Deserialize)]
pub struct ListThreadsQuery {
    archived: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct ListPageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize, FromRow)]
pub struct TurnView {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub seq: i32,
    pub status: String,
    pub input: String,
    pub error: Option<String>,
    /// Turn id assigned by codex app-server; set once the turn starts running.
    pub codex_turn_id: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct CreateTurn {
    input: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/workspaces/{id}/threads",
            get(list_threads).post(create_thread),
        )
        .route(
            "/threads/{id}",
            get(get_thread).patch(update_thread).delete(delete_thread),
        )
        .route("/threads/{id}/turns", get(list_turns).post(create_turn))
}

const COLUMNS: &str = "id, workspace_id, environment_id, created_by, title, state, codex_thread_id, \
                       archived_at, created_at, updated_at, last_activity_at";

async fn thread_access(pool: &PgPool, id: Uuid) -> Result<ThreadAccess, ApiError> {
    Ok(sqlx::query_as::<_, ThreadAccess>(
        "SELECT workspace_id, created_by FROM threads WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn load_thread(pool: &PgPool, id: Uuid) -> Result<ThreadView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM threads WHERE id = $1");
    Ok(sqlx::query_as::<_, ThreadView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

/// Creators and admins can change a thread. Other members can read it and add turns.
async fn ensure_can_modify(
    pool: &PgPool,
    access: &ThreadAccess,
    user: &CurrentUser,
) -> Result<(), ApiError> {
    let role = require_role(pool, access.workspace_id, user.id, Role::Member).await?;
    if access.created_by == Some(user.id) || role >= Role::Admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn list_threads(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<Vec<ThreadView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let (limit, offset) = validate::page(query.limit, query.offset)?;
    let sql = format!(
        "SELECT {COLUMNS} FROM threads
         WHERE workspace_id = $1
           AND ($2::bool IS NULL OR (archived_at IS NOT NULL) = $2)
         ORDER BY last_activity_at DESC, id
         LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query_as::<_, ThreadView>(&sql)
        .bind(id)
        .bind(query.archived)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

async fn create_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateThread>,
) -> Result<(StatusCode, Json<ThreadView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let title = req.title.unwrap_or_else(|| "Untitled".to_string());
    validate::length("title", &title, 1, 200)?;

    if let Some(env_id) = req.environment_id {
        let belongs: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM environments WHERE id = $1 AND workspace_id = $2)",
        )
        .bind(env_id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if !belongs {
            return Err(ApiError::BadRequest(
                "environment does not belong to this workspace".to_string(),
            ));
        }
    }

    let mut tx = state.pool.begin().await?;
    let thread_id: Uuid = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, environment_id, created_by, title)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(id)
    .bind(req.environment_id)
    .bind(user.id)
    .bind(&title)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "thread.created",
        "thread",
        thread_id,
        json!({ "title": title }),
    )
    .await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(load_thread(&state.pool, thread_id).await?),
    ))
}

async fn get_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ThreadView>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    Ok(Json(load_thread(&state.pool, id).await?))
}

async fn update_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateThread>,
) -> Result<Json<ThreadView>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    ensure_can_modify(&state.pool, &access, &user).await?;
    if let Some(title) = &req.title {
        validate::length("title", title, 1, 200)?;
    }

    sqlx::query(
        "UPDATE threads
         SET title = COALESCE($2, title),
             archived_at = CASE
                 WHEN $3::bool IS NULL THEN archived_at
                 WHEN $3::bool THEN COALESCE(archived_at, now())
                 ELSE NULL
             END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.title.as_deref())
    .bind(req.archived)
    .execute(&state.pool)
    .await?;
    Ok(Json(load_thread(&state.pool, id).await?))
}

async fn delete_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    ensure_can_modify(&state.pool, &access, &user).await?;

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM threads WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        access.workspace_id,
        user.id,
        "thread.deleted",
        "thread",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_turns(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPageQuery>,
) -> Result<Json<Vec<TurnView>>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let (limit, offset) = validate::page(query.limit, query.offset)?;
    let rows = sqlx::query_as::<_, TurnView>(
        "SELECT id, thread_id, seq, status, input, error, codex_turn_id, created_by, created_at, completed_at
         FROM turns
         WHERE thread_id = $1
         ORDER BY seq
         LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

/// Records a new turn as `queued`. Dispatch to codex app-server is a separate step.
async fn create_turn(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTurn>,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    validate::length("input", &req.input, 1, 100_000)?;
    if !state.turn_limiter.allow(user.id) {
        return Err(ApiError::TooManyRequests);
    }

    let mut tx = state.pool.begin().await?;
    // Lock the thread so concurrent turns get distinct sequence numbers.
    let archived_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM threads WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if archived_at.is_some() {
        return Err(ApiError::Conflict("thread is archived".to_string()));
    }

    let seq: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM turns WHERE thread_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    let turn = sqlx::query_as::<_, TurnView>(
        "INSERT INTO turns (thread_id, seq, input, created_by)
         VALUES ($1, $2, $3, $4)
         RETURNING id, thread_id, seq, status, input, error, codex_turn_id, created_by, created_at, completed_at",
    )
    .bind(id)
    .bind(seq)
    .bind(&req.input)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE threads SET last_activity_at = now(), updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    crate::runtime::dispatch::spawn_drain(state.clone(), turn.thread_id);
    Ok((StatusCode::CREATED, Json(turn)))
}
