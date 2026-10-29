use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
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
    pub project_id: Option<Uuid>,
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
    project_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct UpdateThread {
    title: Option<String>,
    /// `true` archives the thread, `false` restores it.
    archived: Option<bool>,
    /// Absent keeps the project, `null` removes it, a value moves the thread.
    #[serde(default, deserialize_with = "crate::serde_helpers::double_option")]
    project_id: Option<Option<Uuid>>,
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
pub struct ToolUseView {
    pub id: Uuid,
    pub item_id: String,
    pub kind: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
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
    /// Model requested for this turn, if one was chosen.
    pub model: Option<String>,
    /// Reasoning effort requested for this turn, if one was chosen.
    pub effort: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct CreateTurn {
    input: String,
    model: Option<String>,
    effort: Option<String>,
    /// Uploaded text attachments whose contents are added to the turn input.
    attachment_ids: Option<Vec<Uuid>>,
    /// Skills in this workspace to apply to this turn, chosen with "/name" in the composer.
    skill_ids: Option<Vec<Uuid>>,
}

/// Most skills one turn can apply.
const MAX_TURN_SKILLS: usize = 10;

/// Largest total size of text attachments included in one turn.
const MAX_INLINE_ATTACHMENT_BYTES: i64 = 200_000;

/// Appends each attached text file to the message. Only the uploader's own uploaded text
/// files are accepted, so a turn cannot read someone else's attachment.
async fn turn_input(
    state: &AppState,
    user: &CurrentUser,
    workspace_id: Uuid,
    input: &str,
    ids: &[Uuid],
) -> Result<String, ApiError> {
    if ids.is_empty() {
        return Ok(input.to_string());
    }
    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT filename, content_type, size_bytes, storage_key FROM attachments
         WHERE id = ANY($1) AND workspace_id = $2 AND uploaded_by = $3 AND uploaded
         ORDER BY created_at",
    )
    .bind(ids)
    .bind(workspace_id)
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    if rows.len() != ids.iter().collect::<std::collections::HashSet<_>>().len() {
        return Err(ApiError::BadRequest(
            "some attachments are missing, not uploaded, or not yours".to_string(),
        ));
    }
    let total: i64 = rows.iter().map(|row| row.2).sum();
    if total > MAX_INLINE_ATTACHMENT_BYTES {
        return Err(ApiError::BadRequest(format!(
            "attachments in one message can total at most {MAX_INLINE_ATTACHMENT_BYTES} bytes"
        )));
    }
    let mut text = input.to_string();
    for (filename, content_type, _, storage_key) in rows {
        if !(content_type.starts_with("text/") || content_type == "application/json") {
            return Err(ApiError::BadRequest(format!(
                "{filename}: only text attachments can be sent with a message"
            )));
        }
        let bytes = state.storage.get(&storage_key).await.map_err(|err| {
            tracing::error!(%err, "could not read attachment for a turn");
            ApiError::Internal
        })?;
        let content = String::from_utf8(bytes)
            .map_err(|_| ApiError::BadRequest(format!("{filename} is not valid UTF-8 text")))?;
        text.push_str(&format!(
            "\n\nAttached file: {filename}\n```\n{content}\n```"
        ));
    }
    Ok(text)
}

const TURN_COLUMNS: &str = "id, thread_id, seq, status, input, error, codex_turn_id, model, effort, created_by, created_at, completed_at";

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
        .route("/threads/{id}/tool-uses", get(list_tool_uses))
        .route("/turns/{id}/interrupt", post(interrupt_turn))
}

