//! Code repositories: GitHub clones, one git worktree and branch per code thread, and commit and push.
//!
//! Tokens are encrypted with AES-256-GCM under `GATEWAY_SECRET_KEY`. They reach git only through its
//! environment (an HTTP header), never a command line or a clone URL.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CodeConfig {
    /// Holds `repos/<repo id>` clones and `worktrees/<thread id>` checkouts.
    pub root: PathBuf,
    /// Base for clone URLs. `https://github.com` in production.
    pub git_base: String,
    /// Key for encrypting stored tokens. Without it, only tokenless (public) repos can be linked.
    pub key: Option<[u8; 32]>,
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("code-data"),
            git_base: "https://github.com".to_string(),
            key: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("the gateway has no GATEWAY_SECRET_KEY, so a token cannot be stored")]
    NoKey,
    #[error("the stored token could not be read")]
    Crypto,
    #[error("{0}")]
    Git(String),
    #[error("nothing to commit: this session has no changes")]
    NothingToCommit,
    #[error("this session has no checkout yet; send a message first")]
    NoCheckout,
    #[error("this chat is not connected to a repository")]
    NotCodeThread,
    #[error("the file system could not be used: {0}")]
    Io(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Parses a 64-character hex key.
pub fn parse_key(hex_key: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_key.trim()).map_err(|_| "GATEWAY_SECRET_KEY must be hex".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "GATEWAY_SECRET_KEY must be 32 bytes (64 hex characters)".to_string())
}

fn seal(key: &[u8; 32], token: &str) -> Result<(Vec<u8>, Vec<u8>), CodeError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CodeError::Crypto)?;
    // The first 12 bytes of a random v4 UUID are unpredictable, which is enough for a fresh nonce.
    let nonce_bytes = Uuid::new_v4().as_bytes()[..12].to_vec();
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), token.as_bytes())
        .map_err(|_| CodeError::Crypto)?;
    Ok((ciphertext, nonce_bytes))
}

fn open(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8]) -> Result<String, CodeError> {
    if nonce.len() != 12 {
        return Err(CodeError::Crypto);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CodeError::Crypto)?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| CodeError::Crypto)?;
    String::from_utf8(plain).map_err(|_| CodeError::Crypto)
}

/// Encrypts a token for storage. Fails when the gateway has no key.
pub fn encrypt_token(config: &CodeConfig, token: &str) -> Result<(Vec<u8>, Vec<u8>), CodeError> {
    let key = config.key.as_ref().ok_or(CodeError::NoKey)?;
    seal(key, token)
}

fn clone_dir(config: &CodeConfig, repo_id: Uuid) -> PathBuf {
    config.root.join("repos").join(repo_id.to_string())
}

fn worktree_dir(config: &CodeConfig, thread_id: Uuid) -> PathBuf {
    config.root.join("worktrees").join(thread_id.to_string())
}

/// The branch a code thread commits to. It is never the repository's default branch.
pub fn branch_for(thread_id: Uuid) -> String {
    format!("baize/{}", &thread_id.simple().to_string()[..8])
}

fn git_env(token: Option<&str>) -> Vec<(&'static str, String)> {
    let mut env = vec![("GIT_TERMINAL_PROMPT", "0".to_string())];
    if let Some(token) = token {
        let basic = STANDARD.encode(format!("x-access-token:{token}"));
        env.push(("GIT_CONFIG_COUNT", "1".to_string()));
        env.push(("GIT_CONFIG_KEY_0", "http.extraHeader".to_string()));
        env.push(("GIT_CONFIG_VALUE_0", format!("Authorization: Basic {basic}")));
    }
    env
}

