//! Routes app-server traffic to browser sessions.
//!
//! One pump per app-server connection reads its notifications and server requests.
//! Events go to the browsers subscribed to the gateway thread they belong to.
//! Approval requests wait for a subscriber's answer; with no subscriber, or after
//! `APPROVAL_TIMEOUT`, they are declined so the turn can continue.

use base64::{Engine as _, engine::general_purpose::STANDARD};
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

/// Notices codex sends for an environment as a whole, which browsers watching its chats are told about.
const ENVIRONMENT_NOTICES: &[&str] = &[
    "account/rateLimits/updated",
    "skills/changed",
    "warning",
    "configWarning",
    "deprecationNotice",
];

/// Most output kept for each stream of a command run from a chat.
pub const RUN_OUTPUT_LIMIT: usize = 256 * 1024;

/// A command run from a code chat that is still going: its chat, and the output codex has sent so far.
#[derive(Default)]
struct ProcessOutput {
    thread_id: Uuid,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// Whether the run was started with its standard input open, so text can be written to it.
    stdin: bool,
    /// Whether the run has a terminal, so its size can be set.
    tty: bool,
}

/// What a command run was started with, for the requests that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMode {
    pub thread_id: Uuid,
    pub stdin: bool,
    pub tty: bool,
}

#[derive(Default)]
pub struct Hub {
    subscribers: Mutex<HashMap<Uuid, Vec<(u64, Outbound)>>>,
    /// Command runs in progress, by process id. Their output is collected here and sent to the chat.
    processes: Mutex<HashMap<String, ProcessOutput>>,
    /// File watches a chat has open, by watch id. Changes to the files are sent to the chat.
    watches: Mutex<HashMap<String, Uuid>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    codex_threads: Mutex<HashMap<String, Uuid>>,
    /// Reply text whose turn is not yet linked to its codex turn, by codex turn id. See `record_reply`.
    unplaced_replies: tokio::sync::Mutex<HashMap<String, (Uuid, Vec<String>)>>,
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

    /// Starts collecting the output of a command run. False if a run with that id is already going.
    pub fn start_process(&self, process_id: &str, mode: ProcessMode) -> bool {
        let mut processes = self.processes.lock().expect("processes lock poisoned");
        if processes.contains_key(process_id) {
            return false;
        }
        processes.insert(
            process_id.to_string(),
            ProcessOutput { thread_id: mode.thread_id, stdin: mode.stdin, tty: mode.tty, ..Default::default() },
        );
        true
    }

    /// What a command run that is going was started with.
    pub fn process_mode(&self, process_id: &str) -> Option<ProcessMode> {
        self.processes.lock().expect("processes lock poisoned").get(process_id).map(|process| ProcessMode {
            thread_id: process.thread_id,
            stdin: process.stdin,
            tty: process.tty,
        })
    }

    /// Records a file watch a chat has open. Its changes are sent to that chat.
    pub fn start_watch(&self, watch_id: &str, thread_id: Uuid) {
        self.watches.lock().expect("watches lock poisoned").insert(watch_id.to_string(), thread_id);
    }

    /// Forgets a file watch; its changes are no longer sent.
    pub fn stop_watch(&self, watch_id: &str) -> Option<Uuid> {
        self.watches.lock().expect("watches lock poisoned").remove(watch_id)
    }

    /// The chat a command run belongs to, while it is going.
    pub fn process_thread(&self, process_id: &str) -> Option<Uuid> {
        self.processes
            .lock()
            .expect("processes lock poisoned")
            .get(process_id)
            .map(|process| process.thread_id)
    }

    /// Ends a command run's collection and returns the output it kept, as (stdout, stderr).
    pub fn finish_process(&self, process_id: &str) -> Option<(String, String)> {
        let process = self.processes.lock().expect("processes lock poisoned").remove(process_id)?;
        Some((
            String::from_utf8_lossy(&process.stdout).into_owned(),
            String::from_utf8_lossy(&process.stderr).into_owned(),
        ))
    }

    /// Keeps a chunk of a command run's output, and sends it to the chat as it arrives.
    fn record_process_output(&self, params: &Value) {
        let Some(process_id) = params.get("processId").and_then(Value::as_str) else {
            return;
        };
        let thread_id = {
            let mut processes = self.processes.lock().expect("processes lock poisoned");
            let Some(process) = processes.get_mut(process_id) else {
                return;
            };
            if let Some(bytes) = params.get("deltaBase64").and_then(Value::as_str).and_then(|d| STANDARD.decode(d).ok()) {
                let buffer = if params.get("stream").and_then(Value::as_str) == Some("stderr") {
                    &mut process.stderr
                } else {
                    &mut process.stdout
                };
                let room = RUN_OUTPUT_LIMIT.saturating_sub(buffer.len());
                buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
            }
            process.thread_id
        };
        self.send_to(
            thread_id,
            &json!({ "type": "event", "threadId": thread_id, "method": "command/exec/outputDelta", "params": params }),
        );
    }

