//! GitHub: OAuth sign-in for a user, and the repositories and branches their account can reach.
//!
//! The base addresses come from configuration, so tests can point them at a stand-in server.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    /// OAuth App client id and secret, from `GATEWAY_GITHUB_CLIENT_ID` and `..._SECRET`.
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    /// `https://github.com/login/oauth` in production.
    pub oauth_base: String,
    /// `https://api.github.com` in production.
    pub api_base: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            oauth_base: "https://github.com/login/oauth".to_string(),
            api_base: "https://api.github.com".to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("GitHub sign-in is not configured on this gateway")]
    NotConfigured,
    #[error("GitHub did not accept the request: {0}")]
    Rejected(String),
    #[error("could not reach GitHub: {0}")]
    Network(String),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Repo {
    pub full_name: String,
    pub default_branch: String,
    pub private: bool,
}

/// Most pages to read when listing; 100 items per page.
const MAX_PAGES: u32 = 5;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("baize-gateway")
        .build()
        .expect("the HTTP client builds with default settings")
}

fn credentials(config: &GitHubConfig) -> Result<(&str, &str), GitHubError> {
    match (config.client_id.as_deref(), config.client_secret.as_deref()) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err(GitHubError::NotConfigured),
    }
}

/// The address the browser is sent to in order to approve access.
pub fn authorize_url(config: &GitHubConfig, redirect_uri: &str, state: &str) -> Result<String, GitHubError> {
    let (client_id, _) = credentials(config)?;
    let url = reqwest::Url::parse_with_params(
        &format!("{}/authorize", config.oauth_base.trim_end_matches('/')),
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", "repo read:user"),
            ("state", state),
        ],
    )
    .map_err(|err| GitHubError::Rejected(err.to_string()))?;
    Ok(url.to_string())
}

/// Trades the code from the callback for an access token.
pub async fn exchange_code(config: &GitHubConfig, code: &str, redirect_uri: &str) -> Result<String, GitHubError> {
    let (client_id, client_secret) = credentials(config)?;
    let response = client()
        .post(format!("{}/access_token", config.oauth_base.trim_end_matches('/')))
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|err| GitHubError::Network(err.to_string()))?;
    let body: Value = response.json().await.map_err(|err| GitHubError::Network(err.to_string()))?;
    match body.get("access_token").and_then(Value::as_str) {
        Some(token) if !token.is_empty() => Ok(token.to_string()),
        _ => Err(GitHubError::Rejected(
            body.get("error_description")
                .or_else(|| body.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("no access token was returned")
                .to_string(),
        )),
    }
}

async fn get_json(config: &GitHubConfig, token: &str, path: &str) -> Result<Value, GitHubError> {
    let url = format!("{}{path}", config.api_base.trim_end_matches('/'));
    let response = client()
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|err| GitHubError::Network(err.to_string()))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let message = body.get("message").and_then(Value::as_str).unwrap_or("access was refused");
        return Err(GitHubError::Rejected(message.to_string()));
    }
    Err(GitHubError::Network(format!("GitHub answered {status}")))
}

/// The login of the account a token belongs to.
pub async fn login(config: &GitHubConfig, token: &str) -> Result<String, GitHubError> {
    let body = get_json(config, token, "/user").await?;
    body.get("login")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| GitHubError::Network("GitHub returned no login".to_string()))
}

/// Repositories the token can reach, most recently updated first.
pub async fn list_repos(config: &GitHubConfig, token: &str) -> Result<Vec<Repo>, GitHubError> {
    let mut repos = Vec::new();
    for page in 1..=MAX_PAGES {
        let body = get_json(
            config,
            token,
            &format!("/user/repos?per_page=100&page={page}&sort=updated&affiliation=owner,collaborator,organization_member"),
        )
        .await?;
        let items = body.as_array().cloned().unwrap_or_default();
        let count = items.len();
        for item in items {
            let Some(full_name) = item.get("full_name").and_then(Value::as_str) else {
                continue;
            };
            repos.push(Repo {
                full_name: full_name.to_string(),
                default_branch: item.get("default_branch").and_then(Value::as_str).unwrap_or("main").to_string(),
                private: item.get("private").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        if count < 100 {
            break;
        }
    }
    Ok(repos)
}

/// Branch names of one repository. `full_name` must already be validated as `owner/name`.
pub async fn list_branches(config: &GitHubConfig, token: &str, full_name: &str) -> Result<Vec<String>, GitHubError> {
    let mut branches = Vec::new();
    for page in 1..=MAX_PAGES {
        let body = get_json(config, token, &format!("/repos/{full_name}/branches?per_page=100&page={page}")).await?;
        let items = body.as_array().cloned().unwrap_or_default();
        let count = items.len();
        branches.extend(items.iter().filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string)));
        if count < 100 {
            break;
        }
    }
    Ok(branches)
}
