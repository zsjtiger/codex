//! Runs queued turns on their environment's codex app-server and records the outcome.
//!
//! Turns on one thread run one at a time, oldest first. A drain task holds the
//! thread's lock and works through its queued turns until none are left.

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{Notification, RpcError, RuntimeError, outputs};
use crate::{routes::usage::record_usage, state::AppState};

/// Longest a single turn may run before the gateway gives up waiting for it.
const TURN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
enum DispatchError {
    NoEnvironment,
    Runtime(RuntimeError),
    Rpc(RpcError),
    Protocol(String),
    Lagged,
    Disconnected,
    Timeout,
    Database(sqlx::Error),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEnvironment => f.write_str("thread has no environment"),
            Self::Runtime(err) => write!(f, "{err}"),
            Self::Rpc(err) => write!(f, "codex app-server error: {err}"),
            Self::Protocol(msg) => write!(f, "unexpected app-server response: {msg}"),
            Self::Lagged => f.write_str("event stream fell behind and dropped events"),
            Self::Disconnected => f.write_str("app-server connection closed during the turn"),
            Self::Timeout => f.write_str("turn timed out"),
            Self::Database(err) => write!(f, "database error: {err}"),
        }
    }
}

impl From<RuntimeError> for DispatchError {
    fn from(err: RuntimeError) -> Self {
        Self::Runtime(err)
    }
}

impl From<RpcError> for DispatchError {
    fn from(err: RpcError) -> Self {
        Self::Rpc(err)
    }
}

impl From<sqlx::Error> for DispatchError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}

/// Starts draining a thread's queue in the background.
pub fn spawn_drain(state: AppState, thread_id: Uuid) {
    tokio::spawn(async move { drain(&state, thread_id).await });
}

async fn drain(state: &AppState, thread_id: Uuid) {
    let lock = state.runtime.thread_lock(thread_id);
    let _turn_order = lock.lock().await;
    loop {
        let next: Option<Uuid> = match sqlx::query_scalar(
            "SELECT id FROM turns WHERE thread_id = $1 AND status = 'queued' ORDER BY seq LIMIT 1",
        )
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await
        {
            Ok(next) => next,
            Err(err) => {
                tracing::error!(%thread_id, %err, "could not read turn queue");
                return;
            }
        };
        let Some(turn_id) = next else {
            return;
        };
        if let Err(err) = run_turn(state, turn_id).await {
            tracing::warn!(%turn_id, %err, "turn failed");
            if let Err(db_err) = mark_failed(&state.pool, turn_id, &err.to_string()).await {
                tracing::error!(%turn_id, %db_err, "could not record turn failure");
            }
        }
    }
}

