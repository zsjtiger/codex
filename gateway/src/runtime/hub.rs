//! Routes app-server traffic to browser sessions.
//!
//! One pump per app-server connection reads its notifications and server requests.
//! Events go to the browsers subscribed to the gateway thread they belong to.
//! Approval requests wait for a subscriber's answer; with no subscriber, or after
//! `APPROVAL_TIMEOUT`, they are declined so the turn can continue.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::rpc::{RpcConnection, ServerRequest};

pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const METHOD_NOT_SUPPORTED: i64 = -32601;

pub type Outbound = mpsc::UnboundedSender<Value>;

struct Pending {
    thread_id: Uuid,
    connection: Arc<RpcConnection>,
    id: Value,
    method: String,
}

#[derive(Default)]
pub struct Hub {
    subscribers: Mutex<HashMap<Uuid, Vec<(u64, Outbound)>>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    codex_threads: Mutex<HashMap<String, Uuid>>,
    next_subscription: AtomicU64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    Unknown,
    NotSubscribed,
}

impl Hub {
    /// Registers a browser for a thread's events. Returns a handle for `unsubscribe`.
    pub fn subscribe(&self, thread_id: Uuid, outbound: Outbound) -> u64 {
        let handle = self.next_subscription.fetch_add(1, Ordering::Relaxed);
        self.subscribers
            .lock()
            .expect("subscribers lock poisoned")
            .entry(thread_id)
            .or_default()
            .push((handle, outbound));
        handle
    }

    pub fn unsubscribe(&self, thread_id: Uuid, handle: u64) {
        let mut subscribers = self.subscribers.lock().expect("subscribers lock poisoned");
        if let Some(list) = subscribers.get_mut(&thread_id) {
            list.retain(|(h, _)| *h != handle);
            if list.is_empty() {
                subscribers.remove(&thread_id);
            }
        }
    }

    fn has_subscribers(&self, thread_id: Uuid) -> bool {
        self.subscribers
            .lock()
            .expect("subscribers lock poisoned")
            .get(&thread_id)
            .is_some_and(|list| !list.is_empty())
    }

    fn send_to(&self, thread_id: Uuid, frame: &Value) {
        let mut subscribers = self.subscribers.lock().expect("subscribers lock poisoned");
        if let Some(list) = subscribers.get_mut(&thread_id) {
            list.retain(|(_, outbound)| outbound.send(frame.clone()).is_ok());
        }
    }

    /// Maps a codex thread id to the gateway thread that owns it, caching the result.
    async fn gateway_thread(&self, pool: &PgPool, codex_thread_id: &str) -> Option<Uuid> {
        if let Some(found) = self
            .codex_threads
            .lock()
            .expect("codex_threads lock poisoned")
            .get(codex_thread_id)
        {
            return Some(*found);
        }
        let found: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM threads WHERE codex_thread_id = $1")
                .bind(codex_thread_id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        if let Some(thread_id) = found {
            self.codex_threads
                .lock()
                .expect("codex_threads lock poisoned")
                .insert(codex_thread_id.to_string(), thread_id);
        }
        found
    }

    async fn forward_event(&self, pool: &PgPool, method: &str, params: &Value) {
        let Some(codex_thread_id) = params.get("threadId").and_then(Value::as_str) else {
            return;
        };
        let Some(thread_id) = self.gateway_thread(pool, codex_thread_id).await else {
            return;
        };
        if method == "mcpServer/startupStatus/updated" {
            record_connector_status(pool, thread_id, params).await;
        }
        if method == "item/completed" {
            record_tool_use(pool, thread_id, params).await;
            record_work(pool, thread_id, params).await;
            record_reply(pool, thread_id, params).await;
        }
        self.send_to(
            thread_id,
            &json!({ "type": "event", "threadId": thread_id, "method": method, "params": params }),
        );
    }

    async fn handle_server_request(
        self: &Arc<Self>,
        pool: &PgPool,
        connection: &Arc<RpcConnection>,
        request: ServerRequest,
    ) {
        let codex_thread_id = request
            .params
            .get("threadId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let thread_id = match self.gateway_thread(pool, codex_thread_id).await {
            Some(thread_id) if self.has_subscribers(thread_id) => thread_id,
            found => {
                // Nobody is watching this chat (for example a scheduled run), so the request is declined
                // at once. The decline is recorded so the task's history can say what was skipped.
                if let Some(thread_id) = found {
                    record_unattended_decline(pool, thread_id, &request.method).await;
                }
                decline(connection, &request.id, &request.method).await;
                return;
            }
        };

        let request_id = Uuid::new_v4();
        self.pending.lock().expect("pending lock poisoned").insert(
            request_id,
            Pending {
                thread_id,
                connection: Arc::clone(connection),
                id: request.id.clone(),
                method: request.method.clone(),
            },
        );
        self.send_to(
            thread_id,
            &json!({
                "type": "approvalRequest",
                "requestId": request_id,
                "threadId": thread_id,
                "method": request.method,
                "params": request.params,
            }),
        );

        let hub = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_TIMEOUT).await;
            let expired = hub
                .pending
                .lock()
                .expect("pending lock poisoned")
                .remove(&request_id);
            if let Some(pending) = expired {
                decline(&pending.connection, &pending.id, &pending.method).await;
            }
        });
    }

    /// Delivers a browser's answer to a pending approval. `subscribed` lists the
    /// threads the answering session is subscribed to.
    pub async fn resolve(
        &self,
        request_id: Uuid,
        subscribed: &[Uuid],
        result: Value,
    ) -> Result<(), ResolveError> {
        let pending = {
            let mut map = self.pending.lock().expect("pending lock poisoned");
            match map.get(&request_id) {
                None => return Err(ResolveError::Unknown),
                Some(p) if !subscribed.contains(&p.thread_id) => {
                    return Err(ResolveError::NotSubscribed);
                }
                Some(_) => map.remove(&request_id).expect("checked above"),
            }
        };
        let _ = pending.connection.respond(pending.id, result).await;
        Ok(())
    }

    /// Reads one connection's traffic until the connection closes.
    pub async fn pump(self: Arc<Self>, pool: PgPool, connection: Arc<RpcConnection>) {
        let mut notifications = connection.subscribe();
        let mut requests = connection.subscribe_server_requests();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                event = notifications.recv() => match event {
                    Ok(event) => self.forward_event(&pool, &event.method, &event.params).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                request = requests.recv() => match request {
                    Ok(request) => self.handle_server_request(&pool, &connection, request).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if connection.is_closed() {
                        break;
                    }
                }
            }
        }
    }
}

