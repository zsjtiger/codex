//! The built-in marketplace: a catalog of plugins that ship with Baize. Adding an entry copies its
//! contents into the workspace as a plugin, which an admin then installs. "Most used" counts how many
//! installed plugins came from each entry across all workspaces; only the counts are shown.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;

use super::plugins::{Manifest, PluginView, load, validate_manifest};
use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
};

const CATALOG: &str = include_str!("../../marketplace/catalog.json");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CatalogEntry {
    pub key: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub manifest: Manifest,
}

#[derive(Serialize)]
pub struct CatalogView {
    pub key: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub manifest: Manifest,
    /// Installed plugins that came from this entry, across all workspaces.
    pub installs: i64,
}

#[derive(Deserialize)]
pub struct AddFromCatalog {
    key: String,
}

/// The built-in catalog. It is part of the build, so an invalid file fails the tests.
pub fn catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(CATALOG).expect("the built-in marketplace catalog is valid JSON")
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/marketplace", get(list))
        .route("/workspaces/{id}/plugins/from-catalog", post(add_from_catalog))
}

async fn list(State(state): State<AppState>, _user: CurrentUser) -> Result<Json<Vec<CatalogView>>, ApiError> {
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT p.catalog_key, COUNT(*) FROM plugins p
         WHERE p.catalog_key IS NOT NULL
           AND (EXISTS (SELECT 1 FROM skills s WHERE s.plugin_id = p.id)
                OR EXISTS (SELECT 1 FROM connectors c WHERE c.plugin_id = p.id))
         GROUP BY p.catalog_key",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut views: Vec<CatalogView> = catalog()
        .into_iter()
        .map(|entry| {
            let installs = counts.iter().find(|(key, _)| *key == entry.key).map_or(0, |(_, n)| *n);
            CatalogView {
                key: entry.key,
                name: entry.name,
                description: entry.description,
                publisher: entry.publisher,
                manifest: entry.manifest,
                installs,
            }
        })
        .collect();
    // Most used first; ties keep the catalog's order.
    views.sort_by(|a, b| b.installs.cmp(&a.installs));
    Ok(Json(views))
}

/// Copies a catalog entry into the workspace as a plugin, ready to install.
async fn add_from_catalog(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<AddFromCatalog>,
) -> Result<(StatusCode, Json<PluginView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    let entry = catalog()
        .into_iter()
        .find(|entry| entry.key == req.key)
        .ok_or(ApiError::NotFound)?;
    validate_manifest(&entry.manifest)?;

    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM plugins WHERE workspace_id = $1 AND catalog_key = $2)",
    )
    .bind(id)
    .bind(&entry.key)
    .fetch_one(&state.pool)
    .await?;
    if already {
        return Err(ApiError::Conflict("this plugin is already in the workspace".to_string()));
    }

    let mut tx = state.pool.begin().await?;
    let plugin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plugins (workspace_id, name, description, manifest, created_by, catalog_key)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(id)
    .bind(&entry.name)
    .bind(&entry.description)
    .bind(SqlJson(&entry.manifest))
    .bind(user.id)
    .bind(&entry.key)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "plugin.added_from_marketplace",
        "plugin",
        plugin_id,
        json!({ "catalog_key": entry.key }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load(&state.pool, plugin_id).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_entry_is_a_valid_plugin() {
        let entries = catalog();
        assert!(!entries.is_empty());
        let mut keys = std::collections::HashSet::new();
        for entry in &entries {
            assert!(keys.insert(entry.key.clone()), "{} is listed twice", entry.key);
            validate_manifest(&entry.manifest).unwrap_or_else(|err| panic!("{}: {err:?}", entry.key));
        }
    }
}
