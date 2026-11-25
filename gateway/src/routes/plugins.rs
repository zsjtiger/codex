//! Plugins: a bundle of skills and connectors that an admin installs into a workspace in one step.
//! Installing adds the bundle's skills and connectors, tagged with the plugin; uninstalling removes
//! the tagged ones, and deleting a plugin uninstalls it first.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool, types::Json as SqlJson};
use uuid::Uuid;

use super::connectors::{check_url, key_for};
use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

const MAX_SKILLS: usize = 50;
const MAX_CONNECTORS: usize = 20;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManifestSkill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManifestConnector {
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Manifest {
    #[serde(default)]
    pub skills: Vec<ManifestSkill>,
    #[serde(default)]
    pub connectors: Vec<ManifestConnector>,
}

#[derive(Serialize, FromRow)]
struct PluginRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    description: String,
    manifest: SqlJson<Manifest>,
    installed: bool,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct PluginView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: String,
    pub manifest: Manifest,
    /// Whether its skills and connectors are in the workspace now.
    pub installed: bool,
    pub created_at: DateTime<Utc>,
}

impl From<PluginRow> for PluginView {
    fn from(row: PluginRow) -> Self {
        Self {
            id: row.id,
            workspace_id: row.workspace_id,
            name: row.name,
            description: row.description,
            manifest: row.manifest.0,
            installed: row.installed,
            created_at: row.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct CreatePlugin {
    name: String,
    description: Option<String>,
    manifest: Manifest,
}

const COLUMNS: &str = "p.id, p.workspace_id, p.name, p.description, p.manifest, p.created_at,
    EXISTS (SELECT 1 FROM skills s WHERE s.plugin_id = p.id)
        OR EXISTS (SELECT 1 FROM connectors c WHERE c.plugin_id = p.id) AS installed";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/plugins", get(list).post(create))
        .route("/plugins/{id}", axum::routing::delete(delete_plugin))
        .route("/plugins/{id}/install", post(install))
        .route("/plugins/{id}/uninstall", post(uninstall))
}

/// Checks a manifest before it is stored or installed: sizes, names, addresses, and no repeats.
fn validate_manifest(manifest: &Manifest) -> Result<(), ApiError> {
    if manifest.skills.is_empty() && manifest.connectors.is_empty() {
        return Err(ApiError::BadRequest("a plugin needs at least one skill or connector".to_string()));
    }
    if manifest.skills.len() > MAX_SKILLS || manifest.connectors.len() > MAX_CONNECTORS {
        return Err(ApiError::BadRequest(format!(
            "a plugin holds at most {MAX_SKILLS} skills and {MAX_CONNECTORS} connectors"
        )));
    }
    let mut skill_names = std::collections::HashSet::new();
    for skill in &manifest.skills {
        validate::length("skill name", &skill.name, 1, 100)?;
        validate::length("skill description", &skill.description, 0, 500)?;
        validate::length("skill instructions", &skill.instructions, 1, 20_000)?;
        if !skill_names.insert(skill.name.trim().to_lowercase()) {
            return Err(ApiError::BadRequest(format!("skill {} is listed twice", skill.name)));
        }
    }
    let mut keys = std::collections::HashSet::new();
    for connector in &manifest.connectors {
        validate::length("connector name", &connector.name, 1, 100)?;
        check_url(&connector.url)?;
        let key = key_for(&connector.name)?;
        if !keys.insert(key) {
            return Err(ApiError::BadRequest(format!("connector {} is listed twice", connector.name)));
        }
    }
    Ok(())
}

async fn load(pool: &PgPool, id: Uuid) -> Result<PluginView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM plugins p WHERE p.id = $1");
    let row = sqlx::query_as::<_, PluginRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(row.into())
}