/// On startup: turns left `running` by a previous process can no longer finish, so they
/// are failed. Threads with queued turns get their queues drained again.
pub async fn recover(state: &AppState) -> Result<usize, sqlx::Error> {
    let interrupted = sqlx::query(
        "UPDATE turns SET status = 'failed', error = 'gateway restarted before the turn finished', completed_at = now()
         WHERE status = 'running'",
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    sqlx::query(
        "UPDATE threads SET state = 'failed'
         WHERE id IN (SELECT thread_id FROM turns WHERE error = 'gateway restarted before the turn finished')",
    )
    .execute(&state.pool)
    .await?;

    let queued: Vec<Uuid> =
        sqlx::query_scalar("SELECT DISTINCT thread_id FROM turns WHERE status = 'queued'")
            .fetch_all(&state.pool)
            .await?;
    for thread_id in queued {
        spawn_drain(state.clone(), thread_id);
    }
    Ok(interrupted as usize)
}

struct TurnRow {
    thread_id: Uuid,
    workspace_id: Uuid,
    created_by: Option<Uuid>,
    input: String,
    model: Option<String>,
    effort: Option<String>,
    codex_thread_id: Option<String>,
    environment_id: Option<Uuid>,
    endpoint: Option<String>,
    skill_ids: Vec<Uuid>,
    repo_id: Option<Uuid>,
}

/// Approval policy sent to codex for every thread and turn. See `start_codex_thread`.
const APPROVAL_POLICY: &str = "on-request";

/// Puts the instructions of the skills chosen for a turn in front of the message, so the model follows
/// them for this request. The stored message stays as the user typed it.
async fn with_skills(
    pool: &PgPool,
    skill_ids: &[Uuid],
    input: &str,
) -> Result<String, sqlx::Error> {
    if skill_ids.is_empty() {
        return Ok(input.to_string());
    }
    let skills: Vec<(String, String)> =
        sqlx::query_as("SELECT name, instructions FROM skills WHERE id = ANY($1) ORDER BY name")
            .bind(skill_ids)
            .fetch_all(pool)
            .await?;
    let mut text = String::from("Follow these skills for this request.\n");
    for (name, instructions) in skills {
        text.push_str(&format!("\n## Skill: {name}\n{instructions}\n"));
    }
    text.push_str("\n## Request\n");
    text.push_str(input);
    Ok(text)
}

/// Commits and pushes what a finished code turn changed, to the session's own branch. A failure is
/// logged; it does not fail the turn, and the person can still commit from the chat.
async fn auto_commit_turn(state: &AppState, turn: &TurnRow, repo_id: Uuid) {
    let Some(user_id) = turn.created_by else {
        return;
    };
    let result: Result<(String, String), crate::code::CodeError> = async {
        let (name, email) = crate::code::author(&state.pool, user_id).await?;
        let auth = crate::code::user_auth(&state.code, &state.pool, user_id).await?;
        let message = crate::code::auto_message(&turn.input);
        crate::code::commit_and_push(&state.code, &state.pool, repo_id, turn.thread_id, &message, &name, &email, auth).await
    }
    .await;
    match result {
        Ok((sha, branch)) => tracing::info!(%sha, %branch, "code turn committed"),
        Err(crate::code::CodeError::NothingToCommit) => {}
        Err(err) => tracing::warn!(%err, "code turn was not committed"),
    }
}

async fn run_turn(state: &AppState, turn_id: Uuid) -> Result<(), DispatchError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>, String, Option<String>, Option<String>, Option<String>, Option<Uuid>, Option<String>, Vec<Uuid>, Option<Uuid>)>(
        "SELECT th.id, th.workspace_id, t.created_by, t.input, t.model, t.effort, th.codex_thread_id, th.environment_id, e.endpoint, t.skill_ids, th.repo_id
         FROM turns t
         JOIN threads th ON th.id = t.thread_id
         LEFT JOIN environments e ON e.id = th.environment_id
         WHERE t.id = $1",
    )
    .bind(turn_id)
    .fetch_one(&state.pool)
    .await?;
    let turn = TurnRow {
        thread_id: row.0,
        workspace_id: row.1,
        created_by: row.2,
        input: row.3,
        model: row.4,
        effort: row.5,
        codex_thread_id: row.6,
        environment_id: row.7,
        endpoint: row.8,
        skill_ids: row.9,
        repo_id: row.10,
    };
    let environment_id = turn.environment_id.ok_or(DispatchError::NoEnvironment)?;
    let lease = state
        .runtime
        .lease(environment_id, turn.endpoint.as_deref())
        .await?;

    // A code thread runs in its own checkout of the repository.
    let cwd = match turn.repo_id {
        Some(repo_id) => {
            let author_auth = match turn.created_by {
                Some(user_id) => crate::code::user_auth(&state.code, &state.pool, user_id)
                    .await
                    .map_err(|err| DispatchError::Protocol(err.to_string()))?,
                None => None,
            };
            let path = crate::code::checkout(&state.code, &state.pool, repo_id, turn.thread_id, author_auth)
                .await
                .map_err(|err| DispatchError::Protocol(err.to_string()))?;
            Some(path.to_string_lossy().to_string())
        }
        None => None,
    };

    let codex_thread_id = match &turn.codex_thread_id {
        Some(id) => id.clone(),
        None => {
            let instructions =
                developer_instructions(&state.pool, turn.thread_id, turn.workspace_id).await?;
            start_codex_thread(
                &state.pool,
                &lease,
                turn.thread_id,
                turn.workspace_id,
                instructions,
                cwd.clone(),
            )
            .await?
        }
    };
    // A thread that was started by an earlier app-server process must be loaded again.
    if !lease.is_thread_loaded(&codex_thread_id) {
        lease
            .request("thread/resume", json!({ "threadId": codex_thread_id }))
            .await?;
        lease.mark_thread_loaded(&codex_thread_id);
    }

    // Subscribe before starting the turn so no notification can be missed.
    let mut events = lease.subscribe();
    // Local turns are told where to save files for the user; those files are kept with the reply.
    let output_dir = (turn.endpoint.is_none()).then(|| outputs::output_dir(turn_id));
    let mut text = with_skills(&state.pool, &turn.skill_ids, &turn.input).await?;
    if let Some(context) = project_files_context(state, turn.thread_id).await {
        text = format!("{context}\n\n{text}");
    }
    if let Some(context) = memory_context(&state.pool, turn.created_by).await {
        text = format!("{context}\n\n{text}");
    }
    if let Some(context) = project_memory_context(&state.pool, turn.thread_id).await {
        text = format!("{context}\n\n{text}");
    }
    if let Some(context) = past_chats_context(&state.pool, turn.created_by, turn.workspace_id, turn.thread_id, &turn.input).await {
        text = format!("{context}\n\n{text}");
    }
    if let Some(dir) = &output_dir {
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::warn!(%err, "could not create the turn's output folder");
        }
        text = format!("{}\n\n{text}", outputs::prompt_note(dir));
    }
    let mut params = json!({
        "threadId": codex_thread_id,
        "approvalPolicy": APPROVAL_POLICY,
        "input": [{ "type": "text", "text": text }],
    });
    if let Some(model) = &turn.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = &turn.effort {
        params["effort"] = json!(effort);
    }
    if let Some(cwd) = &cwd {
        params["cwd"] = json!(cwd);
    }
    let response = lease.request("turn/start", params).await?;
    let codex_turn_id = response
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::Protocol("turn/start returned no turn id".to_string()))?
        .to_string();

    sqlx::query("UPDATE turns SET status = 'running', codex_turn_id = $2 WHERE id = $1")
        .bind(turn_id)
        .bind(&codex_turn_id)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE threads SET state = 'running', updated_at = now() WHERE id = $1")
        .bind(turn.thread_id)
        .execute(&state.pool)
        .await?;

    let outcome = tokio::time::timeout(
        TURN_TIMEOUT,
        wait_for_completion(&mut events, &codex_thread_id, &codex_turn_id),
    )
    .await
    .map_err(|_| DispatchError::Timeout)??;

    sqlx::query("UPDATE turns SET status = $2, error = $3, completed_at = now() WHERE id = $1")
        .bind(turn_id)
        .bind(&outcome.status)
        .bind(outcome.error.as_deref())
        .execute(&state.pool)
        .await?;
    let thread_state = if outcome.status == "completed" {
        "idle"
    } else {
        "failed"
    };
    sqlx::query(
        "UPDATE threads SET state = $2, last_activity_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(turn.thread_id)
    .bind(thread_state)
    .execute(&state.pool)
    .await?;

    if outcome.status == "completed" {
        if let Some(repo_id) = turn.repo_id {
            auto_commit_turn(state, &turn, repo_id).await;
        }
    }

    if let Some(dir) = &output_dir {
        outputs::collect(state, turn.workspace_id, turn.thread_id, turn_id, dir).await?;
    }

    let (input_tokens, output_tokens) = outcome.usage;
    if input_tokens > 0 || output_tokens > 0 {
        record_usage(
            &state.pool,
            turn.workspace_id,
            turn.created_by,
            Some(turn.thread_id),
            Some(turn_id),
            input_tokens,
            output_tokens,
        )
        .await?;
    }
    Ok(())
}

