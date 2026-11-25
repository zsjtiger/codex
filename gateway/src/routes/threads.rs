use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub repo_id: Option<Uuid>,
    pub work_branch: Option<String>,
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
    /// A linked repository: the chat becomes a code session in its own checkout.
    repo_id: Option<Uuid>,
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

/// An attachment as shown in the conversation.
#[derive(Clone, Serialize, FromRow)]
pub struct AttachmentRef {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
}

#[derive(Serialize, FromRow)]
pub struct TurnView {
    /// Attachments sent with the turn, for display. Filled in when the turn list is read.
    #[sqlx(skip)]
    pub attachments: Vec<AttachmentRef>,
    /// Ids of the attachments; kept internally and not returned.
    #[serde(skip)]
    #[sqlx(default)]
    pub attachment_ids: Vec<Uuid>,
    /// Files the agent produced for the turn; shown after the attachments sent with it.
    #[serde(skip)]
    #[sqlx(default)]
    pub output_attachment_ids: Vec<Uuid>,
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
    /// The assistant's reply text, saved when the turn's messages complete.
    #[sqlx(default)]
    pub reply: Option<String>,
    /// The user's rating of the reply: "up", "down", or unset.
    #[sqlx(default)]
    pub feedback: Option<String>,
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
    let total: i64 = rows
        .iter()
        .filter(|row| is_text(&row.1))
        .map(|row| row.2)
        .sum();
    if total > MAX_INLINE_ATTACHMENT_BYTES {
        return Err(ApiError::BadRequest(format!(
            "attachments in one message can total at most {MAX_INLINE_ATTACHMENT_BYTES} bytes"
        )));
    }
    let mut text = input.to_string();
    for (filename, content_type, _, storage_key) in rows {
        if !is_text(&content_type) {
            // Binary files (PDF, Word, ...) are shown to the user but their contents are not
            // sent to the model.
            text.push_str(&format!(
                "\n\nAttached file: {filename} ({content_type}). Its contents are not included in this message."
            ));
            continue;
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

/// Text-like attachments are inlined into the message for the model.
fn is_text(content_type: &str) -> bool {
    content_type.starts_with("text/") || content_type == "application/json"
}

const TURN_COLUMNS: &str = "id, thread_id, seq, status, input, error, codex_turn_id, model, effort, reply, feedback, attachment_ids, output_attachment_ids, created_by, created_at, completed_at";

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
        .route("/threads/{id}/work", get(list_work))
        .route("/threads/{id}/diffs", get(list_diffs))
        .route("/threads/{id}/summaries", post(summarize))
        .route("/turns/{id}/interrupt", post(interrupt_turn))
        .route("/threads/{id}/review", post(start_review))
        .route("/threads/{id}/compact", post(start_compact))
        .route("/threads/{id}/steer", post(steer_turn))
        .route("/threads/{id}/fork", post(fork_thread))
        .route("/turns/{id}/feedback", put(set_feedback))
}

const COLUMNS: &str = "id, workspace_id, environment_id, project_id, created_by, title, state, codex_thread_id, \
                       archived_at, created_at, updated_at, last_activity_at, repo_id, work_branch";

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

#[derive(Deserialize)]
pub struct SummarizeRequest {
    /// The completed reply to summarize.
    turn_id: Uuid,
}

/// Asks the model for a document summarizing a completed reply. The new turn's reply is saved as a
/// document of the chat when it completes.
async fn summarize(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<SummarizeRequest>,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let reply: Option<String> = sqlx::query_scalar(
        "SELECT reply FROM turns WHERE id = $1 AND thread_id = $2 AND status = 'completed'",
    )
    .bind(req.turn_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let reply = reply.ok_or_else(|| ApiError::Conflict("that reply has no text to summarize".to_string()))?;
    // The source is cut so the request stays within the input limit.
    let source: String = reply.chars().take(40_000).collect();
    let input = format!(
        "请把下面的回答整理成一份结构清楚的文档：Markdown 格式，第一行是一级标题，只输出文档本身。\n\n---\n\n{source}"
    );

    if !state.turn_limiter.allow(user.id) {
        return Err(ApiError::TooManyRequests);
    }
    let mut tx = state.pool.begin().await?;
    let archived_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM threads WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if archived_at.is_some() {
        return Err(ApiError::Conflict("thread is archived".to_string()));
    }
    let seq: i32 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM turns WHERE thread_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let turn = sqlx::query_as::<_, TurnView>(
        "INSERT INTO turns (thread_id, seq, input, created_by, save_as_document)
         VALUES ($1, $2, $3, $4, true)
         RETURNING id, thread_id, seq, status, input, error, codex_turn_id, model, effort, reply, feedback, attachment_ids, created_by, created_at, completed_at",
    )
    .bind(id)
    .bind(seq)
    .bind(&input)
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

#[derive(Serialize)]
pub struct WorkTurn {
    codex_turn_id: String,
    /// The finished commands and file changes of that turn, in the order they happened.
    items: Vec<serde_json::Value>,
}

/// The finished commands and file changes of a thread's code turns, grouped by turn.
async fn list_work(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<WorkTurn>>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT codex_turn_id, item FROM turn_work WHERE thread_id = $1 ORDER BY seq")
            .bind(id)
            .fetch_all(&state.pool)
            .await?;
    let mut turns: Vec<WorkTurn> = Vec::new();
    for (codex_turn_id, item) in rows {
        match turns.last_mut() {
            Some(turn) if turn.codex_turn_id == codex_turn_id => turn.items.push(item),
            _ => turns.push(WorkTurn { codex_turn_id, items: vec![item] }),
        }
    }
    Ok(Json(turns))
}

#[derive(Serialize, FromRow)]
pub struct TurnDiff {
    /// The chat's turn, when it is still known to the gateway.
    turn_id: Option<Uuid>,
    seq: Option<i32>,
    input: Option<String>,
    codex_turn_id: String,
    /// The unified diff of everything the turn changed.
    diff: String,
}

/// The diffs of a code chat's turns, in the order the turns happened.
async fn list_diffs(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<TurnDiff>>, ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let rows = sqlx::query_as::<_, TurnDiff>(
        "SELECT t.id AS turn_id, t.seq, t.input, d.codex_turn_id, d.diff
           FROM turn_diffs d
           LEFT JOIN turns t ON t.thread_id = d.thread_id AND t.codex_turn_id = d.codex_turn_id
          WHERE d.thread_id = $1
          ORDER BY t.seq NULLS LAST, d.updated_at",
    )
    .bind(id)
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

    if let Some(repo_id) = req.repo_id {
        let in_workspace: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM code_repos WHERE id = $1 AND workspace_id = $2 AND ready)",
        )
        .bind(repo_id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if !in_workspace {
            return Err(ApiError::BadRequest(
                "this repository does not belong to the workspace, or is not ready yet".to_string(),
            ));
        }
    }

    let new_id = Uuid::new_v4();
    let work_branch = req.repo_id.map(|_| crate::code::branch_for(new_id));
    let mut tx = state.pool.begin().await?;
    let thread_id: Uuid = sqlx::query_scalar(
        "INSERT INTO threads (id, workspace_id, environment_id, project_id, created_by, title, repo_id, work_branch)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(new_id)
    .bind(id)
    .bind(req.environment_id)
    .bind(req.project_id)
    .bind(user.id)
    .bind(&title)
    .bind(req.repo_id)
    .bind(work_branch)
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
    let mut rows = sqlx::query_as::<_, TurnView>(&sql)
        .bind(id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;
    let ids: Vec<Uuid> = rows
        .iter()
        .flat_map(|turn| turn.attachment_ids.iter().chain(&turn.output_attachment_ids).copied())
        .collect();
    if !ids.is_empty() {
        let found = sqlx::query_as::<_, AttachmentRef>(
            "SELECT id, filename, content_type, size_bytes FROM attachments WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&state.pool)
        .await?;
        for turn in &mut rows {
            turn.attachments = turn
                .attachment_ids
                .iter()
                .chain(&turn.output_attachment_ids)
                .filter_map(|aid| found.iter().find(|a| a.id == *aid).cloned())
                .collect();
        }
    }
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
    let attachment_ids = req.attachment_ids.clone().unwrap_or_default();
    if attachment_ids.len() > 20 {
        return Err(ApiError::BadRequest(
            "a message can include at most 20 attachments".to_string(),
        ));
    }
    let input = turn_input(
        &state,
        &user,
        access.workspace_id,
        &req.input,
        &attachment_ids,
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
        "INSERT INTO turns (thread_id, seq, input, model, effort, created_by, skill_ids, attachment_ids)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, thread_id, seq, status, input, error, codex_turn_id, model, effort, reply, feedback, attachment_ids, created_by, created_at, completed_at",
    )
    .bind(id)
    .bind(seq)
    .bind(&input)
    .bind(req.model.as_deref())
    .bind(req.effort.as_deref())
    .bind(user.id)
    .bind(&skill_ids)
    .bind(&attachment_ids)
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

#[derive(Deserialize)]
pub struct SetFeedback {
    rating: Option<String>,
}

/// Records whether a reply was helpful: "up" or "down". `null` clears the rating.
async fn set_feedback(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<SetFeedback>,
) -> Result<StatusCode, ApiError> {
    if !matches!(req.rating.as_deref(), None | Some("up") | Some("down")) {
        return Err(ApiError::BadRequest(
            "rating must be \"up\", \"down\", or null".to_string(),
        ));
    }
    let workspace_id: Uuid = sqlx::query_scalar(
        "SELECT th.workspace_id FROM turns t JOIN threads th ON th.id = t.thread_id WHERE t.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    sqlx::query("UPDATE turns SET feedback = $2 WHERE id = $1")
        .bind(id)
        .bind(req.rating)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReviewRequest {
    target: Value,
}

#[derive(Deserialize)]
pub struct SteerRequest {
    input: String,
}

/// The label a review shows in the chat, and the target as app-server takes it. Only the fields
/// app-server reads are passed on.
fn review_target(target: &Value) -> Result<(String, Value), ApiError> {
    let text = |field: &str| target.get(field).and_then(Value::as_str).unwrap_or_default().to_string();
    match text("type").as_str() {
        "uncommittedChanges" => Ok((
            "Review uncommitted changes".to_string(),
            json!({ "type": "uncommittedChanges" }),
        )),
        "baseBranch" => {
            let branch = text("branch");
            validate::length("branch", &branch, 1, 200)?;
            if branch.starts_with('-') {
                return Err(ApiError::BadRequest("a branch name cannot start with '-'".to_string()));
            }
            Ok((
                format!("Review changes against {branch}"),
                json!({ "type": "baseBranch", "branch": branch }),
            ))
        }
        "commit" => {
            let sha = text("sha");
            if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ApiError::BadRequest("a commit is named by 7 to 40 hex characters".to_string()));
            }
            Ok((format!("Review commit {}", &sha[..7]), json!({ "type": "commit", "sha": sha })))
        }
        _ => Err(ApiError::BadRequest(
            "the review target must be uncommittedChanges, baseBranch or commit".to_string(),
        )),
    }
}

/// Adds a turn that is not a plain message (a review, a compaction, or a steer note) to a chat. A
/// queued one runs in its place in the chat's order.
async fn enqueue_special(
    state: &AppState,
    thread_id: Uuid,
    user_id: Uuid,
    input: &str,
    kind: &str,
    target: Option<String>,
    status: &str,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    if !state.turn_limiter.allow(user_id) {
        return Err(ApiError::TooManyRequests);
    }
    let mut tx = state.pool.begin().await?;
    let archived_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM threads WHERE id = $1 FOR UPDATE")
            .bind(thread_id)
            .fetch_one(&mut *tx)
            .await?;
    if archived_at.is_some() {
        return Err(ApiError::Conflict("thread is archived".to_string()));
    }
    let seq: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM turns WHERE thread_id = $1")
            .bind(thread_id)
            .fetch_one(&mut *tx)
            .await?;
    let turn = sqlx::query_as::<_, TurnView>(
        "INSERT INTO turns (thread_id, seq, input, created_by, kind, target, status, completed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, CASE WHEN $7 = 'completed' THEN now() END)
         RETURNING id, thread_id, seq, status, input, error, codex_turn_id, model, effort, reply, feedback, attachment_ids, created_by, created_at, completed_at",
    )
    .bind(thread_id)
    .bind(seq)
    .bind(input)
    .bind(user_id)
    .bind(kind)
    .bind(target)
    .bind(status)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE threads SET last_activity_at = now(), updated_at = now() WHERE id = $1")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if status == "queued" {
        crate::runtime::dispatch::spawn_drain(state.clone(), thread_id);
    }
    Ok((StatusCode::CREATED, Json(turn)))
}

/// Starts a code review of the chat's working copy, a branch's changes, or a commit. It runs as a turn
/// of the chat, through app-server's review.
async fn start_review(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ReviewRequest>,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let (label, target) = review_target(&req.target)?;
    enqueue_special(&state, id, user.id, &label, "review", Some(target.to_string()), "queued").await
}

/// Compacts the chat's history with app-server, so long chats keep room to work.
async fn start_compact(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    enqueue_special(&state, id, user.id, "Compact the conversation", "compact", None, "queued").await
}

/// Adds a note to the turn that is running in the chat, as app-server's steer. The note is kept in the
/// chat, after the turn it was added to.
async fn steer_turn(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<SteerRequest>,
) -> Result<(StatusCode, Json<TurnView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let text = req.input.trim();
    validate::length("input", text, 1, 100_000)?;
    crate::runtime::dispatch::steer(&state, id, text)
        .await
        .map_err(ApiError::Conflict)?;
    enqueue_special(&state, id, user.id, text, "steer", None, "completed").await
}

#[derive(Deserialize)]
pub struct ForkRequest {
    turn_id: Uuid,
}

/// Starts a new chat from a finished reply of this one. The history up to and including that reply is
/// copied, and app-server forks its own thread at the same turn so the new chat carries on from there.
async fn fork_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkRequest>,
) -> Result<(StatusCode, Json<ThreadView>), ApiError> {
    let access = thread_access(&state.pool, id).await?;
    require_role(&state.pool, access.workspace_id, user.id, Role::Member).await?;
    let (title, environment_id, codex_thread_id, project_id, repo_id): (
        String,
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT title, environment_id, codex_thread_id, project_id, repo_id FROM threads WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    if repo_id.is_some() {
        return Err(ApiError::BadRequest("a code session cannot be forked yet".to_string()));
    }
    let (seq, status, codex_turn_id): (i32, String, Option<String>) =
        sqlx::query_as("SELECT seq, status, codex_turn_id FROM turns WHERE id = $1 AND thread_id = $2")
            .bind(req.turn_id)
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    if status != "completed" {
        return Err(ApiError::Conflict("only a finished reply can be forked from".to_string()));
    }
    let (Some(codex_turn_id), Some(codex_thread_id), Some(environment_id)) =
        (codex_turn_id, codex_thread_id, environment_id)
    else {
        return Err(ApiError::Conflict("this chat has nothing to fork from yet".to_string()));
    };
    let endpoint: Option<String> = sqlx::query_scalar("SELECT endpoint FROM environments WHERE id = $1")
        .bind(environment_id)
        .fetch_one(&state.pool)
        .await?;
    let forked = crate::runtime::dispatch::fork_codex_thread(
        &state,
        environment_id,
        endpoint.as_deref(),
        &codex_thread_id,
        &codex_turn_id,
    )
    .await
    .map_err(ApiError::Conflict)?;

    let mut tx = state.pool.begin().await?;
    let new_title: String = format!("{} (fork)", title.chars().take(190).collect::<String>());
    let new_id: Uuid = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, environment_id, created_by, title, project_id, codex_thread_id)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(access.workspace_id)
    .bind(environment_id)
    .bind(user.id)
    .bind(&new_title)
    .bind(project_id)
    .bind(&forked)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO turns (thread_id, seq, input, reply, status, kind, target, created_by, completed_at, model, effort, attachment_ids)
         SELECT $1, ROW_NUMBER() OVER (ORDER BY seq), input, reply, status, kind, target, created_by, completed_at, model, effort, attachment_ids
         FROM turns
         WHERE thread_id = $2 AND seq <= $3 AND status IN ('completed', 'failed', 'interrupted')",
    )
    .bind(new_id)
    .bind(id)
    .bind(seq)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load_thread(&state.pool, new_id).await?)))
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
