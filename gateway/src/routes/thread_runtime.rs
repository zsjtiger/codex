//! What codex keeps for a chat as it works: the chat's goal, and the background terminals it started. The
//! gateway asks codex for them and passes changes on; the chat's history stays in the gateway.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    runtime::Lease,
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/threads/{id}/goal", get(get_goal).put(set_goal).delete(clear_goal))
        .route("/threads/{id}/background-terminals", get(list_terminals))
        .route("/threads/{id}/background-terminals/clean", post(clean_terminals))
        .route("/threads/{id}/background-terminals/{process_id}/terminate", post(terminate_terminal))
        .route("/turns/{id}/prediction", post(request_prediction))
}

/// The chat's workspace, and its codex thread and environment. A chat codex has not started has none of these.
struct CodexChat {
    workspace_id: Uuid,
    codex_thread_id: Option<String>,
    environment_id: Uuid,
    endpoint: Option<String>,
}

async fn codex_chat(pool: &PgPool, thread_id: Uuid) -> Result<CodexChat, ApiError> {
    let row: Option<(Uuid, Option<String>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT th.workspace_id, th.codex_thread_id, th.environment_id, e.endpoint
           FROM threads th LEFT JOIN environments e ON e.id = th.environment_id
          WHERE th.id = $1",
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;
    let Some((workspace_id, codex_thread_id, environment_id, endpoint)) = row else {
        return Err(ApiError::NotFound);
    };
    let Some(environment_id) = environment_id else {
        return Err(ApiError::Conflict("this chat has no environment".to_string()));
    };
    Ok(CodexChat { workspace_id, codex_thread_id, environment_id, endpoint })
}

async fn member(state: &AppState, thread_id: Uuid, user: &CurrentUser) -> Result<CodexChat, ApiError> {
    let chat = codex_chat(&state.pool, thread_id).await?;
    require_role(&state.pool, chat.workspace_id, user.id, Role::Member).await?;
    Ok(chat)
}

async fn lease_for(state: &AppState, chat: &CodexChat) -> Result<Lease, ApiError> {
    state
        .runtime
        .lease(chat.environment_id, chat.endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))
}

/// The codex thread of a chat that has started; a chat with no codex thread yet has no goal or terminals.
fn started(chat: &CodexChat) -> Result<&str, ApiError> {
    chat.codex_thread_id
        .as_deref()
        .ok_or_else(|| ApiError::Conflict("send a message to the chat first".to_string()))
}

#[derive(Serialize)]
pub struct Goal {
    objective: String,
    /// active, paused, blocked, usageLimited, budgetLimited, or complete, as codex names them.
    status: String,
    token_budget: Option<i64>,
    tokens_used: i64,
    time_used_seconds: i64,
}

fn goal_view(value: &Value) -> Option<Goal> {
    Some(Goal {
        objective: value["objective"].as_str()?.to_string(),
        status: value["status"].as_str()?.to_string(),
        token_budget: value["tokenBudget"].as_i64(),
        tokens_used: value["tokensUsed"].as_i64().unwrap_or(0),
        time_used_seconds: value["timeUsedSeconds"].as_i64().unwrap_or(0),
    })
}

const GOAL_STATUSES: [&str; 6] = ["active", "paused", "blocked", "usageLimited", "budgetLimited", "complete"];

