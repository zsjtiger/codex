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
    authz::{Role, ensure_creator_or_admin, require_role},
    error::ApiError,
    serde_helpers::double_option,
    state::AppState,
    validate,
};

const KINDS: [&str; 4] = ["document", "slides", "design", "dashboard"];

#[derive(Serialize, FromRow)]
pub struct ArtifactView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Option<Uuid>,
    pub thread_id: Option<Uuid>,
    pub title: String,
    pub kind: String,
    pub content: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateArtifact {
    title: String,
    kind: String,
    content: Option<String>,
    project_id: Option<Uuid>,
    thread_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct UpdateArtifact {
    title: Option<String>,
    content: Option<String>,
    /// Absent keeps the project, `null` removes it, a value moves it.
    #[serde(default, deserialize_with = "double_option")]
    project_id: Option<Option<Uuid>>,
}

#[derive(Deserialize)]
pub struct ListArtifactsQuery {
    project_id: Option<Uuid>,
    kind: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/artifacts", get(list).post(create))
        .route("/artifacts/{id}", get(get_one).patch(update).delete(delete))
}

const COLUMNS: &str = "id, workspace_id, project_id, thread_id, title, kind, content, created_by, created_at, updated_at";

async fn load(pool: &PgPool, id: Uuid) -> Result<ArtifactView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM artifacts WHERE id = $1");
    Ok(sqlx::query_as::<_, ArtifactView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

fn check_kind(kind: &str) -> Result<(), ApiError> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "kind must be one of {}",
            KINDS.join(", ")
        )))
    }
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListArtifactsQuery>,
) -> Result<Json<Vec<ArtifactView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    if let Some(kind) = &query.kind {
        check_kind(kind)?;
    }
    let sql = format!(
        "SELECT {COLUMNS} FROM artifacts
         WHERE workspace_id = $1
           AND ($2::uuid IS NULL OR project_id = $2)
           AND ($3::text IS NULL OR kind = $3)
         ORDER BY updated_at DESC
         LIMIT 200"
    );
    let rows = sqlx::query_as::<_, ArtifactView>(&sql)
        .bind(id)
        .bind(query.project_id)
        .bind(query.kind.as_deref())
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

/// Checks that a referenced project or thread belongs to the same workspace.
async fn ensure_in_workspace(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    workspace_id: Uuid,
) -> Result<(), ApiError> {
    let sql = format!("SELECT EXISTS (SELECT 1 FROM {table} WHERE id = $1 AND workspace_id = $2)");
    let found: bool = sqlx::query_scalar(&sql)
        .bind(id)
        .bind(workspace_id)
        .fetch_one(pool)
        .await?;
    if found {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{table} does not belong to this workspace"
        )))
    }
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateArtifact>,
) -> Result<(StatusCode, Json<ArtifactView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    validate::length("title", &req.title, 1, 200)?;
    check_kind(&req.kind)?;
    let content = req.content.unwrap_or_default();
    validate::length("content", &content, 0, 500_000)?;
    if let Some(project_id) = req.project_id {
        ensure_in_workspace(&state.pool, "projects", project_id, id).await?;
    }
    if let Some(thread_id) = req.thread_id {
        ensure_in_workspace(&state.pool, "threads", thread_id, id).await?;
    }

    let mut tx = state.pool.begin().await?;
    let artifact_id: Uuid = sqlx::query_scalar(
        "INSERT INTO artifacts (workspace_id, project_id, thread_id, title, kind, content, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(id)
    .bind(req.project_id)
    .bind(req.thread_id)
    .bind(&req.title)
    .bind(&req.kind)
    .bind(&content)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "artifact.created",
        "artifact",
        artifact_id,
        json!({ "kind": req.kind }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load(&state.pool, artifact_id).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ArtifactView>, ApiError> {
    let artifact = load(&state.pool, id).await?;
    require_role(&state.pool, artifact.workspace_id, user.id, Role::Member).await?;
    Ok(Json(artifact))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateArtifact>,
) -> Result<Json<ArtifactView>, ApiError> {
    let artifact = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        artifact.workspace_id,
        artifact.created_by,
        user.id,
    )
    .await?;
    if let Some(title) = &req.title {
        validate::length("title", title, 1, 200)?;
    }
    if let Some(content) = &req.content {
        validate::length("content", content, 0, 500_000)?;
    }
    let (set_project, project_id) = match req.project_id {
        None => (false, None),
        Some(None) => (true, None),
        Some(Some(project)) => {
            ensure_in_workspace(&state.pool, "projects", project, artifact.workspace_id).await?;
            (true, Some(project))
        }
    };
    sqlx::query(
        "UPDATE artifacts SET title = COALESCE($2, title), content = COALESCE($3, content),
             project_id = CASE WHEN $4 THEN $5 ELSE project_id END, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.title.as_deref())
    .bind(req.content.as_deref())
    .bind(set_project)
    .bind(project_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let artifact = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        artifact.workspace_id,
        artifact.created_by,
        user.id,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM artifacts WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        artifact.workspace_id,
        user.id,
        "artifact.deleted",
        "artifact",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