    /// Sends an environment-wide notice (one that names no thread) to every browser watching a chat in that environment.
    async fn forward_environment_event(&self, pool: &PgPool, environment_id: Uuid, method: &str, params: &Value) {
        let threads: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM threads WHERE environment_id = $1")
            .bind(environment_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        let frame = json!({ "type": "event", "environmentId": environment_id, "method": method, "params": params });
        let mut subscribers = self.subscribers.lock().expect("subscribers lock poisoned");
        // A browser watching several chats in the environment gets the notice once.
        let mut reached = std::collections::HashSet::new();
        for thread in threads {
            if let Some(list) = subscribers.get_mut(&thread) {
                list.retain(|(handle, outbound)| {
                    if !reached.insert(*handle) {
                        return true;
                    }
                    outbound.send(frame.clone()).is_ok()
                });
            }
        }
    }

    async fn forward_event(&self, pool: &PgPool, environment_id: Uuid, method: &str, params: &Value) {
        // Command output names its process, not its thread, so it is routed by the process.
        if method == "command/exec/outputDelta" {
            self.record_process_output(params);
            return;
        }
        // File changes name their watch, which belongs to one chat.
        if method == "fs/changed" {
            let thread = params.get("watchId").and_then(Value::as_str).and_then(|id| self.watches.lock().expect("watches lock poisoned").get(id).copied());
            if let Some(thread_id) = thread {
                self.send_to(thread_id, &json!({ "type": "event", "threadId": thread_id, "method": method, "params": params }));
            }
            return;
        }
        // Some notices name no thread but concern the whole environment's codex.
        if ENVIRONMENT_NOTICES.contains(&method) && params.get("threadId").is_none() {
            self.forward_environment_event(pool, environment_id, method, params).await;
            return;
        }
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
            self.record_reply(pool, thread_id, params).await;
        }
        if method == "turn/diff/updated" {
            record_turn_diff(pool, thread_id, params).await;
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
        // The clock needs no one's answer: codex asks what time it is, and the gateway's clock is the answer.
        if request.method == "currentTime/read" {
            let now = chrono::Utc::now().timestamp();
            let _ = connection.respond(request.id.clone(), json!({ "currentTimeAt": now })).await;
            return;
        }
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
    pub async fn pump(self: Arc<Self>, pool: PgPool, connection: Arc<RpcConnection>, environment_id: Uuid) {
        let mut notifications = connection.subscribe();
        let mut requests = connection.subscribe_server_requests();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                event = notifications.recv() => match event {
                    Ok(event) => self.forward_event(&pool, environment_id, &event.method, &event.params).await,
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
        "mcpServer/elicitation/request" => Some(json!({ "action": "decline" })),
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
impl Hub {
    /// Appends a completed message to its turn. The turn learns its codex turn id a moment after the
    /// first events can arrive; a message that finds no turn waits in `unplaced_replies` until the
    /// turn is linked, so it is neither lost nor written twice (see `link_replies`).
    async fn record_reply(&self, pool: &PgPool, thread_id: Uuid, params: &Value) {
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
        let mut unplaced = self.unplaced_replies.lock().await;
        match append_reply(pool, thread_id, codex_turn_id, &text).await {
            Ok(0) => unplaced
                .entry(codex_turn_id.to_string())
                .or_insert_with(|| (thread_id, Vec::new()))
                .1
                .push(text),
            Ok(_) => {}
            Err(err) => tracing::warn!(%err, "could not record assistant reply"),
        }
    }

    /// Writes the replies that arrived before their turn was linked to `codex_turn_id`. Called once
    /// the turn's codex turn id is stored.
    pub async fn link_replies(&self, pool: &PgPool, codex_turn_id: &str) {
        let mut unplaced = self.unplaced_replies.lock().await;
        let Some((thread_id, texts)) = unplaced.remove(codex_turn_id) else {
            return;
        };
        for text in texts {
            if let Err(err) = append_reply(pool, thread_id, codex_turn_id, &text).await {
                tracing::warn!(%err, "could not record assistant reply");
            }
        }
    }
}

/// Appends `text` to the reply of the turn with this codex turn id; returns how many turns matched.
async fn append_reply(pool: &PgPool, thread_id: Uuid, codex_turn_id: &str, text: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE turns SET reply = CASE WHEN reply IS NULL THEN $3
                                       ELSE left(reply || E'\n\n' || $3, 200000) END
         WHERE thread_id = $1 AND codex_turn_id = $2",
    )
    .bind(thread_id)
    .bind(codex_turn_id)
    .bind(text)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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

/// Largest diff kept per turn, in bytes. The end of a very long diff is what is kept.
const TURN_DIFF_LIMIT: usize = 256 * 1024;

/// Keeps the latest diff of everything a turn changed. Codex sends the whole turn's diff each time,
/// so a new report replaces the one before.
async fn record_turn_diff(pool: &PgPool, thread_id: Uuid, params: &Value) {
    let (Some(codex_turn_id), Some(diff)) = (
        params.get("turnId").and_then(Value::as_str),
        params.get("diff").and_then(Value::as_str),
    ) else {
        return;
    };
    let result = sqlx::query(
        "INSERT INTO turn_diffs (thread_id, codex_turn_id, diff) VALUES ($1, $2, $3)
         ON CONFLICT (thread_id, codex_turn_id) DO UPDATE SET diff = EXCLUDED.diff, updated_at = now()",
    )
    .bind(thread_id)
    .bind(codex_turn_id)
    .bind(capped(diff, TURN_DIFF_LIMIT))
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(%err, "could not record a turn's diff");
    }
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
