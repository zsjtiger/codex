//! Files in a code chat's checkout: searching them for "@" mentions, and browsing and reading them.
//! Codex does the work on the environment the chat's turns run in. The gateway keeps every request inside
//! the checkout, so a path cannot reach outside it or into the checkout's git data.

use std::path::{Component, Path, PathBuf};

use axum::{
    Json, Router,
    extract::{Path as PathParam, Query, State},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    authz::{Role, require_role},
    auth::CurrentUser,
    error::ApiError,
    runtime::Lease,
    state::AppState,
};

/// Most text returned for one file. A longer file is cut here and marked as truncated.
const READ_LIMIT: usize = 256 * 1024;
/// Most matches returned for one search.
const SEARCH_LIMIT: usize = 20;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/threads/{id}/files", get(search_files))
        .route("/threads/{id}/files/list", get(list_directory))
        .route("/threads/{id}/files/read", get(read_file))
}

/// The chat's checkout on disk, and the environment its turns run in.
struct Checkout {
    root: PathBuf,
    environment_id: Uuid,
    endpoint: Option<String>,
}

/// The checkout of a code chat. None for a chat that is not a code chat, or whose checkout is not on disk yet.
async fn checkout_of(state: &AppState, thread_id: Uuid) -> Result<Option<Checkout>, ApiError> {
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<String>)>(
        "SELECT th.repo_id, th.environment_id, e.endpoint
           FROM threads th LEFT JOIN environments e ON e.id = th.environment_id
          WHERE th.id = $1",
    )
    .bind(thread_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((Some(_repo), Some(environment_id), endpoint)) = row else {
        return Ok(None);
    };
    let root = crate::code::worktree_path(&state.code, thread_id);
    if !root.join(".git").exists() {
        return Ok(None);
    }
    Ok(Some(Checkout { root, environment_id, endpoint }))
}

/// A path given relative to the checkout's root, resolved to a real path inside the checkout. Paths that
/// go up, are absolute, pass through the checkout's git data, or follow a link out of the checkout are refused.
fn inside(root: &Path, relative: &str) -> Result<PathBuf, ApiError> {
    let outside = || ApiError::BadRequest("that path is not in the checkout".to_string());
    let rel = Path::new(relative);
    for part in rel.components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) if name != ".git" => {}
            _ => return Err(outside()),
        }
    }
    let real_root = root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let real = root.join(rel).canonicalize().map_err(|_| ApiError::NotFound)?;
    if !real.starts_with(&real_root) {
        return Err(outside());
    }
    Ok(real)
}

/// The environment's codex, which runs the file operations. A chat whose environment cannot start codex gets an error.
async fn codex_for(state: &AppState, checkout: &Checkout) -> Result<Lease, ApiError> {
    state
        .runtime
        .lease(checkout.environment_id, checkout.endpoint.as_deref())
        .await
        .map_err(|err| {
            tracing::warn!(%err, "codex is not reachable for a checkout");
            ApiError::BadRequest("codex is not running for this chat's environment".to_string())
        })
}

async fn member_of(state: &AppState, thread_id: Uuid, user: &CurrentUser) -> Result<(), ApiError> {
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM threads WHERE id = $1")
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
}

#[derive(Serialize)]
pub struct FileMatch {
    /// Path relative to the checkout.
    path: String,
    file_name: String,
}

/// Files in a code chat's checkout whose paths match the query, for "@" mentions in the composer.
/// The search runs on the chat's environment. Anything it cannot search gives no matches.
async fn search_files(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<FileMatch>>, ApiError> {
    member_of(&state, id, &user).await?;
    let q = query.q.trim();
    if q.is_empty() || q.chars().count() > 200 {
        return Ok(Json(Vec::new()));
    }
    let Some(checkout) = checkout_of(&state, id).await? else {
        return Ok(Json(Vec::new()));
    };
    let lease = match codex_for(&state, &checkout).await {
        Ok(lease) => lease,
        Err(_) => return Ok(Json(Vec::new())),
    };
    let files = match lease
        .request("fuzzyFileSearch", json!({ "query": q, "roots": [checkout.root.to_string_lossy()] }))
        .await
    {
        Ok(response) => response
            .get("files")
            .and_then(|files| files.as_array())
            .into_iter()
            .flatten()
            .filter_map(|file| {
                Some(FileMatch {
                    path: file.get("path")?.as_str()?.to_string(),
                    file_name: file.get("fileName")?.as_str()?.to_string(),
                })
            })
            .take(SEARCH_LIMIT)
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "file search failed");
            Vec::new()
        }
    };
    Ok(Json(files))
}

#[derive(Deserialize)]
pub struct PathQuery {
    /// Relative to the checkout's root; empty for the root itself.
    #[serde(default)]
    path: String,
}

#[derive(Serialize)]
pub struct Entry {
    name: String,
    is_directory: bool,
}

/// The entries of a folder in the checkout: folders first, then files, each by name. The git data is hidden.
async fn list_directory(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<Vec<Entry>>, ApiError> {
    member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let folder = inside(&checkout.root, &query.path)?;
    if !folder.is_dir() {
        return Err(ApiError::BadRequest("that is not a folder".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    let response = lease
        .request("fs/readDirectory", json!({ "path": folder }))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let mut entries: Vec<Entry> = response
        .get("entries")
        .and_then(|entries| entries.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(Entry {
                name: entry.get("fileName")?.as_str()?.to_string(),
                is_directory: entry.get("isDirectory")?.as_bool()?,
            })
        })
        .filter(|entry| entry.name != ".git")
        .collect();
    entries.sort_by(|a, b| b.is_directory.cmp(&a.is_directory).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(entries))
}

#[derive(Serialize)]
pub struct FileView {
    path: String,
    /// The text of the file, cut to the first part when it is long. None for a binary file.
    content: Option<String>,
    binary: bool,
    truncated: bool,
    /// The file's full size in bytes.
    bytes: usize,
}

/// One file of the checkout, as text. Binary files are reported, not returned.
async fn read_file(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileView>, ApiError> {
    member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let file = inside(&checkout.root, &query.path)?;
    if file.is_dir() {
        return Err(ApiError::BadRequest("that is a folder".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    let response = lease
        .request("fs/readFile", json!({ "path": file }))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let data = response
        .get("dataBase64")
        .and_then(|data| data.as_str())
        .ok_or(ApiError::Internal)?;
    let bytes = STANDARD.decode(data).map_err(|_| ApiError::Internal)?;
    let size = bytes.len();
    let view = match std::str::from_utf8(&bytes) {
        Err(_) => FileView { path: query.path, content: None, binary: true, truncated: false, bytes: size },
        Ok(text) => {
            let truncated = text.len() > READ_LIMIT;
            let mut end = text.len().min(READ_LIMIT);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            FileView {
                path: query.path,
                content: Some(text[..end].to_string()),
                binary: false,
                truncated,
                bytes: size,
            }
        }
    };
    Ok(Json(view))
}
