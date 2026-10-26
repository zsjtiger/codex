//! Runs one queued turn on its environment's codex app-server and writes the outcome back.

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::{sync::broadcast, time::timeout};
use uuid::Uuid;

use super::{Notification, RpcConnection, RpcError};
use crate::{routes::usage::record_usage, state::AppState};

/// Longest a single turn may run before the gateway gives up waiting for it.
const TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

#[derive(Debug)]
enum DispatchError {
    NoEnvironment,
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
            Self::NoEnvironment => f.write_str("thread has no environment with an endpoint"),
            Self::Rpc(err) => write!(f, "codex app-server error: {err}"),
            Self::Protocol(msg) => write!(f, "unexpected app-server response: {msg}"),
            Self::Lagged => f.write_str("event stream fell behind and dropped events"),
            Self::Disconnected => f.write_str("app-server connection closed during the turn"),
            Self::Timeout => f.write_str("turn timed out"),
            Self::Database(err) => write!(f, "database error: {err}"),
        }
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

/// Dispatches a turn in the background. Failures are recorded on the turn, not returned.
pub fn spawn_turn(state: AppState, turn_id: Uuid) {
    tokio::spawn(async move {
        if let Err(err) = run_turn(&state, turn_id).await {
            tracing::warn!(%turn_id, %err, "turn failed");
            if let Err(db_err) = mark_failed(&state.pool, turn_id, &err.to_string()).await {
                tracing::error!(%turn_id, %db_err, "could not record turn failure");
            }
        }
    });
}

struct TurnRow {
    thread_id: Uuid,
    workspace_id: Uuid,
    created_by: Option<Uuid>,
    input: String,
    codex_thread_id: Option<String>,
    environment_id: Option<Uuid>,
    endpoint: Option<String>,
}

async fn run_turn(state: &AppState, turn_id: Uuid) -> Result<(), DispatchError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>, String, Option<String>, Option<Uuid>, Option<String>)>(
        "SELECT th.id, th.workspace_id, t.created_by, t.input, th.codex_thread_id, th.environment_id, e.endpoint
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
        codex_thread_id: row.4,
        environment_id: row.5,
        endpoint: row.6,
    };

    let (Some(environment_id), Some(endpoint)) = (turn.environment_id, turn.endpoint.as_deref())
    else {
        return Err(DispatchError::NoEnvironment);
    };
    let connection = state.runtime.connection(environment_id, endpoint).await?;

    let codex_thread_id = match &turn.codex_thread_id {
        Some(id) => id.clone(),
        None => start_codex_thread(&state.pool, &connection, turn.thread_id).await?,
    };

    // Subscribe before starting the turn so no notification can be missed.
    let mut events = connection.subscribe();
    let response = connection
        .request(
            "turn/start",
            json!({
                "threadId": codex_thread_id,
                "input": [{ "type": "text", "text": turn.input }],
            }),
        )
        .await?;
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

    let outcome = timeout(
        TURN_TIMEOUT,
        wait_for_completion(&mut events, &codex_thread_id, &codex_turn_id),
    )
    .await
    .map_err(|_| DispatchError::Timeout)??;

    let (input_tokens, output_tokens) = outcome.usage;
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

async fn start_codex_thread(
    pool: &PgPool,
    connection: &RpcConnection,
    thread_id: Uuid,
) -> Result<String, DispatchError> {
    let response = connection.request("thread/start", json!({})).await?;
    let started = response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::Protocol("thread/start returned no thread id".to_string()))?;
    // Keep the first id if two turns race to start the same thread.
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
    stored.ok_or_else(|| DispatchError::Protocol("thread lost its codex id".to_string()))
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
                let raw_status = params
                    .pointer("/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                let status = match raw_status {
                    "completed" => "completed",
                    "interrupted" => "interrupted",
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
