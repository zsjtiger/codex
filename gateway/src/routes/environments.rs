use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

const ENVIRONMENT_STATUSES: [&str; 4] = ["provisioning", "ready", "stopped", "failed"];

#[derive(Serialize, FromRow)]
pub struct EnvironmentView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub status: String,
    pub endpoint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateEnvironment {
    name: String,
    endpoint: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateEnvironment {
    name: Option<String>,
    status: Option<String>,
    endpoint: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/environments", get(list).post(create))
        .route(
            "/environments/{id}",
            get(get_one).patch(update).delete(delete),
        )
        .route("/environments/{id}/models", get(models))
}

#[derive(Serialize)]
pub struct ModelOption {
    /// The name to pass as `model` when starting a turn.
    pub model: String,
    pub display_name: String,
    pub is_default: bool,
    /// Reasoning efforts this model accepts, for the effort menu.
    pub efforts: Vec<String>,
}

/// The model the environment's codex is configured to use, and only that one. The name comes
/// from `config/read`. Its details come from `model/list` when codex lists it; otherwise the
/// configured name is shown on its own, since codex may not list models from custom providers.
/// Members can read the list.
async fn models(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ModelOption>>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let config = lease
        .request("config/read", serde_json::json!({ "cwd": null }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the configuration: {err}")))?;
    let Some(configured) = config["config"]["model"].as_str().filter(|m| !m.is_empty()) else {
        return Ok(Json(Vec::new()));
    };
    let listed = lease
        .request("model/list", serde_json::json!({}))
        .await
        .unwrap_or(serde_json::Value::Null);
    let details = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["model"].as_str() == Some(configured));
    let option = match details {
        Some(m) => ModelOption {
            model: configured.to_string(),
            display_name: m["displayName"].as_str().unwrap_or(configured).to_string(),
            is_default: true,
            efforts: m["supportedReasoningEfforts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["reasoningEffort"].as_str().map(str::to_string))
                .collect(),
        },
        None => ModelOption {
            model: configured.to_string(),
            display_name: configured.to_string(),
            is_default: true,
            efforts: Vec::new(),
        },
    };
    Ok(Json(vec![option]))
}

const COLUMNS: &str = "id, workspace_id, name, status, endpoint, created_at, updated_at";

async fn load(pool: &sqlx::PgPool, id: Uuid) -> Result<EnvironmentView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM environments WHERE id = $1");
    Ok(sqlx::query_as::<_, EnvironmentView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<EnvironmentView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql =
        format!("SELECT {COLUMNS} FROM environments WHERE workspace_id = $1 ORDER BY created_at");
    let rows = sqlx::query_as::<_, EnvironmentView>(&sql)
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateEnvironment>,
) -> Result<(StatusCode, Json<EnvironmentView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    validate::length("name", &req.name, 1, 100)?;
    if let Some(endpoint) = &req.endpoint {
        validate::endpoint(endpoint)?;
    }

    let mut tx = state.pool.begin().await?;
    let env_id: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (workspace_id, name, endpoint) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(req.endpoint.as_deref())
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "environment.created",
        "environment",
        env_id,
        json!({ "name": req.name }),
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(load(&state.pool, env_id).await?)))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<EnvironmentView>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    Ok(Json(env))
}

/// Status and endpoint are normally written by the runtime orchestrator; admins can set them by hand.
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEnvironment>,
) -> Result<Json<EnvironmentView>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    if let Some(status) = &req.status
        && !ENVIRONMENT_STATUSES.contains(&status.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "status must be one of {}",
            ENVIRONMENT_STATUSES.join(", ")
        )));
    }
    if let Some(endpoint) = &req.endpoint {
        validate::endpoint(endpoint)?;
    }

    sqlx::query(
        "UPDATE environments
         SET name = COALESCE($2, name),
             status = COALESCE($3, status),
             endpoint = COALESCE($4, endpoint),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.status.as_deref())
    .bind(req.endpoint.as_deref())
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

/// Threads that ran here keep their history; their `environment_id` is set to NULL.
async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM environments WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        env.workspace_id,
        user.id,
        "environment.deleted",
        "environment",
        id,
        json!({ "name": env.name }),
    )
    .await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}
