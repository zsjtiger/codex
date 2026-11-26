//! Files in a code chat's checkout: searching them for "@" mentions, and browsing and reading them.
//! Codex does the work on the environment the chat's turns run in. The gateway keeps every request inside
//! the checkout, so a path cannot reach outside it or into the checkout's git data.

use std::path::{Component, Path, PathBuf};

use axum::{
    Json, Router,
    http::StatusCode,
    extract::{Path as PathParam, Query, State},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
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
        .route("/threads/{id}/files", get(search_files).delete(delete_path))
        .route("/threads/{id}/files/content", put(write_file))
        .route("/threads/{id}/files/folder", post(create_folder))
        .route("/threads/{id}/files/move", post(move_path))
        .route("/threads/{id}/files/list", get(list_directory))
        .route("/threads/{id}/files/read", get(read_file))
        .route("/threads/{id}/run", post(run_command))
        .route("/threads/{id}/run/{process_id}/stop", post(stop_command))
        .route("/threads/{id}/workspace-diff", get(workspace_diff))
        .route("/threads/{id}/files/watch", post(start_watch))
        .route("/threads/{id}/files/watch/{watch_id}", axum::routing::delete(stop_watch))
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

/// Like `inside`, for a path that may not exist yet: its folder must be inside the checkout. A path that
/// exists is checked exactly as `inside` checks it, so a link out of the checkout is still refused.
fn inside_new(root: &Path, relative: &str) -> Result<PathBuf, ApiError> {
    let outside = || ApiError::BadRequest("that path is not in the checkout".to_string());
    let rel = Path::new(relative);
    for part in rel.components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) if name != ".git" => {}
            _ => return Err(outside()),
        }
    }
    let name = rel
        .file_name()
        .ok_or_else(|| ApiError::BadRequest("name a file or folder inside the checkout".to_string()))?;
    let real_root = root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let folder = root.join(rel).parent().unwrap_or(root).to_path_buf();
    let real_folder = folder
        .canonicalize()
        .map_err(|_| ApiError::BadRequest("the folder that holds it does not exist".to_string()))?;
    if !real_folder.starts_with(&real_root) {
        return Err(outside());
    }
    let target = real_folder.join(name);
    if target.symlink_metadata().is_ok() {
        return inside(root, relative);
    }
    Ok(target)
}

/// Most text a file write may carry.
const WRITE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct WriteRequest {
    path: String,
    content: String,
}