const COLUMNS: &str = "id, workspace_id, environment_id, project_id, created_by, title, state, codex_thread_id, \
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

    if let Some(project_id) = req.project_id {
        let in_workspace: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        )
        .bind(project_id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if !in_workspace {
            return Err(ApiError::BadRequest(
                "project does not belong to this workspace".to_string(),
            ));
        }
    }

    let mut tx = state.pool.begin().await?;
    let thread_id: Uuid = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, environment_id, project_id, created_by, title)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(id)
    .bind(req.environment_id)
    .bind(req.project_id)
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

    let (set_project, project_id) = match req.project_id {
        None => (false, None),
        Some(None) => (true, None),
        Some(Some(project)) => {
            let in_workspace: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
            )
            .bind(project)
            .bind(access.workspace_id)
            .fetch_one(&state.pool)
            .await?;
            if !in_workspace {
                return Err(ApiError::BadRequest(
                    "project does not belong to this workspace".to_string(),
                ));
            }
            (true, Some(project))
        }
    };
    sqlx::query(
        "UPDATE threads
         SET title = COALESCE($2, title),
             archived_at = CASE
                 WHEN $3::bool IS NULL THEN archived_at
                 WHEN $3::bool THEN COALESCE(archived_at, now())
                 ELSE NULL
             END,
             project_id = CASE WHEN $4 THEN $5 ELSE project_id END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.title.as_deref())
    .bind(req.archived)
    .bind(set_project)
    .bind(project_id)
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
    let sql = format!(
        "SELECT {TURN_COLUMNS} FROM turns WHERE thread_id = $1 ORDER BY seq LIMIT $2 OFFSET $3"
    );
    let rows = sqlx::query_as::<_, TurnView>(&sql)
        .bind(id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

/// Tool, search, and file actions Codex completed in the thread, oldest first.
async fn list_tool_uses(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPageQuery>,
) -> Result<Json<Vec<ToolUseView>>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let (limit, offset) = validate::page(query.limit, query.offset)?;
    let rows = sqlx::query_as::<_, ToolUseView>(
        "SELECT id, item_id, kind, label, created_at FROM thread_tool_uses
         WHERE thread_id = $1 ORDER BY created_at, id LIMIT $2 OFFSET $3",
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
    let input = turn_input(
        &state,
        &user,
        access.workspace_id,
        &req.input,
        req.attachment_ids.as_deref().unwrap_or(&[]),
    )
    .await?;
    validate::length("input", &input, 1, 100_000)?;
    if let Some(model) = &req.model {
        validate::length("model", model, 1, 100)?;
    }
    if let Some(effort) = &req.effort {
        validate::length("effort", effort, 1, 32)?;
    }
    let mut skill_ids = req.skill_ids.clone().unwrap_or_default();
    skill_ids.sort();
    skill_ids.dedup();
    if skill_ids.len() > MAX_TURN_SKILLS {
        return Err(ApiError::BadRequest(format!(
            "a turn can apply at most {MAX_TURN_SKILLS} skills"
        )));
    }
    if !skill_ids.is_empty() {
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM skills WHERE id = ANY($1) AND workspace_id = $2 AND kind = 'skill'",
        )
        .bind(&skill_ids)
        .bind(access.workspace_id)
        .fetch_one(&state.pool)
        .await?;
        if found != skill_ids.len() as i64 {
            return Err(ApiError::BadRequest(
                "some skills are not in this workspace".to_string(),
            ));
        }
    }
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
        "INSERT INTO turns (thread_id, seq, input, model, effort, created_by, skill_ids)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, thread_id, seq, status, input, error, codex_turn_id, model, effort, created_by, created_at, completed_at",
    )
    .bind(id)
    .bind(seq)
    .bind(&input)
    .bind(req.model.as_deref())
    .bind(req.effort.as_deref())
    .bind(user.id)
    .bind(&skill_ids)
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

/// Asks app-server to stop a running turn. The turn finishes as `interrupted` once
/// app-server reports it, so this returns before the turn has ended.
async fn interrupt_turn(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let row: (Uuid, String) = sqlx::query_as(
        "SELECT th.workspace_id, t.status FROM turns t JOIN threads th ON th.id = t.thread_id WHERE t.id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    require_role(&state.pool, row.0, user.id, Role::Member).await?;
    if row.1 != "running" {
        return Err(ApiError::Conflict(
            "only a running turn can be interrupted".to_string(),
        ));
    }
    crate::runtime::dispatch::interrupt(&state, id)
        .await
        .map_err(|err| ApiError::Conflict(err.to_string()))?;
    Ok(StatusCode::ACCEPTED)
}
