use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
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
    /// Last startup status codex reported: starting, ready, failed or cancelled. Null until checked.
    pub status: Option<String>,
    pub status_error: Option<String>,
    pub status_at: Option<DateTime<Utc>>,
    /// Sign-in state for connectors that use OAuth: pending, signed_in or failed. Null when
    /// no sign-in has been started.
    pub oauth_state: Option<String>,
    pub oauth_error: Option<String>,
    pub oauth_updated_at: Option<DateTime<Utc>>,
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
        .route("/connectors/{id}/sign-in", post(sign_in))
        .route("/connectors/oauth/callback", get(oauth_callback))
        .route("/connectors/oauth/callback/{*rest}", get(oauth_callback))
}

/// Where the provider sends the browser after sign-in. The gateway forwards it to codex.
pub const CALLBACK_PATH: &str = "/api/v1/connectors/oauth/callback";

/// The gateway's public address, from `GATEWAY_PUBLIC_URL`. Without it, OAuth sign-in is off.
pub fn public_base() -> Option<String> {
    std::env::var("GATEWAY_PUBLIC_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

/// The fixed port codex's local OAuth listener uses on the runtime host.
pub fn callback_port() -> u16 {
    std::env::var("GATEWAY_OAUTH_CALLBACK_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(17600)
}

/// How long a sign-in may wait for the browser before it is marked failed.
const SIGN_IN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

const COLUMNS: &str = "id, workspace_id, name, key, url, enabled, status, status_error, status_at, oauth_state, oauth_error, oauth_updated_at, created_at, updated_at";

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
             enabled = COALESCE($5, enabled),
             status = CASE WHEN $4 IS NOT NULL OR $5 IS NOT NULL THEN NULL ELSE status END,
             status_error = CASE WHEN $4 IS NOT NULL OR $5 IS NOT NULL THEN NULL ELSE status_error END,
             updated_at = now()
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

#[derive(Deserialize, Default)]
pub struct SignInRequest {
    /// The runtime to sign in on. Defaults to the workspace's first environment.
    environment_id: Option<Uuid>,
}

/// The host of a runtime endpoint such as `ws://10.0.0.5:4500`. Local runtimes have no endpoint
/// and run on this host.
pub fn endpoint_host(endpoint: Option<&str>) -> String {
    let Some(endpoint) = endpoint else {
        return "127.0.0.1".to_string();
    };
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, rest)| rest);
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or_default().to_string();
    }
    authority.rsplit_once(':').map_or(authority, |(host, _)| host).to_string()
}

/// The OAuth `state` value in an authorization URL.
fn state_of(authorization_url: &str) -> Option<String> {
    let query = authorization_url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("state="))
        .map(|value| value.split('#').next().unwrap_or_default().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Serialize)]
pub struct SignInStarted {
    pub authorization_url: String,
}

