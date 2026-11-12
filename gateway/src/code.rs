//! Code repositories: git clones, one git worktree and branch per code thread, and commit and push.
//!
//! A repository is either a GitHub repository (reached through a connected GitHub account) or any git
//! remote added by its address and an access token (GitLab, Gitea, a self-hosted server).
//!
//! Tokens are encrypted with AES-256-GCM under `GATEWAY_SECRET_KEY`. They reach git only through its
//! environment (an HTTP header), never a command line or a clone URL.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
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
    /// Base for GitHub clone URLs. `https://github.com` in production.
    pub git_base: String,
    /// Key for encrypting stored tokens. Without it, only tokenless (public) repos can be linked.
    pub key: Option<[u8; 32]>,
    pub github: crate::github::GitHubConfig,
    /// Allows `file://` remotes. Off in production: a local path would let anyone read files on the gateway.
    pub allow_file_remotes: bool,
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("code-data"),
            git_base: "https://github.com".to_string(),
            key: None,
            github: crate::github::GitHubConfig::default(),
            allow_file_remotes: false,
        }
    }
}

/// What git presents to a remote: an access token (HTTP basic authentication), or an SSH key with the
/// host key it must match.
#[derive(Debug, Clone)]
pub enum Auth {
    Token { username: String, secret: String },
    Ssh { private_key: String, known_hosts: String },
}

/// The username GitHub expects with an access token.
pub const GITHUB_USERNAME: &str = "x-access-token";
/// The username GitLab and most self-hosted servers accept with an access token.
pub const TOKEN_USERNAME: &str = "oauth2";

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("the gateway has no GATEWAY_SECRET_KEY, so a token cannot be stored")]
    NoKey,
    #[error("the stored token could not be read")]
    Crypto,
    #[error("{0}")]
    Git(String),
    #[error("{0}")]
    Remote(String),
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

/// The GitHub account a user connected, as git credentials. None when they have not connected one.
pub async fn user_auth(config: &CodeConfig, pool: &PgPool, user_id: Uuid) -> Result<Option<Auth>, CodeError> {
    let row: Option<(Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT token_ciphertext, token_nonce FROM github_accounts WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    match row {
        Some((ciphertext, nonce)) => {
            let key = config.key.as_ref().ok_or(CodeError::NoKey)?;
            Ok(Some(Auth::Token {
                username: GITHUB_USERNAME.to_string(),
                secret: open(key, &ciphertext, &nonce)?,
            }))
        }
        None => Ok(None),
    }
}

fn clone_dir(config: &CodeConfig, repo_id: Uuid) -> PathBuf {
    config.root.join("repos").join(repo_id.to_string())
}

fn worktree_dir(config: &CodeConfig, thread_id: Uuid) -> PathBuf {
    config.root.join("worktrees").join(thread_id.to_string())
}

/// One lock per code thread. Its checkout, commits and pushes run one at a time, so an automatic commit
/// and a manual one never race in the same worktree.
fn thread_lock(thread_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    map.entry(thread_id).or_default().clone()
}

/// The branch a code thread commits to. It is never the repository's default branch.
pub fn branch_for(thread_id: Uuid) -> String {
    format!("baize/{}", &thread_id.simple().to_string()[..8])
}

fn git_env(auth: Option<&Auth>) -> Vec<(&'static str, String)> {
    let mut env = vec![("GIT_TERMINAL_PROMPT", "0".to_string())];
    if let Some(Auth::Token { username, secret }) = auth {
        let basic = STANDARD.encode(format!("{username}:{secret}"));
        env.push(("GIT_CONFIG_COUNT", "1".to_string()));
        env.push(("GIT_CONFIG_KEY_0", "http.extraHeader".to_string()));
        env.push(("GIT_CONFIG_VALUE_0", format!("Authorization: Basic {basic}")));
    }
    env
}

/// The private key and the pinned host keys of one git call, as files that exist only while it runs.
struct KeyFiles {
    key: PathBuf,
    known_hosts: PathBuf,
}

impl KeyFiles {
    fn write(private_key: &str, known_hosts: &str) -> Result<Self, CodeError> {
        let dir = std::env::temp_dir().join("baize-keys");
        std::fs::create_dir_all(&dir).map_err(|err| CodeError::Io(err.to_string()))?;
        let id = Uuid::new_v4().simple().to_string();
        let files = Self { key: dir.join(&id), known_hosts: dir.join(format!("{id}.known_hosts")) };
        std::fs::write(&files.key, private_key).map_err(|err| CodeError::Io(err.to_string()))?;
        std::fs::write(&files.known_hosts, known_hosts).map_err(|err| CodeError::Io(err.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&files.key, std::fs::Permissions::from_mode(0o600))
                .map_err(|err| CodeError::Io(err.to_string()))?;
        }
        Ok(files)
    }

    /// Only the key this repository uses, and only the host keys pinned for it.
    fn ssh_command(&self) -> String {
        format!(
            "ssh -i '{}' -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile='{}'",
            self.key.display(),
            self.known_hosts.display()
        )
    }
}

impl Drop for KeyFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.key);
        let _ = std::fs::remove_file(&self.known_hosts);
    }
}

