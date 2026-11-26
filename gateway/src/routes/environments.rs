use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
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
        .route("/environments/{id}/rate-limits", get(rate_limits))
        .route("/environments/{id}/mcp-servers", get(mcp_servers))
        .route("/environments/{id}/memory", get(codex_memory))
        .route("/environments/{id}/memory/reset", post(reset_codex_memory))
        .route("/environments/{id}/codex-skills", get(codex_skills))
        .route("/environments/{id}/codex-plugins", get(codex_plugins))
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

pub(super) async fn load(pool: &sqlx::PgPool, id: Uuid) -> Result<EnvironmentView, ApiError> {
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

/// One rate-limit window: how much of it is used, how long it is, and when it resets.
#[derive(Serialize)]
pub struct RateWindow {
    used_percent: i64,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Serialize)]
pub struct Credits {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

/// The codex account's limits in an environment. Each environment has its own account, so these are the
/// environment's, not a person's.
#[derive(Serialize)]
pub struct RateLimits {
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary: Option<RateWindow>,
    secondary: Option<RateWindow>,
    credits: Option<Credits>,
    reached_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWindow {
    used_percent: i64,
    window_duration_mins: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCredits {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSnapshot {
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
    credits: Option<RawCredits>,
    rate_limit_reached_type: Option<String>,
}

/// The codex account's rate limits for an environment, as codex reports them now.
async fn rate_limits(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<RateLimits>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let response = lease
        .request("account/rateLimits/read", serde_json::Value::Null)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the limits: {err}")))?;
    let snapshot: RawSnapshot = serde_json::from_value(response["rateLimits"].clone())
        .map_err(|_| ApiError::Internal)?;
    let window = |raw: RawWindow| RateWindow {
        used_percent: raw.used_percent,
        window_minutes: raw.window_duration_mins,
        resets_at: raw.resets_at,
    };
    Ok(Json(RateLimits {
        limit_name: snapshot.limit_name,
        plan_type: snapshot.plan_type,
        primary: snapshot.primary.map(window),
        secondary: snapshot.secondary.map(window),
        credits: snapshot.credits.map(|c| Credits { has_credits: c.has_credits, unlimited: c.unlimited, balance: c.balance }),
        reached_type: snapshot.rate_limit_reached_type,
    }))
}

/// An MCP server as codex has it loaded in an environment right now.
#[derive(Serialize)]
pub struct McpServer {
    /// The server's name in codex's configuration; a connector's key.
    name: String,
    /// Connection state, as codex names it: connected, starting, authenticationRequired, failed, and so on.
    runtime_status: Option<String>,
    auth_status: Option<String>,
    /// Names of the tools the server offers, sorted.
    tool_names: Vec<String>,
    tools_error: Option<String>,
}

/// Every MCP server codex has in an environment, with its connection state and tools. Reads the whole list,
/// a page at a time.
async fn mcp_servers(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<McpServer>>, ApiError> {
    const PAGE_LIMIT: u32 = 100;
    const MAX_PAGES: usize = 20;
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let mut servers = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let response = lease
            .request(
                "mcpServerStatus/list",
                json!({ "detail": "toolsAndAuthOnly", "limit": PAGE_LIMIT, "cursor": cursor }),
            )
            .await
            .map_err(|err| ApiError::BadRequest(format!("could not list the servers: {err}")))?;
        for entry in response["data"].as_array().into_iter().flatten() {
            let Some(name) = entry["name"].as_str() else { continue };
            let mut tool_names: Vec<String> = entry["tools"]
                .as_object()
                .map(|tools| tools.keys().cloned().collect())
                .unwrap_or_default();
            tool_names.sort();
            servers.push(McpServer {
                name: name.to_string(),
                runtime_status: entry["runtimeStatus"].as_str().map(str::to_string),
                auth_status: entry["authStatus"].as_str().map(str::to_string),
                tool_names,
                tools_error: entry["toolsError"].as_str().map(str::to_string),
            });
        }
        cursor = response["nextCursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(Json(servers))
}

/// Codex's own memory in an environment: how many past threads it has consolidated, and whether it is ready
/// to use. This is codex's memory, separate from the notes a person keeps in Baize.
#[derive(Serialize)]
pub struct CodexMemory {
    consolidated_threads: u32,
    ready: bool,
}

async fn codex_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<CodexMemory>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let status = lease
        .request("memory/status", json!({}))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read codex's memory: {err}")))?;
    Ok(Json(CodexMemory {
        consolidated_threads: status["v2ConsolidatedThreads"].as_u64().unwrap_or(0) as u32,
        ready: status["v2Ready"].as_bool().unwrap_or(false),
    }))
}

/// A skill codex found, and whether it is switched on.
#[derive(Serialize)]
pub struct CodexSkill {
    name: String,
    description: String,
    /// Where codex found it: user, repo, system, or admin.
    scope: String,
    enabled: bool,
    path: String,
}

/// A problem codex had reading a skill.
#[derive(Serialize)]
pub struct CodexSkillError {
    path: String,
    message: String,
}

/// What codex found in the environment's skill folders.
#[derive(Serialize)]
pub struct CodexSkills {
    skills: Vec<CodexSkill>,
    errors: Vec<CodexSkillError>,
}

/// The skills codex discovers in an environment, read now. Includes the skills Baize writes for the
/// workspace, since codex finds those through its extra skill roots.
async fn codex_skills(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<CodexSkills>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let response = lease
        .request("skills/list", json!({ "cwds": [], "forceReload": true }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not list the skills: {err}")))?;
    let mut skills = Vec::new();
    let mut errors = Vec::new();
    for entry in response["data"].as_array().into_iter().flatten() {
        for skill in entry["skills"].as_array().into_iter().flatten() {
            let (Some(name), Some(path)) = (skill["name"].as_str(), skill["path"].as_str()) else { continue };
            skills.push(CodexSkill {
                name: name.to_string(),
                description: skill["description"].as_str().unwrap_or_default().to_string(),
                scope: skill["scope"].as_str().unwrap_or_default().to_string(),
                enabled: skill["enabled"].as_bool().unwrap_or(false),
                path: path.to_string(),
            });
        }
        for error in entry["errors"].as_array().into_iter().flatten() {
            errors.push(CodexSkillError {
                path: error["path"].as_str().unwrap_or_default().to_string(),
                message: error["message"].as_str().unwrap_or_default().to_string(),
            });
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(CodexSkills { skills, errors }))
}

/// A plugin codex has installed in an environment.
#[derive(Serialize)]
pub struct CodexPlugin {
    id: String,
    name: String,
    /// The marketplace the plugin came from.
    marketplace: String,
    version: Option<String>,
    enabled: bool,
}

/// A marketplace codex could not load.
#[derive(Serialize)]
pub struct CodexPluginError {
    marketplace_path: String,
    message: String,
}

#[derive(Serialize)]
pub struct CodexPlugins {
    plugins: Vec<CodexPlugin>,
    errors: Vec<CodexPluginError>,
}

/// The plugins codex has installed in an environment, read now. Only installed plugins are listed.
async fn codex_plugins(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<CodexPlugins>, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Member).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let response = lease
        .request("plugin/installed", json!({}))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not list the plugins: {err}")))?;
    let mut plugins = Vec::new();
    for marketplace in response["marketplaces"].as_array().into_iter().flatten() {
        let market = marketplace["name"].as_str().unwrap_or_default().to_string();
        for plugin in marketplace["plugins"].as_array().into_iter().flatten() {
            if plugin["installed"].as_bool() != Some(true) {
                continue;
            }
            let Some(id) = plugin["id"].as_str() else { continue };
            plugins.push(CodexPlugin {
                id: id.to_string(),
                name: plugin["name"].as_str().unwrap_or(id).to_string(),
                marketplace: market.clone(),
                version: plugin["version"].as_str().map(str::to_string),
                enabled: plugin["enabled"].as_bool().unwrap_or(false),
            });
        }
    }
    plugins.sort_by(|a, b| a.marketplace.cmp(&b.marketplace).then_with(|| a.name.cmp(&b.name)));
    let errors = response["marketplaceLoadErrors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|error| CodexPluginError {
            marketplace_path: error["marketplacePath"].as_str().unwrap_or_default().to_string(),
            message: error["message"].as_str().unwrap_or_default().to_string(),
        })
        .collect();
    Ok(Json(CodexPlugins { plugins, errors }))
}

/// Clears codex's own memory in an environment. Only workspace admins can do this: it cannot be undone, and
/// it affects every chat in the environment. Recorded in the audit log.
async fn reset_codex_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let env = load(&state.pool, id).await?;
    require_role(&state.pool, env.workspace_id, user.id, Role::Admin).await?;
    let lease = state
        .runtime
        .lease(id, env.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    lease
        .request("memory/reset", serde_json::Value::Null)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not reset codex's memory: {err}")))?;
    audit::record(&state.pool, env.workspace_id, user.id, "environment.memory_reset", "environment", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}