/// The answer that means "do not run this" for each request kind. Unknown kinds get an error.
/// Writes an audit entry for an approval that was declined because no one was watching the chat.
async fn record_unattended_decline(pool: &PgPool, thread_id: Uuid, method: &str) {
    let result = sqlx::query(
        "INSERT INTO audit_logs (workspace_id, actor_id, action, target_type, target_id, metadata)
         SELECT workspace_id, NULL, 'approval.declined_unattended', 'thread', id::text, $2
         FROM threads WHERE id = $1",
    )
    .bind(thread_id)
    .bind(json!({ "method": method }))
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record a declined approval");
    }
}

async fn decline(connection: &RpcConnection, id: &Value, method: &str) {
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({ "decision": "decline" }))
        }
        "item/permissions/requestApproval" => Some(json!({ "permissions": {}, "scope": "turn" })),
        "item/tool/requestUserInput" => Some(json!({ "answers": {} })),
        "item/tool/call" => Some(json!({ "contentItems": [], "success": false })),
        _ => None,
    };
    match result {
        Some(result) => {
            let _ = connection.respond(id.clone(), result).await;
        }
        None => {
            let _ = connection
                .respond_error(
                    id.clone(),
                    METHOD_NOT_SUPPORTED,
                    "not supported by codex-gateway",
                )
                .await;
        }
    }
}

/// Stores the startup status codex reports for an MCP server as the workspace connector's status.
/// Unknown status values are ignored.
async fn record_connector_status(pool: &PgPool, thread_id: Uuid, params: &Value) {
    let (Some(name), Some(status)) = (
        params.get("name").and_then(Value::as_str),
        params.get("status").and_then(Value::as_str),
    ) else {
        return;
    };
    if !matches!(status, "starting" | "ready" | "failed" | "cancelled") {
        return;
    }
    let error = params
        .get("error")
        .and_then(Value::as_str)
        .map(|e| truncate(e, 2000));
    let result = sqlx::query(
        "UPDATE connectors SET status = $3, status_error = $4, status_at = now()
         WHERE workspace_id = (SELECT workspace_id FROM threads WHERE id = $1) AND key = $2",
    )
    .bind(thread_id)
    .bind(name)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record connector status");
    }
}