async fn git(dir: Option<&Path>, args: &[&str], auth: Option<&Auth>) -> Result<String, CodeError> {
    let mut command = tokio::process::Command::new("git");
    command.args(args).stdin(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    for (key, value) in git_env(auth) {
        command.env(key, value);
    }
    // Held until git has finished, then removed.
    let _key_files = match auth {
        Some(Auth::Ssh { private_key, known_hosts }) => {
            let files = KeyFiles::write(private_key, known_hosts)?;
            command.env("GIT_SSH_COMMAND", files.ssh_command());
            Some(files)
        }
        _ => None,
    };
    let output = command
        .output()
        .await
        .map_err(|err| CodeError::Git(format!("could not run git: {err}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if let Some(Auth::Token { secret, .. }) = auth {
        message = message.replace(secret.as_str(), "***");
    }
    Err(CodeError::Git(message))
}

struct RepoAccess {
    full_name: String,
    default_branch: String,
    /// The address of a repository added by URL. None for a GitHub repository.
    clone_url: Option<String>,
    /// The token stored on the repository itself, used when the person has none of their own.
    stored: Option<Auth>,
}

async fn repo_access(config: &CodeConfig, pool: &PgPool, repo_id: Uuid) -> Result<RepoAccess, CodeError> {
    let (full_name, default_branch, clone_url, username, ciphertext, nonce, key_ciphertext, key_nonce, known_hosts): (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT full_name, default_branch, clone_url, username, token_ciphertext, token_nonce,
                ssh_key_ciphertext, ssh_key_nonce, ssh_known_hosts
         FROM code_repos WHERE id = $1",
    )
    .bind(repo_id)
    .fetch_one(pool)
    .await?;
    let stored = match (key_ciphertext, key_nonce, ciphertext, nonce) {
        (Some(key_ciphertext), Some(key_nonce), _, _) => {
            let key = config.key.as_ref().ok_or(CodeError::NoKey)?;
            Some(Auth::Ssh {
                private_key: open(key, &key_ciphertext, &key_nonce)?,
                known_hosts: known_hosts.unwrap_or_default(),
            })
        }
        (_, _, Some(ciphertext), Some(nonce)) => {
            let key = config.key.as_ref().ok_or(CodeError::NoKey)?;
            Some(Auth::Token {
                username: username.unwrap_or_else(|| GITHUB_USERNAME.to_string()),
                secret: open(key, &ciphertext, &nonce)?,
            })
        }
        _ => None,
    };
    Ok(RepoAccess { full_name, default_branch, clone_url, stored })
}

/// The stored credentials of a repository: its SSH key, or its token. Used to check it when it is added.
pub async fn stored_auth(config: &CodeConfig, pool: &PgPool, repo_id: Uuid) -> Result<Option<Auth>, CodeError> {
    Ok(repo_access(config, pool, repo_id).await?.stored)
}

/// The address git clones from: the one given for a URL repository, or GitHub's for the rest.
fn remote_url(config: &CodeConfig, full_name: &str, clone_url: Option<&str>) -> String {
    match clone_url {
        Some(url) => url.to_string(),
        None => format!("{}/{full_name}.git", config.git_base.trim_end_matches('/')),
    }
}

/// Only what a remote needs: https, or file for tests and local development when enabled. No
/// credentials inside the address; the token goes in its own field.
pub fn validate_url(config: &CodeConfig, url: &str) -> Result<(), CodeError> {
    let url = url.trim();
    if url.len() > 500 || url.chars().any(char::is_whitespace) {
        return Err(CodeError::Remote("the address is not valid".to_string()));
    }
    if let Some(rest) = url.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') {
            return Err(CodeError::Remote("use an address without a user name in it; enter the token separately".to_string()));
        }
        return Ok(());
    }
    if is_ssh(url) {
        ssh_target(url).ok_or_else(|| CodeError::Remote("the SSH address is not valid".to_string()))?;
        return Ok(());
    }
    if url.starts_with("file://") && config.allow_file_remotes {
        return Ok(());
    }
    Err(CodeError::Remote("the address must start with https:// or ssh://, or be an SSH address like git@host:group/project.git".to_string()))
}

/// Whether an address uses SSH: `ssh://...`, or `user@host:path` (the scp form git also accepts).
pub fn is_ssh(url: &str) -> bool {
    let url = url.trim();
    if url.starts_with("ssh://") {
        return true;
    }
    !url.contains("://") && url.contains('@') && url.contains(':')
}

/// The host and port an SSH address connects to. The port is 22 unless the address gives one.
pub fn ssh_target(url: &str) -> Option<(String, u16)> {
    let url = url.trim();
    let (authority, port) = if let Some(rest) = url.strip_prefix("ssh://") {
        let authority = rest.split('/').next()?;
        let host_part = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        match host_part.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse::<u16>().ok()?),
            None => (host_part.to_string(), 22),
        }
    } else {
        let host_part = url.split_once('@')?.1.split_once(':')?.0;
        (host_part.to_string(), 22)
    };
    let valid = !authority.is_empty()
        && authority.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    valid.then_some((authority, port))
}