/// The workspace a plugin belongs to, checked for the caller's role.
async fn plugin_workspace(pool: &PgPool, id: Uuid, user_id: Uuid, role: Role) -> Result<Uuid, ApiError> {
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM plugins WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    require_role(pool, workspace_id, user_id, role).await?;
    Ok(workspace_id)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PluginView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!("SELECT {COLUMNS} FROM plugins p WHERE p.workspace_id = $1 ORDER BY p.name");
    let rows = sqlx::query_as::<_, PluginRow>(&sql).bind(id).fetch_all(&state.pool).await?;
    Ok(Json(rows.into_iter().map(PluginView::from).collect()))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreatePlugin>,
) -> Result<(StatusCode, Json<PluginView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    validate::length("name", &req.name, 1, 100)?;
    let description = req.description.unwrap_or_default();
    validate::length("description", &description, 0, 500)?;
    validate_manifest(&req.manifest)?;

    let mut tx = state.pool.begin().await?;
    let plugin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plugins (workspace_id, name, description, manifest, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&description)
    .bind(SqlJson(&req.manifest))
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "plugin.created",
        "plugin",
        plugin_id,
        json!({ "name": req.name }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load(&state.pool, plugin_id).await?)))
}

async fn install(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginView>, ApiError> {
    let workspace_id = plugin_workspace(&state.pool, id, user.id, Role::Admin).await?;
    let plugin = load(&state.pool, id).await?;
    if plugin.installed {
        return Err(ApiError::Conflict("the plugin is already installed".to_string()));
    }

    let mut tx = state.pool.begin().await?;
    // Names are checked first, so a clash is reported as the plugin's conflict, not a database error.
    for skill in &plugin.manifest.skills {
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM skills WHERE workspace_id = $1 AND name = $2)")
            .bind(workspace_id)
            .bind(&skill.name)
            .fetch_one(&mut *tx)
            .await?;
        if taken {
            return Err(ApiError::Conflict(format!("a skill named {} already exists", skill.name)));
        }
    }
    for connector in &plugin.manifest.connectors {
        let key = key_for(&connector.name)?;
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM connectors WHERE workspace_id = $1 AND key = $2)")
            .bind(workspace_id)
            .bind(&key)
            .fetch_one(&mut *tx)
            .await?;
        if taken {
            return Err(ApiError::Conflict(format!("a connector named {} already exists", connector.name)));
        }
    }
    for skill in &plugin.manifest.skills {
        sqlx::query(
            "INSERT INTO skills (workspace_id, kind, name, description, instructions, enabled, created_by, plugin_id)
             VALUES ($1, 'skill', $2, $3, $4, true, $5, $6)",
        )
        .bind(workspace_id)
        .bind(&skill.name)
        .bind(&skill.description)
        .bind(&skill.instructions)
        .bind(user.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    for connector in &plugin.manifest.connectors {
        sqlx::query(
            "INSERT INTO connectors (workspace_id, name, key, url, enabled, created_by, plugin_id)
             VALUES ($1, $2, $3, $4, true, $5, $6)",
        )
        .bind(workspace_id)
        .bind(&connector.name)
        .bind(key_for(&connector.name)?)
        .bind(&connector.url)
        .bind(user.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    audit::record(
        &mut *tx,
        workspace_id,
        user.id,
        "plugin.installed",
        "plugin",
        id,
        json!({ "skills": plugin.manifest.skills.len(), "connectors": plugin.manifest.connectors.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(load(&state.pool, id).await?))
}

/// Removes the skills and connectors a plugin installed, in one transaction.
async fn remove_installed(pool: &PgPool, plugin_id: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM skills WHERE plugin_id = $1").bind(plugin_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM connectors WHERE plugin_id = $1").bind(plugin_id).execute(&mut *tx).await?;
    tx.commit().await
}

async fn uninstall(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginView>, ApiError> {
    let workspace_id = plugin_workspace(&state.pool, id, user.id, Role::Admin).await?;
    remove_installed(&state.pool, id).await?;
    audit::record(&state.pool, workspace_id, user.id, "plugin.uninstalled", "plugin", id, json!({})).await?;
    Ok(Json(load(&state.pool, id).await?))
}

async fn delete_plugin(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = plugin_workspace(&state.pool, id, user.id, Role::Admin).await?;
    remove_installed(&state.pool, id).await?;
    sqlx::query("DELETE FROM plugins WHERE id = $1").bind(id).execute(&state.pool).await?;
    audit::record(&state.pool, workspace_id, user.id, "plugin.deleted", "plugin", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}
