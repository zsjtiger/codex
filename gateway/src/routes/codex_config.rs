//! Codex's configuration and runtime switches for an environment: what its config requires, changing
//! config values, reloading MCP servers, experimental features, skill switches, and diagnostics. Only
//! workspace admins can use these. Every change is recorded in the audit log, with the key paths and
//! strategies changed but never the values.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
};

/// Longest key path the gateway will pass on.
const MAX_KEY_PATH: usize = 200;
/// Largest value, as JSON, that one edit may carry.
const MAX_VALUE_BYTES: usize = 64 * 1024;
/// Edits in one batch.
const MAX_BATCH: usize = 50;
/// Words that mark a config key as holding a secret. Such keys are refused: their values belong in
/// the environment's secrets, not in codex's config.
const SECRET_WORDS: [&str; 5] = ["key", "token", "secret", "password", "credential"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/environments/{id}/config/requirements", get(requirements))
        .route("/environments/{id}/config/value", put(write_value))
        .route("/environments/{id}/config/batch", post(write_batch))
        .route("/environments/{id}/config/mcp-reload", post(reload_mcp))
        .route("/environments/{id}/experimental-features", get(experimental_features).put(set_experimental_features))
        .route("/environments/{id}/codex-skills/config", post(set_skill_enabled))
        .route("/environments/{id}/diagnostics", get(diagnostics))
}

/// Checks a key path before it goes to codex: dotted names of letters, digits, `_` and `-`, not too long,
/// and not one that names a secret.
fn check_key_path(key_path: &str) -> Result<(), ApiError> {
    let valid = !key_path.is_empty()
        && key_path.len() <= MAX_KEY_PATH
        && key_path.split('.').all(|part| !part.is_empty())
        && key_path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !valid {
        return Err(ApiError::BadRequest(format!("'{key_path}' is not a config key path")));
    }
    let lower = key_path.to_ascii_lowercase();
    if SECRET_WORDS.iter().any(|word| lower.contains(word)) {
        return Err(ApiError::BadRequest(format!(
            "'{key_path}' looks like a secret; keep secrets in the environment's settings, not in codex's config"
        )));
    }
    Ok(())
}

fn check_value(value: &Value) -> Result<(), ApiError> {
    if serde_json::to_vec(value).map(|bytes| bytes.len()).unwrap_or(usize::MAX) > MAX_VALUE_BYTES {
        return Err(ApiError::BadRequest("the value is too large".to_string()));
    }
    Ok(())
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum MergeStrategy {
    Replace,
    Upsert,
}

impl MergeStrategy {
    fn as_str(self) -> &'static str {
        match self {
            MergeStrategy::Replace => "replace",
            MergeStrategy::Upsert => "upsert",
        }
    }
}

#[derive(Deserialize)]
pub struct ConfigEdit {
    key_path: String,
    value: Value,
    merge_strategy: MergeStrategy,
}

impl ConfigEdit {
    fn check(&self) -> Result<(), ApiError> {
        check_key_path(&self.key_path)?;
        check_value(&self.value)
    }

    fn to_params(&self) -> Value {
        json!({ "keyPath": self.key_path, "value": self.value, "mergeStrategy": self.merge_strategy.as_str() })
    }

    /// The audit entry: the key path and strategy, never the value.
    fn audit_entry(&self) -> Value {
        json!({ "key_path": self.key_path, "merge_strategy": self.merge_strategy.as_str() })
    }
}

/// What codex's config requires, as codex reports it. Admins only.
async fn requirements(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let response = lease
        .request("configRequirements/read", Value::Null)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the config requirements: {err}")))?;
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct ValueWrite {
    #[serde(flatten)]
    edit: ConfigEdit,
    /// The version of the config the change is based on; codex refuses the write if it has moved on.
    expected_version: Option<String>,
}

/// Sets one config value. Admins only.
async fn write_value(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ValueWrite>,
) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    body.edit.check()?;
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let mut params = body.edit.to_params();
    params["expectedVersion"] = json!(body.expected_version);
    let response = lease
        .request("config/value/write", params)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not write the config value: {err}")))?;
    audit::record(&state.pool, env.workspace_id, user.id, "environment.config_write", "environment", id, body.edit.audit_entry()).await?;
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct BatchWrite {
    edits: Vec<ConfigEdit>,
    expected_version: Option<String>,
    #[serde(default)]
    reload_user_config: bool,
}

/// Sets several config values in one write. Admins only.
async fn write_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<BatchWrite>,
) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    if body.edits.is_empty() || body.edits.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!("a batch holds 1 to {MAX_BATCH} edits")));
    }
    for edit in &body.edits {
        edit.check()?;
    }
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let params = json!({
        "edits": body.edits.iter().map(ConfigEdit::to_params).collect::<Vec<_>>(),
        "expectedVersion": body.expected_version,
        "reloadUserConfig": body.reload_user_config,
    });
    let response = lease
        .request("config/batchWrite", params)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not write the config: {err}")))?;
    let entries: Vec<Value> = body.edits.iter().map(ConfigEdit::audit_entry).collect();
    audit::record(&state.pool, env.workspace_id, user.id, "environment.config_batch_write", "environment", id, json!({ "edits": entries })).await?;
    Ok(Json(response))
}