/// Writes a text file in a code chat's checkout, creating it if needed. Members can do this, as they can run
/// commands; each write is recorded in the audit log.
async fn write_file(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Json(req): Json<WriteRequest>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = member_of(&state, id, &user).await?;
    if req.content.len() > WRITE_LIMIT {
        return Err(ApiError::BadRequest("a file is at most 2 MB".to_string()));
    }
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let target = inside_new(&checkout.root, &req.path)?;
    if target.is_dir() {
        return Err(ApiError::BadRequest("that is a folder".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("fs/writeFile", json!({ "path": target, "dataBase64": STANDARD.encode(req.content.as_bytes()) }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not write the file: {err}")))?;
    audit::record(&state.pool, workspace_id, user.id, "checkout.file_write", "thread", id, json!({ "path": req.path, "bytes": req.content.len() })).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PathBody {
    path: String,
}

/// Creates a folder in a code chat's checkout. Its parent folder must exist.
async fn create_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Json(req): Json<PathBody>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let target = inside_new(&checkout.root, &req.path)?;
    if target.symlink_metadata().is_ok() {
        return Err(ApiError::Conflict("something is already there".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("fs/createDirectory", json!({ "path": target, "recursive": false }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not create the folder: {err}")))?;
    audit::record(&state.pool, workspace_id, user.id, "checkout.folder_create", "thread", id, json!({ "path": req.path })).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct MoveBody {
    from: String,
    to: String,
}

/// Moves or renames a file or folder in a code chat's checkout. Codex has no rename, so this copies and then
/// removes the original; a failed copy removes nothing.
async fn move_path(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Json(req): Json<MoveBody>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let root = checkout.root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let source = inside(&checkout.root, &req.from)?;
    if source == root {
        return Err(ApiError::BadRequest("the checkout itself cannot be moved".to_string()));
    }
    let destination = inside_new(&checkout.root, &req.to)?;
    if destination.starts_with(&source) {
        return Err(ApiError::BadRequest("a folder cannot be moved into itself".to_string()));
    }
    if destination.symlink_metadata().is_ok() {
        return Err(ApiError::Conflict("something is already there".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("fs/copy", json!({ "sourcePath": source, "destinationPath": destination, "recursive": true }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not copy it: {err}")))?;
    lease
        .request("fs/remove", json!({ "path": source, "recursive": true, "force": false }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("copied, but could not remove the original: {err}")))?;
    audit::record(&state.pool, workspace_id, user.id, "checkout.file_move", "thread", id, json!({ "from": req.from, "to": req.to })).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    path: String,
}

/// Deletes a file or folder in a code chat's checkout, with everything in it. The checkout itself cannot be deleted.
async fn delete_path(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Query(query): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let root = checkout.root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let target = inside(&checkout.root, &query.path)?;
    if target == root {
        return Err(ApiError::BadRequest("the checkout itself cannot be deleted".to_string()));
    }
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("fs/remove", json!({ "path": target, "recursive": true, "force": false }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not delete it: {err}")))?;
    audit::record(&state.pool, workspace_id, user.id, "checkout.file_delete", "thread", id, json!({ "path": query.path })).await?;
    Ok(StatusCode::NO_CONTENT)
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

/// Checks the person is a member of the chat's workspace, and returns that workspace's id.
async fn member_of(state: &AppState, thread_id: Uuid, user: &CurrentUser) -> Result<Uuid, ApiError> {
    let workspace_id: Uuid = sqlx::query_scalar("SELECT workspace_id FROM threads WHERE id = $1")
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    Ok(workspace_id)
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

/// Most time one command may run, unless the request asks for less.
const RUN_MAX_TIMEOUT_MS: i64 = 600_000;
const RUN_DEFAULT_TIMEOUT_MS: i64 = 120_000;
/// Most output kept for each of stdout and stderr.
const RUN_OUTPUT_CAP: usize = crate::runtime::hub::RUN_OUTPUT_LIMIT;
const RUN_MAX_COMMAND: usize = 2_000;

#[derive(Deserialize)]
pub struct RunRequest {
    /// A shell command line, run in the checkout's root with `sh -c`.
    command: String,
    timeout_ms: Option<i64>,
    /// An id for this run, chosen by the browser so it can stop the run while it goes. A UUID; made up if absent.
    process_id: Option<String>,
}

#[derive(Serialize)]
pub struct RunResult {
    exit_code: i64,
    stdout: String,
    stderr: String,
}

/// Runs one command in a code chat's checkout and returns what it printed. The command runs under Codex's
/// sandbox with writes limited to the checkout and no network access. Each run is recorded in the audit log.
async fn run_command(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
    Json(req): Json<RunRequest>,
) -> Result<Json<RunResult>, ApiError> {
    let workspace_id = member_of(&state, id, &user).await?;
    let command = req.command.trim();
    if command.is_empty() {
        return Err(ApiError::BadRequest("the command is empty".to_string()));
    }
    if command.chars().count() > RUN_MAX_COMMAND || command.contains('\0') {
        return Err(ApiError::BadRequest(format!("a command is at most {RUN_MAX_COMMAND} characters")));
    }
    let timeout = req.timeout_ms.unwrap_or(RUN_DEFAULT_TIMEOUT_MS).clamp(1_000, RUN_MAX_TIMEOUT_MS);
    let process_id = match req.process_id {
        Some(given) => {
            Uuid::parse_str(&given).map_err(|_| ApiError::BadRequest("the process id must be a UUID".to_string()))?;
            given
        }
        None => Uuid::new_v4().to_string(),
    };
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let root = checkout.root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let lease = codex_for(&state, &checkout).await?;
    let hub = state.runtime.hub();
    if !hub.start_process(&process_id, id) {
        return Err(ApiError::Conflict("a command with that id is already running".to_string()));
    }
    // Output streams to the chat as codex sends it; the reply carries what was kept, so nothing is lost.
    let outcome = lease
        .request(
            "command/exec",
            json!({
                "command": ["/bin/sh", "-c", command],
                "cwd": root,
                "processId": process_id,
                "streamStdoutStderr": true,
                "timeoutMs": timeout,
                "outputBytesCap": RUN_OUTPUT_CAP,
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [root],
                    "networkAccess": false,
                    "excludeTmpdirEnvVar": false,
                    "excludeSlashTmp": false,
                },
            }),
        )
        .await;
    let (stdout, stderr) = hub.finish_process(&process_id).unwrap_or_default();
    let response = outcome.map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let result = RunResult {
        exit_code: response.get("exitCode").and_then(|code| code.as_i64()).unwrap_or(-1),
        stdout,
        stderr,
    };
    audit::record(
        &state.pool,
        workspace_id,
        user.id,
        "checkout.command_run",
        "thread",
        id,
        json!({ "command": command, "exit_code": result.exit_code }),
    )
    .await?;
    Ok(Json(result))
}

/// Stops a command run from a code chat while it goes. Its run then returns with the output it had.
async fn stop_command(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam((id, process_id)): PathParam<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    member_of(&state, id, &user).await?;
    if state.runtime.hub().process_thread(&process_id) != Some(id) {
        return Err(ApiError::NotFound);
    }
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("command/exec/terminate", json!({ "processId": process_id }))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct WorkspaceDiff {
    /// The commit the checkout is compared with: the remote's copy of its branch.
    sha: String,
    /// Everything that differs from that commit, committed or not, as a unified diff.
    diff: String,
}

/// The changes in a code chat's checkout against the remote's copy of its branch, as codex computes them.
async fn workspace_diff(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<WorkspaceDiff>, ApiError> {
    member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let root = checkout.root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let lease = codex_for(&state, &checkout).await?;
    let response = lease
        .request("gitDiffToRemote", json!({ "cwd": root }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the changes: {err}")))?;
    // codex answers null when the checkout is not a git repository it can compare.
    let (Some(sha), Some(diff)) = (response["sha"].as_str(), response["diff"].as_str()) else {
        return Err(ApiError::NotFound);
    };
    Ok(Json(WorkspaceDiff { sha: sha.to_string(), diff: diff.to_string() }))
}

#[derive(Serialize)]
pub struct WatchStarted {
    /// The id the browser matches file change notices against.
    watch_id: String,
}

/// Starts watching a code chat's checkout for changes. Each change is sent to the chat as a `fs/changed`
/// event naming this watch. The watch lasts until it is stopped or the codex is restarted.
async fn start_watch(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam(id): PathParam<Uuid>,
) -> Result<(StatusCode, Json<WatchStarted>), ApiError> {
    member_of(&state, id, &user).await?;
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let root = checkout.root.canonicalize().map_err(|_| ApiError::NotFound)?;
    let lease = codex_for(&state, &checkout).await?;
    let watch_id = Uuid::new_v4().to_string();
    state.runtime.hub().start_watch(&watch_id, id);
    if let Err(err) = lease.request("fs/watch", json!({ "watchId": watch_id, "path": root })).await {
        state.runtime.hub().stop_watch(&watch_id);
        return Err(ApiError::BadRequest(format!("could not watch the checkout: {err}")));
    }
    Ok((StatusCode::CREATED, Json(WatchStarted { watch_id })))
}

/// Stops a watch started for a code chat.
async fn stop_watch(
    State(state): State<AppState>,
    user: CurrentUser,
    PathParam((id, watch_id)): PathParam<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    member_of(&state, id, &user).await?;
    if state.runtime.hub().stop_watch(&watch_id) != Some(id) {
        return Err(ApiError::NotFound);
    }
    let checkout = checkout_of(&state, id).await?.ok_or(ApiError::NotFound)?;
    let lease = codex_for(&state, &checkout).await?;
    lease
        .request("fs/unwatch", json!({ "watchId": watch_id }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not stop the watch: {err}")))?;
    Ok(StatusCode::NO_CONTENT)
}