/// Starts OAuth sign-in for a connector. Codex returns the provider's authorization URL, which
/// the browser opens. When the provider calls back, codex stores the grant and the result is
/// recorded on the connector.
async fn sign_in(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    body: Option<Json<SignInRequest>>,
) -> Result<Json<SignInStarted>, ApiError> {
    let requested = body.map(|Json(req)| req.environment_id).unwrap_or_default();
    if public_base().is_none() {
        return Err(ApiError::BadRequest(
            "sign-in needs GATEWAY_PUBLIC_URL to be set on the gateway".to_string(),
        ));
    }
    let (workspace_id, key): (Uuid, String) = sqlx::query_as("SELECT workspace_id, key FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    let (environment_id, endpoint): (Uuid, Option<String>) = sqlx::query_as(
        "SELECT id, endpoint FROM environments
         WHERE workspace_id = $1 AND ($2::uuid IS NULL OR id = $2)
         ORDER BY created_at LIMIT 1",
    )
    .bind(workspace_id)
    .bind(requested)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::BadRequest("the workspace has no matching environment to sign in on".to_string()))?;

    let lease = state
        .runtime
        .lease(environment_id, endpoint.as_deref())
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not reach the runtime for connector sign-in");
            ApiError::Internal
        })?;
    // Subscribe before starting the login so its completion cannot be missed.
    let mut events = lease.subscribe();
    let config = crate::runtime::dispatch::codex_mcp_config(&state.pool, workspace_id)
        .await?
        .unwrap_or_else(|| json!({}));
    let started = lease
        .request("thread/start", json!({ "approvalPolicy": "on-request", "config": config }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not start the sign-in: {err}")))?;
    let codex_thread = started
        .pointer("/thread/id")
        .and_then(serde_json::Value::as_str)
        .ok_or(ApiError::Internal)?
        .to_string();
    let login = lease
        .request(
            "mcpServer/oauth/login",
            json!({ "name": key, "threadId": codex_thread }),
        )
        .await
        .map_err(|err| ApiError::BadRequest(format!("this connector cannot sign in: {err}")))?;
    let authorization_url = login
        .get("authorizationUrl")
        .and_then(serde_json::Value::as_str)
        .ok_or(ApiError::Internal)?
        .to_string();
    let login_id = login.get("loginId").and_then(serde_json::Value::as_str).map(str::to_string);

    // The provider's redirect carries the state; remember which runtime's listener it belongs to.
    if let Some(oauth_state) = state_of(&authorization_url) {
        sqlx::query(
            "INSERT INTO oauth_callbacks (state, target_host, target_port) VALUES ($1, $2, $3)
             ON CONFLICT (state) DO UPDATE SET target_host = EXCLUDED.target_host, target_port = EXCLUDED.target_port",
        )
        .bind(oauth_state)
        .bind(endpoint_host(endpoint.as_deref()))
        .bind(i32::from(callback_port()))
        .execute(&state.pool)
        .await?;
    }

    sqlx::query("UPDATE connectors SET oauth_state = 'pending', oauth_error = NULL, oauth_updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    // Holds the runtime connection until the browser finishes or the sign-in times out.
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(SIGN_IN_TIMEOUT, async {
            loop {
                match events.recv().await {
                    Ok(event) if event.method == "mcpServer/oauthLogin/completed" => {
                        let matches = match &login_id {
                            Some(expected) => event.params.get("loginId").and_then(serde_json::Value::as_str) == Some(expected.as_str()),
                            None => event.params.get("name").and_then(serde_json::Value::as_str) == Some(key.as_str()),
                        };
                        if matches {
                            return event.params;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return serde_json::Value::Null,
                }
            }
        })
        .await;
        let (state_text, error) = match outcome {
            Ok(params) if params.get("success").and_then(serde_json::Value::as_bool) == Some(true) => ("signed_in", None),
            Ok(params) => (
                "failed",
                Some(params.get("error").and_then(serde_json::Value::as_str).unwrap_or("sign-in was not completed").to_string()),
            ),
            Err(_) => ("failed", Some("sign-in timed out".to_string())),
        };
        let _ = sqlx::query("UPDATE connectors SET oauth_state = $2, oauth_error = $3, oauth_updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(state_text)
            .bind(error)
            .execute(&pool)
            .await;
        drop(lease);
    });

    Ok(Json(SignInStarted { authorization_url }))
}

/// Receives the provider's redirect and hands it to codex's OAuth listener on the runtime that
/// started the sign-in, which checks the state and exchanges the code. Only the path and query
/// are forwarded.
async fn oauth_callback(State(state): State<AppState>, uri: axum::http::Uri) -> Response {
    // Inside the nested API router the URI has lost the `/api/v1` prefix, but codex's listener
    // matches the full path from the redirect URI, so the prefix is put back.
    let relative = uri.path_and_query().map(|value| value.as_str().to_string()).unwrap_or_default();
    if !relative.starts_with("/connectors/oauth/callback") || relative.bytes().any(|byte| byte <= 0x20 || byte >= 0x7f) {
        return (StatusCode::BAD_REQUEST, "not a sign-in callback").into_response();
    }
    let target = format!("/api/v1{relative}");
    let query = uri.query().unwrap_or_default();
    let oauth_state = query.split('&').find_map(|pair| pair.strip_prefix("state=")).unwrap_or_default();
    let destination: Option<(String, i32)> = sqlx::query_as(
        "DELETE FROM oauth_callbacks WHERE state = $1 RETURNING target_host, target_port",
    )
    .bind(oauth_state)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let Some((host, port)) = destination else {
        return (StatusCode::BAD_REQUEST, "This sign-in is unknown or has already finished. Start it again from Baize.").into_response();
    };
    match forward_to_listener(&host, port as u16, &target).await {
        Ok((status, body)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        Err(err) => {
            tracing::warn!(%err, "could not reach codex's OAuth listener");
            (StatusCode::BAD_GATEWAY, "The sign-in could not be completed. Start it again from Baize.").into_response()
        }
    }
}

/// A plain HTTP GET to codex's OAuth listener on the runtime host.
async fn forward_to_listener(host: &str, port: u16, target: &str) -> Result<(u16, Vec<u8>), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let address = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut stream = tokio::time::timeout(std::time::Duration::from_secs(5), tokio::net::TcpStream::connect(&address))
        .await
        .map_err(|_| "connect timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let request = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.map_err(|err| err.to_string())?;
    let mut raw = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(20), stream.read_to_end(&mut raw))
        .await
        .map_err(|_| "reading the response timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n").ok_or("malformed response")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("malformed status line")?;
    Ok((status, raw[split + 4..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::{endpoint_host, state_of};

    #[test]
    fn endpoint_host_reads_the_runtime_address() {
        assert_eq!(endpoint_host(None), "127.0.0.1");
        assert_eq!(endpoint_host(Some("ws://10.0.0.5:4500")), "10.0.0.5");
        assert_eq!(endpoint_host(Some("wss://runtime.example.com/path")), "runtime.example.com");
        assert_eq!(endpoint_host(Some("ws://[2001:db8::1]:4500")), "2001:db8::1");
        assert_eq!(endpoint_host(Some("ws://user@host.example:9/")), "host.example");
    }

    #[test]
    fn state_is_read_from_the_authorization_url() {
        let url = "https://idp.example/authorize?response_type=code&client_id=c&state=abc_DEF-1&redirect_uri=x";
        assert_eq!(state_of(url).as_deref(), Some("abc_DEF-1"));
        assert_eq!(state_of("https://idp.example/authorize?client_id=c"), None);
        assert_eq!(state_of("not a url"), None);
    }
}
