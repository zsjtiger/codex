//! Connecting a GitHub account, and the repositories and branches it can reach.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    code,
    error::ApiError,
    github::{self, GitHubError, Repo},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/github", axum::routing::delete(disconnect))
        .route("/github/connect", post(connect))
        .route("/github/callback", get(callback))
        .route("/github/status", get(status))
        .route("/github/repos", get(repos))
        .route("/github/repos/{owner}/{name}/branches", get(branches))
}

fn github_error(err: GitHubError) -> ApiError {
    match err {
        GitHubError::NotConfigured | GitHubError::Rejected(_) => ApiError::BadRequest(err.to_string()),
        GitHubError::Network(_) => {
            tracing::warn!(%err, "GitHub request failed");
            ApiError::BadRequest("GitHub could not be reached; try again".to_string())
        }
    }
}

/// Where GitHub sends the browser back to, on this gateway.
fn callback_url() -> Result<String, ApiError> {
    let public = super::connectors::public_base()
        .ok_or_else(|| ApiError::BadRequest("GATEWAY_PUBLIC_URL must be set to connect GitHub".to_string()))?;
    Ok(format!("{public}/api/v1/github/callback"))
}

/// Where the gateway sends the browser when the flow ends: the web app.
fn web_base() -> String {
    std::env::var("GATEWAY_WEB_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .or_else(super::connectors::public_base)
        .unwrap_or_default()
}

#[derive(Serialize)]
pub struct ConnectResponse {
    url: String,
}

async fn connect(State(state): State<AppState>, user: CurrentUser) -> Result<Json<ConnectResponse>, ApiError> {
    let redirect = callback_url()?;
    let value = Uuid::new_v4().simple().to_string();
    // A state is good for an hour, and only for the person who asked for it.
    sqlx::query("DELETE FROM github_oauth_states WHERE created_at < now() - interval '1 hour'")
        .execute(&state.pool)
        .await?;
    sqlx::query("INSERT INTO github_oauth_states (state, user_id) VALUES ($1, $2)")
        .bind(&value)
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    let url = github::authorize_url(&state.code.github, &redirect, &value).map_err(github_error)?;
    Ok(Json(ConnectResponse { url }))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GitHub's redirect after approval. No session is needed: the state identifies the person.
async fn callback(State(state): State<AppState>, Query(query): Query<CallbackQuery>) -> Redirect {
    let web = web_base();
    let back = |outcome: &str| Redirect::to(&format!("{web}/?github={outcome}"));
    if query.error.is_some() {
        return back("denied");
    }
    let (Some(code), Some(value)) = (query.code, query.state) else {
        return back("error");
    };
    let owner: Option<Uuid> = match sqlx::query_scalar(
        "DELETE FROM github_oauth_states WHERE state = $1 AND created_at > now() - interval '1 hour' RETURNING user_id",
    )
    .bind(&value)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(owner) => owner,
        Err(_) => return back("error"),
    };
    let Some(user_id) = owner else {
        return back("expired");
    };
    let Ok(redirect) = callback_url() else {
        return back("error");
    };
    let Ok(token) = github::exchange_code(&state.code.github, &code, &redirect).await else {
        return back("error");
    };
    let Ok(login) = github::login(&state.code.github, &token).await else {
        return back("error");
    };
    let Ok((ciphertext, nonce)) = code::encrypt_token(&state.code, &token) else {
        return back("error");
    };
    let saved = sqlx::query(
        "INSERT INTO github_accounts (user_id, login, token_ciphertext, token_nonce) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE SET login = EXCLUDED.login, token_ciphertext = EXCLUDED.token_ciphertext,
           token_nonce = EXCLUDED.token_nonce, connected_at = now()",
    )
    .bind(user_id)
    .bind(&login)
    .bind(ciphertext)
    .bind(nonce)
    .execute(&state.pool)
    .await;
    match saved {
        Ok(_) => back("connected"),
        Err(_) => back("error"),
    }
}

async fn status(State(state): State<AppState>, user: CurrentUser) -> Result<Json<serde_json::Value>, ApiError> {
    let login: Option<String> = sqlx::query_scalar("SELECT login FROM github_accounts WHERE user_id = $1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(Json(json!({ "connected": login.is_some(), "login": login })))
}

async fn disconnect(State(state): State<AppState>, user: CurrentUser) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM github_accounts WHERE user_id = $1")
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The token of the person asking, or a conflict telling them to connect first.
async fn own_token(state: &AppState, user: &CurrentUser) -> Result<String, ApiError> {
    code::user_token(&state.code, &state.pool, user.id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or_else(|| ApiError::Conflict("connect a GitHub account first".to_string()))
}

#[derive(Deserialize)]
pub struct RepoQuery {
    q: Option<String>,
}

async fn repos(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<RepoQuery>,
) -> Result<Json<Vec<Repo>>, ApiError> {
    let token = own_token(&state, &user).await?;
    let mut list = github::list_repos(&state.code.github, &token).await.map_err(github_error)?;
    if let Some(q) = query.q.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty()) {
        list.retain(|repo| repo.full_name.to_lowercase().contains(&q));
    }
    Ok(Json(list))
}

async fn branches(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<Vec<String>>, ApiError> {
    let full_name = format!("{owner}/{name}");
    if !super::code::valid_full_name(&full_name) {
        return Err(ApiError::BadRequest("use the form owner/repository".to_string()));
    }
    let token = own_token(&state, &user).await?;
    let list = github::list_branches(&state.code.github, &token, &full_name).await.map_err(github_error)?;
    Ok(Json(list))
}
