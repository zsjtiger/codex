//! Runs queued turns on their environment's codex app-server and records the outcome.
//!
//! Turns on one thread run one at a time, oldest first. A drain task holds the
//! thread's lock and works through its queued turns until none are left.

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{Notification, RpcError, RuntimeError};
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
}

async fn run_turn(state: &AppState, turn_id: Uuid) -> Result<(), DispatchError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>, String, Option<String>, Option<String>, Option<String>, Option<Uuid>, Option<String>)>(
        "SELECT th.id, th.workspace_id, t.created_by, t.input, t.model, t.effort, th.codex_thread_id, th.environment_id, e.endpoint
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
    };
    let environment_id = turn.environment_id.ok_or(DispatchError::NoEnvironment)?;
    let lease = state
        .runtime
        .lease(environment_id, turn.endpoint.as_deref())
        .await?;

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
    let mut params = json!({
        "threadId": codex_thread_id,
        "input": [{ "type": "text", "text": turn.input }],
    });
    if let Some(model) = &turn.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = &turn.effort {
        params["effort"] = json!(effort);
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
async fn mcp_servers(pool: &PgPool, workspace_id: Uuid) -> Result<Option<Value>, DispatchError> {
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
) -> Result<String, DispatchError> {
    let mut params = json!({});
    if let Some(instructions) = instructions {
        params["developerInstructions"] = json!(instructions);
    }
    if let Some(servers) = mcp_servers(pool, workspace_id).await? {
        params["config"] = json!({ "mcp_servers": servers });
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
