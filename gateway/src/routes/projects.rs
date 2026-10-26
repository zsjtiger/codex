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
    authz::{Role, ensure_creator_or_admin, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct ProjectView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Added to every thread in the project, through developer instructions.
    pub instructions: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateProject {
    name: String,
    instructions: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateProject {
    name: Option<String>,
    instructions: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/projects", get(list).post(create))
        .route("/projects/{id}", get(get_one).patch(update).delete(delete))
}

const COLUMNS: &str = "id, workspace_id, name, instructions, created_by, created_at, updated_at";

pub async fn load(pool: &PgPool, id: Uuid) -> Result<ProjectView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM projects WHERE id = $1");
    Ok(sqlx::query_as::<_, ProjectView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ProjectView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!(
        "SELECT {COLUMNS} FROM projects WHERE workspace_id = $1 ORDER BY updated_at DESC LIMIT 200"
    );
    Ok(Json(
        sqlx::query_as::<_, ProjectView>(&sql)
            .bind(id)
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateProject>,
) -> Result<(StatusCode, Json<ProjectView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    validate::length("name", &req.name, 1, 100)?;
    let instructions = req.instructions.unwrap_or_default();
    validate::length("instructions", &instructions, 0, 20_000)?;

    let mut tx = state.pool.begin().await?;
    let project_id: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name, instructions, created_by) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&instructions)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "project.created",
        "project",
        project_id,
        json!({ "name": req.name }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load(&state.pool, project_id).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ProjectView>, ApiError> {
    let project = load(&state.pool, id).await?;
    require_role(&state.pool, project.workspace_id, user.id, Role::Member).await?;
    Ok(Json(project))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProject>,
) -> Result<Json<ProjectView>, ApiError> {
    let project = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        project.workspace_id,
        project.created_by,
        user.id,
    )
    .await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    if let Some(instructions) = &req.instructions {
        validate::length("instructions", instructions, 0, 20_000)?;
    }
    sqlx::query(
        "UPDATE projects SET name = COALESCE($2, name), instructions = COALESCE($3, instructions), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.instructions.as_deref())
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

/// Threads in the project stay; they just lose their project.
async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let project = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        project.workspace_id,
        project.created_by,
        user.id,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        project.workspace_id,
        user.id,
        "project.deleted",
        "project",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