/// The host keys a server offers, in known_hosts form. Read without authentication, once, at creation.
pub async fn scan_host(url: &str) -> Result<String, CodeError> {
    let (host, port) = ssh_target(url).ok_or_else(|| CodeError::Remote("the SSH address is not valid".to_string()))?;
    let output = tokio::process::Command::new("ssh-keyscan")
        .args(["-T", "10", "-t", "ed25519,ecdsa,rsa", "-p", &port.to_string(), &host])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|err| CodeError::Git(format!("could not run ssh-keyscan: {err}")))?;
    let keys = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if keys.is_empty() {
        return Err(CodeError::Remote(format!("could not reach {host} on port {port} over SSH")));
    }
    Ok(keys)
}

/// The SHA256 fingerprints of pinned host keys, as `ssh-keygen` prints them. Shown for confirmation.
pub async fn host_fingerprints(known_hosts: &str) -> Result<String, CodeError> {
    let dir = std::env::temp_dir().join("baize-keys");
    std::fs::create_dir_all(&dir).map_err(|err| CodeError::Io(err.to_string()))?;
    let file = dir.join(format!("{}.scan", Uuid::new_v4().simple()));
    std::fs::write(&file, known_hosts).map_err(|err| CodeError::Io(err.to_string()))?;
    let output = tokio::process::Command::new("ssh-keygen")
        .arg("-lf")
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .await;
    let _ = std::fs::remove_file(&file);
    let output = output.map_err(|err| CodeError::Git(format!("could not run ssh-keygen: {err}")))?;
    if !output.status.success() {
        return Err(CodeError::Remote("the host keys could not be read".to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A new key for one repository, as a private key (to encrypt) and a public key (to show).
pub async fn generate_key() -> Result<(String, String), CodeError> {
    let dir = std::env::temp_dir().join("baize-keys");
    std::fs::create_dir_all(&dir).map_err(|err| CodeError::Io(err.to_string()))?;
    let base = dir.join(format!("{}.gen", Uuid::new_v4().simple()));
    let output = tokio::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "baize", "-f"])
        .arg(&base)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|err| CodeError::Git(format!("could not run ssh-keygen: {err}")))?;
    let public = base.with_extension("gen.pub");
    let result = if output.status.success() {
        let private = std::fs::read_to_string(&base).map_err(|err| CodeError::Io(err.to_string()));
        let public = std::fs::read_to_string(&public).map_err(|err| CodeError::Io(err.to_string()));
        private.and_then(|private| public.map(|public| (private, public.trim().to_string())))
    } else {
        Err(CodeError::Io("the key could not be made".to_string()))
    };
    let _ = std::fs::remove_file(&base);
    let _ = std::fs::remove_file(base.with_extension("gen.pub"));
    result
}

/// A display name from an address: its last two path segments, such as `group/project`.
pub fn name_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(_, path)| path)?,
        None => url.split_once(':')?.1,
    };
    let segments: Vec<&str> = path
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let tail = &segments[segments.len().saturating_sub(2)..];
    if tail.is_empty() {
        return None;
    }
    let name = tail.join("/");
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
        && !name.contains("..");
    ok.then_some(name)
}

