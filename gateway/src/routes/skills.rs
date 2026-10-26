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

const KINDS: [&str; 2] = ["skill", "plugin"];

#[derive(Serialize, FromRow)]
pub struct SkillView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// `skill` or `plugin`. Both add their instructions to new threads.
    pub kind: String,
    pub name: String,
    pub description: String,
    /// Added to new codex threads while the skill is enabled.
    pub instructions: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateSkill {
    kind: Option<String>,
    name: String,
    description: Option<String>,
    instructions: String,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct ListSkillsQuery {
    kind: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSkill {
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    enabled: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/skills", get(list).post(create))
        .route("/skills/{id}", get(get_one).patch(update).delete(delete))
}

const COLUMNS: &str =
    "id, workspace_id, kind, name, description, instructions, enabled, created_at, updated_at";

async fn load(pool: &PgPool, id: Uuid) -> Result<SkillView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM skills WHERE id = $1");
    Ok(sqlx::query_as::<_, SkillView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

fn check_kind(kind: &str) -> Result<(), ApiError> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "kind must be skill or plugin".to_string(),
        ))
    }
}

/// Members can read skills and plugins. Only admins change them, because they affect every
/// member's threads.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListSkillsQuery>,
) -> Result<Json<Vec<SkillView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    if let Some(kind) = &query.kind {
        check_kind(kind)?;
    }
    let sql = format!(
        "SELECT {COLUMNS} FROM skills WHERE workspace_id = $1 AND ($2::text IS NULL OR kind = $2) ORDER BY name"
    );
    Ok(Json(
        sqlx::query_as::<_, SkillView>(&sql)
            .bind(id)
            .bind(query.kind.as_deref())
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSkill>,
) -> Result<(StatusCode, Json<SkillView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    let kind = req.kind.unwrap_or_else(|| "skill".to_string());
    check_kind(&kind)?;
    validate::length("name", &req.name, 1, 100)?;
    let description = req.description.unwrap_or_default();
    validate::length("description", &description, 0, 500)?;
    validate::length("instructions", &req.instructions, 1, 20_000)?;

    let mut tx = state.pool.begin().await?;
    let skill_id: Uuid = sqlx::query_scalar(
        "INSERT INTO skills (workspace_id, kind, name, description, instructions, enabled, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(id)
    .bind(&kind)
    .bind(&req.name)
    .bind(&description)
    .bind(&req.instructions)
    .bind(req.enabled.unwrap_or(true))
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "skill.created",
        "skill",
        skill_id,
        json!({ "name": req.name, "kind": kind }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load(&state.pool, skill_id).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<SkillView>, ApiError> {
    let skill = load(&state.pool, id).await?;
    require_role(&state.pool, skill.workspace_id, user.id, Role::Member).await?;
    Ok(Json(skill))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSkill>,
) -> Result<Json<SkillView>, ApiError> {
    let skill = load(&state.pool, id).await?;
    require_role(&state.pool, skill.workspace_id, user.id, Role::Admin).await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    if let Some(description) = &req.description {
        validate::length("description", description, 0, 500)?;
    }
    if let Some(instructions) = &req.instructions {
        validate::length("instructions", instructions, 1, 20_000)?;
    }
    sqlx::query(
        "UPDATE skills SET name = COALESCE($2, name), description = COALESCE($3, description),
             instructions = COALESCE($4, instructions), enabled = COALESCE($5, enabled), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.description.as_deref())
    .bind(req.instructions.as_deref())
    .bind(req.enabled)
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let skill = load(&state.pool, id).await?;
    require_role(&state.pool, skill.workspace_id, user.id, Role::Admin).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM skills WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        skill.workspace_id,
        user.id,
        "skill.deleted",
        "skill",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