/// Most saved memory text added to one turn, in bytes.
const MEMORY_BUDGET: usize = 8_000;

/// The user's saved memory notes, when memory is on. `None` when there is nothing to add.
async fn memory_context(pool: &PgPool, user_id: Option<Uuid>) -> Option<String> {
    let user_id = user_id?;
    let enabled: Option<bool> = sqlx::query_scalar("SELECT memory_enabled FROM user_settings WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()?;
    if !enabled.unwrap_or(true) {
        return None;
    }
    let notes: Vec<String> = sqlx::query_scalar("SELECT content FROM user_memories WHERE user_id = $1 ORDER BY created_at")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .ok()?;
    let mut out = String::new();
    for note in notes {
        if out.len() + note.len() + 3 > MEMORY_BUDGET {
            break;
        }
        out.push_str(&format!("\n- {note}"));
    }
    (!out.is_empty()).then(|| format!("Saved memory about the user (use it when relevant):{out}"))
}

/// Most project memory text added to one turn, in bytes.
const PROJECT_MEMORY_BUDGET: usize = 4_000;

/// The notes saved for the thread's project, when there are any.
async fn project_memory_context(pool: &PgPool, thread_id: Uuid) -> Option<String> {
    let notes: Vec<String> = sqlx::query_scalar(
        "SELECT m.content FROM project_memories m JOIN threads th ON th.project_id = m.project_id
         WHERE th.id = $1 ORDER BY m.created_at",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await
    .ok()?;
    let mut out = String::new();
    for note in notes {
        if out.len() + note.len() + 3 > PROJECT_MEMORY_BUDGET {
            break;
        }
        out.push_str(&format!("\n- {note}"));
    }
    (!out.is_empty()).then(|| format!("Project memory (use it when relevant):{out}"))
}

/// Most earlier-chat text added to one turn, in bytes.
const PAST_CHATS_BUDGET: usize = 4_000;

/// Excerpts from the user's earlier chats in the workspace that match words in the new message,
/// when the user has turned on "Search and reference chats". `None` when off or nothing matches.
async fn past_chats_context(pool: &PgPool, user_id: Option<Uuid>, workspace_id: Uuid, thread_id: Uuid, input: &str) -> Option<String> {
    let user_id = user_id?;
    let on: Option<bool> = sqlx::query_scalar("SELECT search_chats FROM user_settings WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()?;
    if !on.unwrap_or(false) {
        return None;
    }
    // Words of three or more characters, longest first, a few at most.
    let mut words: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect();
    words.sort_by_key(|word| std::cmp::Reverse(word.len()));
    words.dedup();
    words.truncate(4);
    if words.is_empty() {
        return None;
    }
    let patterns: Vec<String> = words
        .iter()
        .map(|word| format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")))
        .collect();
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT th.title, t.input, t.reply FROM turns t JOIN threads th ON th.id = t.thread_id
         WHERE th.workspace_id = $1 AND t.created_by = $2 AND th.id <> $3
           AND EXISTS (SELECT 1 FROM unnest($4::text[]) AS p WHERE t.input ILIKE p OR t.reply ILIKE p)
         ORDER BY t.created_at DESC LIMIT 5",
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(thread_id)
    .bind(&patterns)
    .fetch_all(pool)
    .await
    .ok()?;
    let mut out = String::new();
    for (title, input, reply) in rows {
        let excerpt = format!(
            "\n- {title}: user: {} / assistant: {}",
            clip(&input, 300),
            clip(reply.as_deref().unwrap_or(""), 300)
        );
        if out.len() + excerpt.len() > PAST_CHATS_BUDGET {
            break;
        }
        out.push_str(&excerpt);
    }
    (!out.is_empty()).then(|| format!("Earlier chats that may be relevant (for reference):{out}"))
}

/// The first `max` characters of a text, with a marker when cut.
fn clip(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push_str("…");
    }
    out.replace('\n', " ")
}

/// Most project file text added to one turn, in bytes.
const PROJECT_FILE_BUDGET: usize = 60_000;

/// The text read from the thread's project files, for the model. Files that were cut off or left
/// out are named, so the model can say what it could not see. `None` when there are no files.
async fn project_files_context(state: &AppState, thread_id: Uuid) -> Option<String> {
    let rows: Vec<(uuid::Uuid, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.filename, a.content_type, a.storage_key, a.extracted_text
         FROM attachments a JOIN threads th ON th.project_id = a.project_id
         WHERE th.id = $1 AND a.uploaded
         ORDER BY a.created_at",
    )
    .bind(thread_id)
    .fetch_all(&state.pool)
    .await
    .ok()?;
    let mut out = String::new();
    let mut not_fully_read = Vec::new();
    for (file_id, name, content_type, key, stored) in rows {
        // Files uploaded before their text was kept are read once, now, and the text is saved.
        let text = match stored {
            Some(text) if text.is_empty() => continue,
            Some(text) => text,
            None => {
                let Ok(bytes) = state.storage.get(&key).await else { continue };
                let Some(text) = crate::extract::extract(&name, &content_type, &bytes).await else {
                    continue;
                };
                let _ = sqlx::query("UPDATE attachments SET extracted_text = $2 WHERE id = $1")
                    .bind(file_id)
                    .bind(&text)
                    .execute(&state.pool)
                    .await;
                text
            }
        };
        let remaining = PROJECT_FILE_BUDGET.saturating_sub(out.len());
        if remaining == 0 {
            not_fully_read.push(format!("{name} (not read: the project text limit is reached)"));
            continue;
        }
        let mut end = remaining.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len() {
            not_fully_read.push(format!("{name} (cut off after {end} bytes)"));
        }
        out.push_str(&format!("\n## {name}\n{}\n", &text[..end]));
    }
    if out.is_empty() && not_fully_read.is_empty() {
        return None;
    }
    let mut context = format!("Files in this project (for reference):{out}");
    if !not_fully_read.is_empty() {
        context.push_str(&format!("\nNot fully included: {}.", not_fully_read.join("; ")));
    }
    Some(context)
}

/// Project instructions followed by each enabled skill. `None` when there is nothing to add.
async fn developer_instructions(
    pool: &PgPool,
    thread_id: Uuid,
    workspace_id: Uuid,
) -> Result<Option<String>, DispatchError> {
    let project: Option<String> = sqlx::query_scalar(
        "SELECT p.instructions FROM threads th JOIN projects p ON p.id = th.project_id WHERE th.id = $1",
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let skills: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, instructions FROM skills WHERE workspace_id = $1 AND enabled ORDER BY name",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    let mut sections: Vec<String> = Vec::new();
    if let Some(instructions) = project.filter(|text| !text.trim().is_empty()) {
        sections.push(format!("Project instructions:\n{instructions}"));
    }
    for (name, instructions) in skills {
        sections.push(format!("Skill {name}:\n{instructions}"));
    }
    Ok((!sections.is_empty()).then(|| sections.join("\n\n")))
}

/// Enabled connectors as codex `mcp_servers` entries. `None` when there are none.
/// The codex configuration for a workspace's connectors: their servers, and where MCP OAuth
/// sends its callback when the gateway has a public address. `None` when there are no connectors.
pub async fn codex_mcp_config(pool: &PgPool, workspace_id: Uuid) -> Result<Option<Value>, sqlx::Error> {
    let Some(servers) = mcp_servers(pool, workspace_id).await? else {
        return Ok(None);
    };
    let mut config = json!({ "mcp_servers": servers });
    if let Some(public) = crate::routes::connectors::public_base() {
        config["mcp_oauth_callback_url"] = json!(format!("{public}{}", crate::routes::connectors::CALLBACK_PATH));
        config["mcp_oauth_callback_port"] = json!(crate::routes::connectors::callback_port());
    }
    Ok(Some(config))
}

async fn mcp_servers(pool: &PgPool, workspace_id: Uuid) -> Result<Option<Value>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, url FROM connectors WHERE workspace_id = $1 AND enabled ORDER BY key",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let servers: serde_json::Map<String, Value> = rows
        .into_iter()
        .map(|(key, url)| (key, json!({ "url": url })))
        .collect();
    Ok(Some(Value::Object(servers)))
}

async fn start_codex_thread(
    pool: &PgPool,
    connection: &super::RpcConnection,
    thread_id: Uuid,
    workspace_id: Uuid,
    instructions: Option<String>,
    cwd: Option<String>,
) -> Result<String, DispatchError> {
    // Ask for approval when a command needs it, so the browser can answer. With "never",
    // codex would run or fail such commands without asking anyone.
    let mut params = json!({ "approvalPolicy": APPROVAL_POLICY });
    if let Some(instructions) = instructions {
        params["developerInstructions"] = json!(instructions);
    }
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    if let Some(config) = codex_mcp_config(pool, workspace_id).await? {
        params["config"] = config;
    }
    let response = connection.request("thread/start", params).await?;
    let started = response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::Protocol("thread/start returned no thread id".to_string()))?;
    sqlx::query(
        "UPDATE threads SET codex_thread_id = $2 WHERE id = $1 AND codex_thread_id IS NULL",
    )
    .bind(thread_id)
    .bind(started)
    .execute(pool)
    .await?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT codex_thread_id FROM threads WHERE id = $1")
            .bind(thread_id)
            .fetch_one(pool)
            .await?;
    let stored =
        stored.ok_or_else(|| DispatchError::Protocol("thread lost its codex id".to_string()))?;
    connection.mark_thread_loaded(&stored);
    Ok(stored)
}

struct Outcome {
    status: String,
    error: Option<String>,
    usage: (i64, i64),
}

/// Waits for `turn/completed` for this turn, summing per-call token usage on the way.
async fn wait_for_completion(
    events: &mut broadcast::Receiver<Notification>,
    codex_thread_id: &str,
    codex_turn_id: &str,
) -> Result<Outcome, DispatchError> {
    let mut usage = (0_i64, 0_i64);
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => return Err(DispatchError::Lagged),
            Err(broadcast::error::RecvError::Closed) => return Err(DispatchError::Disconnected),
        };
        let params = &event.params;
        if params.get("threadId").and_then(Value::as_str) != Some(codex_thread_id) {
            continue;
        }
        match event.method.as_str() {
            "thread/tokenUsage/updated"
                if params.get("turnId").and_then(Value::as_str) == Some(codex_turn_id) =>
            {
                usage.0 += params
                    .pointer("/tokenUsage/last/inputTokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                usage.1 += params
                    .pointer("/tokenUsage/last/outputTokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
            }
            "turn/completed"
                if params.pointer("/turn/id").and_then(Value::as_str) == Some(codex_turn_id) =>
            {
                let status = match params.pointer("/turn/status").and_then(Value::as_str) {
                    Some("completed") => "completed",
                    Some("interrupted") => "interrupted",
                    _ => "failed",
                };
                let error = params
                    .pointer("/turn/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Ok(Outcome {
                    status: status.to_string(),
                    error,
                    usage,
                });
            }
            _ => {}
        }
    }
}

async fn mark_failed(pool: &PgPool, turn_id: Uuid, message: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE turns SET status = 'failed', error = $2, completed_at = now()
         WHERE id = $1 AND status IN ('queued', 'running')",
    )
    .bind(turn_id)
    .bind(message)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE threads SET state = 'failed', updated_at = now()
         WHERE id = (SELECT thread_id FROM turns WHERE id = $1)",
    )
    .bind(turn_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Asks app-server to interrupt a running turn.
pub async fn interrupt(state: &AppState, turn_id: Uuid) -> Result<(), String> {
    let (codex_thread_id, codex_turn_id, environment_id, endpoint): (
        Option<String>,
        Option<String>,
        Option<Uuid>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT th.codex_thread_id, t.codex_turn_id, th.environment_id, e.endpoint
             FROM turns t
             JOIN threads th ON th.id = t.thread_id
             LEFT JOIN environments e ON e.id = th.environment_id
             WHERE t.id = $1",
    )
    .bind(turn_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    let environment_id = environment_id.ok_or_else(|| "thread has no environment".to_string())?;
    let (Some(codex_thread_id), Some(codex_turn_id)) = (codex_thread_id, codex_turn_id) else {
        return Err("turn has not started on app-server yet".to_string());
    };
    let lease = state
        .runtime
        .lease(environment_id, endpoint.as_deref())
        .await
        .map_err(|err| err.to_string())?;
    lease
        .request(
            "turn/interrupt",
            json!({ "threadId": codex_thread_id, "turnId": codex_turn_id }),
        )
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}
