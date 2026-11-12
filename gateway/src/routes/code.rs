//! Linked GitHub repositories, and committing a code session's changes.

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
    code::{self, CodeError},
    error::ApiError,
    state::AppState,
    validate,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/repos", get(list).post(link))
        .route("/workspaces/{id}/repos/probe", post(probe_remote))
        .route("/repos/{id}", axum::routing::delete(unlink))
        .route("/threads/{id}/commit", post(commit))
}

#[derive(Serialize, FromRow)]
pub struct RepoView {
    pub id: Uuid,
    pub full_name: String,
    pub default_branch: String,
    pub has_token: bool,
    /// True for a repository added by its git address rather than through GitHub.
    pub by_url: bool,
    pub created_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

const REPO_COLUMNS: &str = "id, full_name, default_branch, (token_ciphertext IS NOT NULL) AS has_token, (clone_url IS NOT NULL) AS by_url, created_at, last_synced_at";

fn code_error(err: CodeError) -> ApiError {
    match err {
        CodeError::NothingToCommit | CodeError::NoCheckout => ApiError::Conflict(err.to_string()),
        CodeError::Git(message) | CodeError::Remote(message) => ApiError::BadRequest(message),
        CodeError::NoKey | CodeError::NotCodeThread => ApiError::BadRequest(err.to_string()),
        CodeError::Crypto | CodeError::Io(_) | CodeError::Db(_) => {
            tracing::error!(%err, "code workspace failure");
            ApiError::Internal
        }
    }
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<RepoView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!("SELECT {REPO_COLUMNS} FROM code_repos WHERE workspace_id = $1 ORDER BY full_name");
    let rows = sqlx::query_as::<_, RepoView>(&sql).bind(id).fetch_all(&state.pool).await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct LinkRepo {
    /// A GitHub repository as `owner/name`, reached through the person's connected account.
    full_name: Option<String>,
    /// Or any git remote, by its address (https), with the credentials it needs.
    url: Option<String>,
    username: Option<String>,
    token: Option<String>,
    default_branch: Option<String>,
}

#[derive(Deserialize)]
pub struct ProbeRequest {
    url: String,
    username: Option<String>,
    token: Option<String>,
}

#[derive(Serialize)]
pub struct ProbeResult {
    name: Option<String>,
    default_branch: Option<String>,
    branches: Vec<String>,
}

/// `owner/name`, using only the characters GitHub allows in those parts.
pub(super) fn valid_full_name(name: &str) -> bool {
    let Some((owner, repo)) = name.split_once('/') else {
        return false;
    };
    let part_ok = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    part_ok(owner) && part_ok(repo) && !repo.contains('/')
}

fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

async fn link(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<LinkRepo>,
) -> Result<(StatusCode, Json<RepoView>), ApiError> {
    // Any member can link a repository they can reach; the host decides what that is, per credential.
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let default_branch = req
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or("main")
        .to_string();
    if !valid_branch(&default_branch) {
        return Err(ApiError::BadRequest("the branch name is not valid".to_string()));
    }

    let url = req.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    let (full_name, clone_url, username, stored, auth) = match url {
        Some(url) => {
            // Any git remote: its own address and token, stored encrypted on the repository.
            code::validate_url(&state.code, &url).map_err(code_error)?;
            let full_name = code::name_from_url(&url)
                .ok_or_else(|| ApiError::BadRequest("the address does not name a repository".to_string()))?;
            let username = req
                .username
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .unwrap_or(code::TOKEN_USERNAME)
                .to_string();
            let secret = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
            let stored = match &secret {
                Some(secret) => Some(code::encrypt_token(&state.code, secret).map_err(code_error)?),
                None => None,
            };
            let auth = secret.map(|secret| code::Auth { username: username.clone(), secret });
            (full_name, Some(url), Some(username), stored, auth)
        }
        None => {
            // A GitHub repository: cloned with the linking person's connected account.
            let full_name = req.full_name.as_deref().map(str::trim).unwrap_or("").to_string();
            if !valid_full_name(&full_name) {
                return Err(ApiError::BadRequest("use the form owner/repository, or give a git address".to_string()));
            }
            let account: Option<(Vec<u8>, Vec<u8>)> =
                sqlx::query_as("SELECT token_ciphertext, token_nonce FROM github_accounts WHERE user_id = $1")
                    .bind(user.id)
                    .fetch_optional(&state.pool)
                    .await?;
            let auth = code::user_auth(&state.code, &state.pool, user.id).await.map_err(code_error)?;
            (full_name, None, None, account, auth)
        }
    };
    let (ciphertext, nonce) = match stored {
        Some((ciphertext, nonce)) => (Some(ciphertext), Some(nonce)),
        None => (None, None),
    };

    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM code_repos WHERE workspace_id = $1 AND full_name = $2)")
        .bind(id)
        .bind(&full_name)
        .fetch_one(&state.pool)
        .await?;
    if exists {
        return Err(ApiError::Conflict("this repository is already linked".to_string()));
    }

    let repo_id: Uuid = sqlx::query_scalar(
        "INSERT INTO code_repos (workspace_id, full_name, default_branch, token_ciphertext, token_nonce, created_by, clone_url, username)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(id)
    .bind(&full_name)
    .bind(&default_branch)
    .bind(ciphertext)
    .bind(nonce)
    .bind(user.id)
    .bind(&clone_url)
    .bind(&username)
    .fetch_one(&state.pool)
    .await?;

    // Clone now, so a wrong address or a bad token fails here rather than in a session.
    if let Err(err) = code::sync(&state.code, repo_id, &full_name, clone_url.as_deref(), auth.as_ref()).await {
        sqlx::query("DELETE FROM code_repos WHERE id = $1").bind(repo_id).execute(&state.pool).await?;
        let hint = if auth.is_none() && clone_url.is_none() { " (connect GitHub to use a private repository)" } else { "" };
        return Err(ApiError::BadRequest(format!("could not clone {full_name}: {err}{hint}")));
    }
    sqlx::query("UPDATE code_repos SET last_synced_at = now() WHERE id = $1")
        .bind(repo_id)
        .execute(&state.pool)
        .await?;
    audit::record(&state.pool, id, user.id, "repo.linked", "repo", repo_id, json!({ "full_name": full_name })).await?;

    let sql = format!("SELECT {REPO_COLUMNS} FROM code_repos WHERE id = $1");
    let view = sqlx::query_as::<_, RepoView>(&sql).bind(repo_id).fetch_one(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// Reads a git address's branches before anything is added. Nothing is stored; the token is used once.
async fn probe_remote(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ProbeRequest>,
) -> Result<Json<ProbeResult>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let url = req.url.trim().to_string();
    let auth = req
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|secret| code::Auth {
            username: req
                .username
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .unwrap_or(code::TOKEN_USERNAME)
                .to_string(),
            secret: secret.to_string(),
        });
    let found = code::probe(&state.code, &url, auth.as_ref()).await.map_err(code_error)?;
    Ok(Json(ProbeResult {
        name: code::name_from_url(&url),
        default_branch: found.default_branch,
        branches: found.branches,
    }))
}

async fn unlink(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM code_repos WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    require_role(&state.pool, workspace_id, user.id, Role::Admin).await?;
    // Threads keep their history; their repository link is cleared by the foreign key.
    sqlx::query("DELETE FROM code_repos WHERE id = $1").bind(id).execute(&state.pool).await?;
    audit::record(&state.pool, workspace_id, user.id, "repo.unlinked", "repo", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CommitRequest {
    message: String,
}

#[derive(Serialize)]
pub struct CommitResult {
    sha: String,
    branch: String,
}

async fn commit(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<CommitResult>, ApiError> {
    let (workspace_id, repo_id): (Uuid, Option<Uuid>) =
        sqlx::query_as("SELECT workspace_id, repo_id FROM threads WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    let repo_id = repo_id.ok_or_else(|| code_error(CodeError::NotCodeThread))?;
    let message = req.message.trim().to_string();
    validate::length("message", &message, 1, 500)?;

    let display_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    // The commit's author is the display name. The address is a placeholder, not the user's email.
    let author_name: String = display_name.chars().filter(|c| !matches!(c, '<' | '>' | '\n' | '\r')).take(100).collect();
    let author_email = format!("{}@users.noreply.baize", user.id);

    let author_auth = code::user_auth(&state.code, &state.pool, user.id).await.map_err(code_error)?;
    let (sha, branch) = code::commit_and_push(&state.code, &state.pool, repo_id, id, &message, &author_name, &author_email, author_auth)
        .await
        .map_err(code_error)?;
    audit::record(&state.pool, workspace_id, user.id, "thread.committed", "thread", id, json!({ "branch": branch, "sha": sha })).await?;
    Ok(Json(CommitResult { sha, branch }))
}