/// Appends the text of a completed assistant message to its turn, so the reply survives a reload.
async fn record_reply(pool: &PgPool, thread_id: Uuid, params: &Value) {
    let item = match params.get("item") {
        Some(item) if item.get("type").and_then(Value::as_str) == Some("agentMessage") => item,
        _ => return,
    };
    let (Some(codex_turn_id), Some(text)) = (
        params.get("turnId").and_then(Value::as_str),
        item.get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty()),
    ) else {
        return;
    };
    let text = truncate(text, 200_000);
    let result = sqlx::query(
        "UPDATE turns SET reply = CASE WHEN reply IS NULL THEN $3
                                       ELSE left(reply || E'\n\n' || $3, 200000) END
         WHERE thread_id = $1 AND codex_turn_id = $2",
    )
    .bind(thread_id)
    .bind(codex_turn_id)
    .bind(text)
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record assistant reply");
    }
}

/// Stores a completed tool, search, or file action so the conversation can list what Codex used.
/// Re-delivered items are ignored.
/// Largest output or diff kept per item, in bytes. The end of a long output is what is kept.
const WORK_TEXT_LIMIT: usize = 64 * 1024;

fn capped(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("[earlier output cut]\n{}", &text[start..])
}

/// Keeps a finished command or file change, so the code view shows it again after a reload.
async fn record_work(pool: &PgPool, thread_id: Uuid, params: &Value) {
    let Some(item) = params.get("item") else {
        return;
    };
    if !matches!(item.get("type").and_then(Value::as_str), Some("commandExecution" | "fileChange")) {
        return;
    }
    let (Some(item_id), Some(codex_turn_id)) = (
        item.get("id").and_then(Value::as_str),
        params.get("turnId").and_then(Value::as_str),
    ) else {
        return;
    };
    let mut item = item.clone();
    if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str).map(str::to_string) {
        item["aggregatedOutput"] = Value::String(capped(&output, WORK_TEXT_LIMIT));
    }
    if let Some(changes) = item.get_mut("changes").and_then(Value::as_array_mut) {
        for change in changes {
            if let Some(diff) = change.get("diff").and_then(Value::as_str).map(str::to_string) {
                change["diff"] = Value::String(capped(&diff, WORK_TEXT_LIMIT));
            }
        }
    }
    let result = sqlx::query(
        "INSERT INTO turn_work (thread_id, codex_turn_id, item_id, item) VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id, item_id) DO UPDATE SET item = EXCLUDED.item",
    )
    .bind(thread_id)
    .bind(codex_turn_id)
    .bind(item_id)
    .bind(item)
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record work item");
    }
}

async fn record_tool_use(pool: &PgPool, thread_id: Uuid, params: &Value) {
    let Some((item_id, kind, label)) = params.get("item").and_then(tool_use_from_item) else {
        return;
    };
    let result = sqlx::query(
        "INSERT INTO thread_tool_uses (thread_id, item_id, kind, label) VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id, item_id) DO NOTHING",
    )
    .bind(thread_id)
    .bind(item_id)
    .bind(kind)
    .bind(label)
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record tool use");
    }
}

/// Maps a completed thread item to the (item id, kind, label) shown in the Used panel.
/// Ordinary messages and reasoning return `None`.
pub(crate) fn tool_use_from_item(item: &Value) -> Option<(String, &'static str, String)> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let field = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let (kind, label) = match item.get("type")?.as_str()? {
        "webSearch" => ("Web search", field("query")),
        "mcpToolCall" => ("Connector", field("server")),
        "dynamicToolCall" => ("Tool", field("tool")),
        "commandExecution" => ("Command", field("command")),
        "fileChange" => {
            let count = item
                .get("changes")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let noun = if count == 1 { "file" } else { "files" };
            ("File change", format!("{count} {noun}"))
        }
        "imageView" => ("Image", field("path")),
        "imageGeneration" => ("Image generation", field("revisedPrompt")),
        _ => return None,
    };
    Some((truncate(id, 200), kind, truncate(&label, 500)))
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::tool_use_from_item;
    use serde_json::json;

    #[test]
    fn maps_tool_items_and_ignores_messages() {
        assert_eq!(
            tool_use_from_item(&json!({"id": "a", "type": "webSearch", "query": "q"})),
            Some(("a".to_string(), "Web search", "q".to_string()))
        );
        assert_eq!(
            tool_use_from_item(&json!({"id": "b", "type": "fileChange", "changes": [{}]})),
            Some(("b".to_string(), "File change", "1 file".to_string()))
        );
        assert_eq!(
            tool_use_from_item(&json!({"id": "c", "type": "fileChange", "changes": [{}, {}]})),
            Some(("c".to_string(), "File change", "2 files".to_string()))
        );
        assert_eq!(
            tool_use_from_item(&json!({"id": "d", "type": "agentMessage", "text": "hi"})),
            None
        );
        assert_eq!(
            tool_use_from_item(&json!({"type": "webSearch", "query": "no id"})),
            None
        );
    }
}