/// Reloads codex's MCP servers from the config. Admins only.
async fn reload_mcp(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    lease
        .request("config/mcpServer/reload", Value::Null)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not reload the MCP servers: {err}")))?;
    audit::record(&state.pool, env.workspace_id, user.id, "environment.mcp_reload", "environment", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Codex's experimental features and whether each is on. Admins only.
async fn experimental_features(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let response = lease
        .request("experimentalFeature/list", json!({ "limit": 200 }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not list the experimental features: {err}")))?;
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct FeatureChange {
    /// Feature name to on or off. Features left out keep their state.
    enablement: BTreeMap<String, bool>,
}

/// Turns experimental features on or off for the environment. Admins only.
async fn set_experimental_features(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FeatureChange>,
) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    if body.enablement.is_empty() || body.enablement.len() > 100 {
        return Err(ApiError::BadRequest("name between 1 and 100 features to change".to_string()));
    }
    if body.enablement.keys().any(|name| name.is_empty() || name.len() > 100) {
        return Err(ApiError::BadRequest("a feature name is 1 to 100 characters".to_string()));
    }
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let response = lease
        .request("experimentalFeature/enablement/set", json!({ "enablement": body.enablement }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not change the experimental features: {err}")))?;
    let changed: Map<String, Value> = body.enablement.iter().map(|(name, on)| (name.clone(), json!(on))).collect();
    audit::record(&state.pool, env.workspace_id, user.id, "environment.features_set", "environment", id, json!({ "features": changed })).await?;
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct SkillSwitch {
    /// The skill's name, as codex lists it. Paths are not accepted: a skill is switched by the name codex shows.
    name: String,
    enabled: bool,
}

/// Switches one of codex's skills on or off. Admins only.
async fn set_skill_enabled(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SkillSwitch>,
) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    if body.name.is_empty() || body.name.len() > 200 {
        return Err(ApiError::BadRequest("a skill name is 1 to 200 characters".to_string()));
    }
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let response = lease
        .request("skills/config/write", json!({ "name": body.name, "enabled": body.enabled }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not switch the skill: {err}")))?;
    audit::record(
        &state.pool,
        env.workspace_id,
        user.id,
        "environment.skill_switch",
        "environment",
        id,
        json!({ "skill": body.name, "enabled": body.enabled }),
    )
    .await?;
    Ok(Json(response))
}

/// Codex's own diagnostics for the environment: its process and gauges. Experimental in codex. Admins only.
async fn diagnostics(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let env = super::environments::load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    let lease = lease_for_env(&state, id, env.endpoint.as_deref()).await?;
    let response = lease
        .request("server/diagnostics", Value::Null)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the diagnostics: {err}")))?;
    Ok(Json(response))
}

async fn lease_for_env(state: &AppState, environment_id: Uuid, endpoint: Option<&str>) -> Result<crate::runtime::Lease, ApiError> {
    state
        .runtime
        .lease(environment_id, endpoint)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_paths_must_be_dotted_names() {
        assert!(check_key_path("model").is_ok());
        assert!(check_key_path("features.goals").is_ok());
        assert!(check_key_path("").is_err());
        assert!(check_key_path("a..b").is_err());
        assert!(check_key_path("a b").is_err());
        assert!(check_key_path("a/b").is_err());
        assert!(check_key_path(&"x".repeat(MAX_KEY_PATH + 1)).is_err());
    }

    #[test]
    fn secret_looking_keys_are_refused() {
        for path in ["model_providers.custom.api_key", "mcp_servers.x.env.GITHUB_TOKEN", "auth.password", "Secret.value", "credentials"] {
            assert!(check_key_path(path).is_err(), "{path} should be refused");
        }
    }
}