async fn git(dir: Option<&Path>, args: &[&str], token: Option<&str>) -> Result<String, CodeError> {
    let mut command = tokio::process::Command::new("git");
    command.args(args).stdin(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    for (key, value) in git_env(token) {
        command.env(key, value);
    }
    let output = command
        .output()
        .await
        .map_err(|err| CodeError::Git(format!("could not run git: {err}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if let Some(token) = token {
        message = message.replace(token, "***");
    }
    Err(CodeError::Git(message))
}

struct RepoAccess {
    full_name: String,
    default_branch: String,
    token: Option<String>,
}

async fn repo_access(config: &CodeConfig, pool: &PgPool, repo_id: Uuid) -> Result<RepoAccess, CodeError> {
    let (full_name, default_branch, ciphertext, nonce): (String, String, Option<Vec<u8>>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT full_name, default_branch, token_ciphertext, token_nonce FROM code_repos WHERE id = $1")
            .bind(repo_id)
            .fetch_one(pool)
            .await?;
    let token = match (ciphertext, nonce) {
        (Some(ciphertext), Some(nonce)) => {
            let key = config.key.as_ref().ok_or(CodeError::NoKey)?;
            Some(open(key, &ciphertext, &nonce)?)
        }
        _ => None,
    };
    Ok(RepoAccess { full_name, default_branch, token })
}

/// Clones the repository, or fetches it if it is already here.
pub async fn sync(config: &CodeConfig, repo_id: Uuid, full_name: &str, token: Option<&str>) -> Result<(), CodeError> {
    let dir = clone_dir(config, repo_id);
    if dir.join(".git").exists() {
        git(Some(&dir), &["fetch", "--prune", "origin"], token).await?;
        return Ok(());
    }
    std::fs::create_dir_all(dir.parent().expect("clone dir has a parent"))
        .map_err(|err| CodeError::Io(err.to_string()))?;
    let url = format!("{}/{full_name}.git", config.git_base.trim_end_matches('/'));
    let dir_text = dir.to_string_lossy().to_string();
    if let Err(err) = git(None, &["clone", "--quiet", &url, &dir_text], token).await {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    Ok(())
}

/// Returns the thread's checkout, creating its branch and worktree on first use.
pub async fn checkout(config: &CodeConfig, pool: &PgPool, repo_id: Uuid, thread_id: Uuid) -> Result<PathBuf, CodeError> {
    let worktree = worktree_dir(config, thread_id);
    if worktree.join(".git").exists() {
        return Ok(worktree);
    }
    let repo = repo_access(config, pool, repo_id).await?;
    let token = repo.token.as_deref();
    sync(config, repo_id, &repo.full_name, token).await?;
    let clone = clone_dir(config, repo_id);
    std::fs::create_dir_all(config.root.join("worktrees")).map_err(|err| CodeError::Io(err.to_string()))?;
    let branch = branch_for(thread_id);
    let base = format!("origin/{}", repo.default_branch);
    let worktree_text = worktree.to_string_lossy().to_string();
    git(Some(&clone), &["worktree", "add", "-b", &branch, &worktree_text, &base], None).await?;
    Ok(worktree)
}

/// Commits everything the session changed and pushes its branch. Returns the commit and branch.
pub async fn commit_and_push(
    config: &CodeConfig,
    pool: &PgPool,
    repo_id: Uuid,
    thread_id: Uuid,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<(String, String), CodeError> {
    let worktree = worktree_dir(config, thread_id);
    if !worktree.join(".git").exists() {
        return Err(CodeError::NoCheckout);
    }
    let repo = repo_access(config, pool, repo_id).await?;
    let token = repo.token.as_deref();
    let branch = branch_for(thread_id);

    git(Some(&worktree), &["add", "-A"], None).await?;
    if git(Some(&worktree), &["status", "--porcelain"], None).await?.is_empty() {
        return Err(CodeError::NothingToCommit);
    }
    let name_arg = format!("user.name={author_name}");
    let email_arg = format!("user.email={author_email}");
    git(
        Some(&worktree),
        &["-c", &name_arg, "-c", &email_arg, "commit", "--quiet", "-m", message],
        None,
    )
    .await?;
    let sha = git(Some(&worktree), &["rev-parse", "HEAD"], None).await?;
    git(Some(&worktree), &["push", "-u", "origin", &branch], token).await?;
    Ok((sha, branch))
}