/// What a remote offers: its default branch and its branches.
pub struct Probe {
    pub default_branch: Option<String>,
    pub branches: Vec<String>,
}

/// Reads a remote's branches without storing anything. The token is used only for this call.
pub async fn probe(config: &CodeConfig, url: &str, auth: Option<&Auth>) -> Result<Probe, CodeError> {
    validate_url(config, url)?;
    let output = git(None, &["ls-remote", "--symref", url.trim(), "HEAD", "refs/heads/*"], auth).await?;
    let mut default_branch = None;
    let mut branches = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
            if let Some((branch, _)) = rest.split_once('\t') {
                default_branch = Some(branch.to_string());
            }
        } else if let Some((_, reference)) = line.split_once('\t') {
            if let Some(branch) = reference.strip_prefix("refs/heads/") {
                branches.push(branch.to_string());
            }
        }
    }
    Ok(Probe { default_branch, branches })
}

/// Clones the repository, or fetches it if it is already here.
pub async fn sync(
    config: &CodeConfig,
    repo_id: Uuid,
    full_name: &str,
    clone_url: Option<&str>,
    auth: Option<&Auth>,
) -> Result<(), CodeError> {
    let dir = clone_dir(config, repo_id);
    if dir.join(".git").exists() {
        git(Some(&dir), &["fetch", "--prune", "origin"], auth).await?;
        return Ok(());
    }
    std::fs::create_dir_all(dir.parent().expect("clone dir has a parent"))
        .map_err(|err| CodeError::Io(err.to_string()))?;
    let url = remote_url(config, full_name, clone_url);
    let dir_text = dir.to_string_lossy().to_string();
    if let Err(err) = git(None, &["clone", "--quiet", &url, &dir_text], auth).await {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    Ok(())
}

/// The credentials for one repository. A GitHub repository uses the person's GitHub account; a URL
/// repository uses the token it was added with, since the person's GitHub account is not its host's.
fn choose_auth(repo: &RepoAccess, user_auth: Option<Auth>) -> Option<Auth> {
    match &repo.clone_url {
        None => user_auth.or_else(|| repo.stored.clone()),
        Some(_) => repo.stored.clone(),
    }
}

/// Returns the thread's checkout, creating its branch and worktree on first use.
pub async fn checkout(
    config: &CodeConfig,
    pool: &PgPool,
    repo_id: Uuid,
    thread_id: Uuid,
    user_auth: Option<Auth>,
) -> Result<PathBuf, CodeError> {
    let lock = thread_lock(thread_id);
    let _guard = lock.lock().await;
    let worktree = worktree_dir(config, thread_id);
    if worktree.join(".git").exists() {
        return Ok(worktree);
    }
    let repo = repo_access(config, pool, repo_id).await?;
    let auth = choose_auth(&repo, user_auth);
    sync(config, repo_id, &repo.full_name, repo.clone_url.as_deref(), auth.as_ref()).await?;
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
    user_auth: Option<Auth>,
) -> Result<(String, String), CodeError> {
    let lock = thread_lock(thread_id);
    let _guard = lock.lock().await;
    let worktree = worktree_dir(config, thread_id);
    if !worktree.join(".git").exists() {
        return Err(CodeError::NoCheckout);
    }
    let repo = repo_access(config, pool, repo_id).await?;
    let auth = choose_auth(&repo, user_auth);
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
    git(Some(&worktree), &["push", "-u", "origin", &branch], auth.as_ref()).await?;
    Ok((sha, branch))
}

/// The commit message for a turn: the request's first line, then a note that Baize made it.
pub fn auto_message(request: &str) -> String {
    let subject: String = request.lines().next().unwrap_or("").trim().chars().take(72).collect();
    let subject = if subject.is_empty() { "Update from a Baize code session".to_string() } else { subject };
    format!("{subject}\n\nMade in a Baize code session.")
}

/// The name and placeholder address that commits by a user carry.
pub async fn author(pool: &PgPool, user_id: Uuid) -> Result<(String, String), CodeError> {
    let display_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    let name: String = display_name.chars().filter(|c| !matches!(c, '<' | '>' | '\n' | '\r')).take(100).collect();
    Ok((name, format!("{user_id}@users.noreply.baize")))
}