/// The chat's goal, or null if it has none.
async fn get_goal(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<Json<Option<Goal>>, ApiError> {
    let chat = member(&state, id, &user).await?;
    let Some(thread) = chat.codex_thread_id.clone() else {
        return Ok(Json(None));
    };
    let lease = lease_for(&state, &chat).await?;
    let response = lease
        .request("thread/goal/get", json!({ "threadId": thread }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not read the goal: {err}")))?;
    Ok(Json(goal_view(&response["goal"])))
}

#[derive(Deserialize)]
pub struct GoalChange {
    objective: Option<String>,
    status: Option<String>,
    /// A number sets the token budget, null removes it, and leaving it out keeps it.
    token_budget: Option<Value>,
}

/// Sets the chat's goal, or changes part of it. Members can do this; each change is recorded in the audit log.
async fn set_goal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<GoalChange>,
) -> Result<Json<Goal>, ApiError> {
    let chat = member(&state, id, &user).await?;
    let thread = started(&chat)?.to_string();
    let mut params = json!({ "threadId": thread, "origin": "user" });
    if let Some(objective) = &req.objective {
        let objective = objective.trim();
        if objective.is_empty() || objective.chars().count() > 2_000 {
            return Err(ApiError::BadRequest("the goal is 1 to 2000 characters".to_string()));
        }
        params["objective"] = json!(objective);
    }
    if let Some(status) = &req.status {
        if !GOAL_STATUSES.contains(&status.as_str()) {
            return Err(ApiError::BadRequest("unknown goal status".to_string()));
        }
        params["status"] = json!(status);
    }
    if let Some(budget) = &req.token_budget {
        match budget {
            Value::Null => params["tokenBudget"] = Value::Null,
            Value::Number(number) if number.as_i64().is_some_and(|n| n > 0) => params["tokenBudget"] = json!(number),
            _ => return Err(ApiError::BadRequest("the token budget is a positive number, or null for none".to_string())),
        }
    }
    let lease = lease_for(&state, &chat).await?;
    let response = lease
        .request("thread/goal/set", params)
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not set the goal: {err}")))?;
    let goal = goal_view(&response["goal"]).ok_or(ApiError::Internal)?;
    audit::record(&state.pool, chat.workspace_id, user.id, "thread.goal_set", "thread", id, json!({ "status": goal.status, "objective": goal.objective })).await?;
    Ok(Json(goal))
}

/// Clears the chat's goal.
async fn clear_goal(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let chat = member(&state, id, &user).await?;
    let thread = started(&chat)?.to_string();
    let lease = lease_for(&state, &chat).await?;
    let response = lease
        .request("thread/goal/clear", json!({ "threadId": thread, "origin": "user" }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not clear the goal: {err}")))?;
    if response["cleared"].as_bool() != Some(true) {
        return Err(ApiError::NotFound);
    }
    audit::record(&state.pool, chat.workspace_id, user.id, "thread.goal_cleared", "thread", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct Terminal {
    process_id: String,
    command: String,
    cwd: String,
    os_pid: Option<u32>,
    cpu_percent: Option<f64>,
    rss_kb: Option<u64>,
}

#[derive(Serialize)]
pub struct Terminals {
    terminals: Vec<Terminal>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct PageQuery {
    cursor: Option<String>,
}

/// The background terminals codex has running for the chat, a page at a time.
async fn list_terminals(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Terminals>, ApiError> {
    let chat = member(&state, id, &user).await?;
    let Some(thread) = chat.codex_thread_id.clone() else {
        return Ok(Json(Terminals { terminals: Vec::new(), next_cursor: None }));
    };
    let lease = lease_for(&state, &chat).await?;
    let response = lease
        .request("thread/backgroundTerminals/list", json!({ "threadId": thread, "cursor": query.cursor }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not list the terminals: {err}")))?;
    let terminals = response["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Terminal {
                process_id: item["processId"].as_str()?.to_string(),
                command: item["command"].as_str().unwrap_or_default().to_string(),
                cwd: item["cwd"].as_str().unwrap_or_default().to_string(),
                os_pid: item["osPid"].as_u64().and_then(|pid| u32::try_from(pid).ok()),
                cpu_percent: item["cpuPercent"].as_f64(),
                rss_kb: item["rssKb"].as_u64(),
            })
        })
        .collect();
    Ok(Json(Terminals { terminals, next_cursor: response["nextCursor"].as_str().map(str::to_string) }))
}

/// Stops one of the chat's background terminals.
async fn terminate_terminal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, process_id)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    let chat = member(&state, id, &user).await?;
    let thread = started(&chat)?.to_string();
    let lease = lease_for(&state, &chat).await?;
    let response = lease
        .request("thread/backgroundTerminals/terminate", json!({ "threadId": thread, "processId": process_id }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not stop the terminal: {err}")))?;
    if response["terminated"].as_bool() != Some(true) {
        return Err(ApiError::NotFound);
    }
    audit::record(&state.pool, chat.workspace_id, user.id, "thread.terminal_stop", "thread", id, json!({ "process_id": process_id })).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stops every background terminal the chat has finished with, as codex decides which those are.
async fn clean_terminals(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let chat = member(&state, id, &user).await?;
    let thread = started(&chat)?.to_string();
    let lease = lease_for(&state, &chat).await?;
    lease
        .request("thread/backgroundTerminals/clean", json!({ "threadId": thread }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not clean up the terminals: {err}")))?;
    audit::record(&state.pool, chat.workspace_id, user.id, "thread.terminals_clean", "thread", id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Asks codex to suggest what the person might write next, from a finished turn. The suggestion comes later, as
/// a `thread/prediction/updated` notice to the chat. It is not asked for on every turn: a person asks for it.
async fn request_prediction(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let row: Option<(Uuid, String, Option<String>, Option<String>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT th.workspace_id, t.status, t.codex_turn_id, th.codex_thread_id, th.environment_id, e.endpoint
           FROM turns t
           JOIN threads th ON th.id = t.thread_id
           LEFT JOIN environments e ON e.id = th.environment_id
          WHERE t.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((workspace_id, status, codex_turn_id, codex_thread_id, environment_id, endpoint)) = row else {
        return Err(ApiError::NotFound);
    };
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    if status != "completed" {
        return Err(ApiError::Conflict("a suggestion is made from a finished turn".to_string()));
    }
    let (Some(codex_turn_id), Some(codex_thread_id), Some(environment_id)) = (codex_turn_id, codex_thread_id, environment_id) else {
        return Err(ApiError::Conflict("this turn has no codex history to suggest from".to_string()));
    };
    let busy: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM turns t JOIN turns self ON self.id = $1 WHERE t.thread_id = self.thread_id AND t.status IN ('running', 'queued'))",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    if busy {
        return Err(ApiError::Conflict("a suggestion waits until the chat is idle".to_string()));
    }
    let lease = state
        .runtime
        .lease(environment_id, endpoint.as_deref())
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    lease
        .request("thread/prediction/request", json!({ "threadId": codex_thread_id, "sourceTurnId": codex_turn_id }))
        .await
        .map_err(|err| ApiError::BadRequest(format!("could not ask for a suggestion: {err}")))?;
    Ok(StatusCode::ACCEPTED)
}
