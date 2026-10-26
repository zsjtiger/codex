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
pub struct ConnectorView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// The MCP server key used in codex's configuration.
    pub key: String,
    pub url: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateConnector {
    name: String,
    url: String,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateConnector {
    name: Option<String>,
    url: Option<String>,
    enabled: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/connectors", get(list).post(create))
        .route(
            "/connectors/{id}",
            get(get_one).patch(update).delete(delete),
        )
}

const COLUMNS: &str = "id, workspace_id, name, key, url, enabled, created_at, updated_at";

/// Turns a display name into a key codex accepts: lowercase letters, digits, `_` and `-`.
pub fn key_for(name: &str) -> Result<String, ApiError> {
    let key: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let key = key.trim_matches('_').to_string();
    if key.is_empty() || key.len() > 40 {
        return Err(ApiError::BadRequest(
            "name must contain letters or digits (up to 40 after normalising)".to_string(),
        ));
    }
    Ok(key)
}

fn check_url(url: &str) -> Result<(), ApiError> {
    if (url.starts_with("http://") || url.starts_with("https://")) && url.len() <= 2000 {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "url must start with http:// or https://".to_string(),
        ))
    }
}

async fn load(pool: &PgPool, id: Uuid) -> Result<ConnectorView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM connectors WHERE id = $1");
    Ok(sqlx::query_as::<_, ConnectorView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

/// Members can see connectors. Admins add, change and remove them, because each one gives
/// codex access to an external server for every thread.
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ConnectorView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!("SELECT {COLUMNS} FROM connectors WHERE workspace_id = $1 ORDER BY name");
    Ok(Json(
        sqlx::query_as::<_, ConnectorView>(&sql)
            .bind(id)
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateConnector>,
) -> Result<(StatusCode, Json<ConnectorView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    validate::length("name", &req.name, 1, 100)?;
    check_url(&req.url)?;
    let key = key_for(&req.name)?;

    let mut tx = state.pool.begin().await?;
    let connector_id: Uuid = sqlx::query_scalar(
        "INSERT INTO connectors (workspace_id, name, key, url, enabled, created_by)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&key)
    .bind(&req.url)
    .bind(req.enabled.unwrap_or(true))
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "connector.created",
        "connector",
        connector_id,
        json!({ "key": key }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load(&state.pool, connector_id).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ConnectorView>, ApiError> {
    let connector = load(&state.pool, id).await?;
    require_role(&state.pool, connector.workspace_id, user.id, Role::Member).await?;
    Ok(Json(connector))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateConnector>,
) -> Result<Json<ConnectorView>, ApiError> {
    let connector = load(&state.pool, id).await?;
    require_role(&state.pool, connector.workspace_id, user.id, Role::Admin).await?;
    if let Some(url) = &req.url {
        check_url(url)?;
    }
    // A new name changes the key too, so the key stays a valid config key.
    let key = match &req.name {
        Some(name) => {
            validate::length("name", name, 1, 100)?;
            Some(key_for(name)?)
        }
        None => None,
    };
    sqlx::query(
        "UPDATE connectors SET name = COALESCE($2, name), key = COALESCE($3, key), url = COALESCE($4, url),
             enabled = COALESCE($5, enabled), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(key.as_deref())
    .bind(req.url.as_deref())
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
    let connector = load(&state.pool, id).await?;
    require_role(&state.pool, connector.workspace_id, user.id, Role::Admin).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM connectors WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        connector.workspace_id,
        user.id,
        "connector.deleted",
        "connector",
        id,
        json!({ "key": connector.key }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
