use axum::{
    Json, Router,
    extract::{Path, State},
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
pub struct WorkspaceView {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// The caller's role in this workspace.
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateWorkspace {
    slug: String,
    name: String,
}

#[derive(Deserialize)]
pub struct UpdateWorkspace {
    name: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces", get(list).post(create))
        .route(
            "/workspaces/{id}",
            get(get_one).patch(update).delete(delete),
        )
}

async fn get_view(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<WorkspaceView, ApiError> {
    let view = sqlx::query_as::<_, WorkspaceView>(
        "SELECT w.id, w.slug, w.name, m.role, w.created_at, w.updated_at
         FROM workspaces w
         JOIN workspace_members m ON m.workspace_id = w.id AND m.user_id = $2
         WHERE w.id = $1",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(view)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<WorkspaceView>>, ApiError> {
    let rows = sqlx::query_as::<_, WorkspaceView>(
        "SELECT w.id, w.slug, w.name, m.role, w.created_at, w.updated_at
         FROM workspace_members m
         JOIN workspaces w ON w.id = m.workspace_id
         WHERE m.user_id = $1
         ORDER BY w.created_at",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

/// Creates a workspace and makes the caller its owner.
async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateWorkspace>,
) -> Result<(StatusCode, Json<WorkspaceView>), ApiError> {
    validate::slug(&req.slug)?;
    validate::length("name", &req.name, 1, 100)?;

    let mut tx = state.pool.begin().await?;
    let workspace_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workspaces (slug, name, created_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&req.slug)
    .bind(&req.name)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(workspace_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        workspace_id,
        user.id,
        "workspace.created",
        "workspace",
        workspace_id,
        json!({ "slug": req.slug }),
    )
    .await?;
    tx.commit().await?;

    let view = get_view(&state.pool, workspace_id, user.id).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkspaceView>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    Ok(Json(get_view(&state.pool, id, user.id).await?))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateWorkspace>,
) -> Result<Json<WorkspaceView>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    sqlx::query(
        "UPDATE workspaces SET name = COALESCE($2, name), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .execute(&state.pool)
    .await?;
    Ok(Json(get_view(&state.pool, id, user.id).await?))
}

/// Deleting a workspace cascades to its threads, environments, files and audit log.
async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_role(&state.pool, id, user.id, Role::Owner).await?;
    sqlx::query("DELETE FROM workspaces WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
