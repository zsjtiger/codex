//! Shared test helpers: a mock codex app-server and HTTP/WebSocket helpers.

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};
use tower::ServiceExt;

use codex_gateway::{AppState, app};

/// Request id the mock uses for its approval requests.
pub const APPROVAL_ID: i64 = 900;
pub const ELICIT_ID: i64 = 901;
pub const CLOCK_ID: i64 = 902;

pub struct MockServer {
    pub url: String,
    /// Methods the gateway has sent, in order.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// Every request the gateway has sent, with its params.
    pub requests: Arc<Mutex<Vec<(String, Value)>>>,
    /// Responses the gateway has sent back to the mock's server requests.
    pub responses: Arc<Mutex<Vec<Value>>>,
    /// Largest number of turns in flight at once.
    pub max_in_flight: Arc<AtomicUsize>,
}

/// Starts a mock app-server. Turn behaviour depends on the input text:
/// `FAIL_START` returns an error, `FAIL_TURN` completes as failed,
/// `APPROVAL` sends an approval request first, and `SLOW` delays completion.
pub async fn mock_app_server() -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let responses = Arc::new(Mutex::new(Vec::new()));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let turn_counter = Arc::new(AtomicU64::new(1));

    let requests_c = Arc::clone(&requests);
    let (seen_c, responses_c, max_c) = (
        Arc::clone(&seen),
        Arc::clone(&responses),
        Arc::clone(&max_in_flight),
    );
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let socket = accept_async(stream).await.unwrap();
            let (mut sink, mut stream) = socket.split();
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                while let Some(frame) = out_rx.recv().await {
                    if sink.send(Message::Text(frame.into())).await.is_err() {
                        break;
                    }
                }
            });

            let requests = Arc::clone(&requests_c);
            let (seen, responses, max_in_flight, in_flight, turn_counter) = (
                Arc::clone(&seen_c),
                Arc::clone(&responses_c),
                Arc::clone(&max_c),
                Arc::clone(&in_flight),
                Arc::clone(&turn_counter),
            );
            tokio::spawn(async move {
                while let Some(Ok(Message::Text(text))) = stream.next().await {
                    let msg: Value = serde_json::from_str(text.as_str()).unwrap();
                    let Some(method) = msg
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    else {
                        // A response from the gateway to one of our server requests.
                        responses.lock().unwrap().push(msg);
                        continue;
                    };
                    seen.lock().unwrap().push(method.clone());
                    requests
                        .lock()
                        .unwrap()
                        .push((method.clone(), msg["params"].clone()));
                    let Some(id) = msg.get("id").cloned() else {
                        continue; // notification such as "initialized"
                    };
                    let reply = |result: Value| json!({ "id": id, "result": result }).to_string();
                    match method.as_str() {
                        "initialize" => out_tx.send(reply(json!({ "userAgent": "mock" }))).unwrap(),
                        "thread/start" => {
                            out_tx
                                .send(reply(json!({ "thread": { "id": "thr_mock" } })))
                                .unwrap();
                            // Report each configured MCP server: URLs containing "ok" connect, others fail.
                            if let Some(servers) =
                                msg["params"]["config"]["mcp_servers"].as_object()
                            {
                                for (name, config) in servers {
                                    let url = config["url"].as_str().unwrap_or_default();
                                    let (status, error) = if url.contains("ok") {
                                        ("ready", Value::Null)
                                    } else {
                                        ("failed", json!("connection refused"))
                                    };
                                    out_tx
                                        .send(
                                            json!({
                                                "method": "mcpServer/startupStatus/updated",
                                                "params": {
                                                    "threadId": "thr_mock",
                                                    "name": name,
                                                    "status": status,
                                                    "error": error,
                                                    "failureReason": null
                                                }
                                            })
                                            .to_string(),
                                        )
                                        .unwrap();
                                }
                            }
                        }
                        "review/start" => {
                            // A review is a turn that finishes with its review as the reply.
                            let turn_id =
                                format!("turn_{}", turn_counter.fetch_add(1, Ordering::Relaxed));
                            out_tx
                                .send(reply(json!({ "reviewThreadId": "thr_mock", "turn": { "id": turn_id } })))
                                .unwrap();
                            let out = out_tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                                out.send(
                                    json!({
                                        "method": "item/completed",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turnId": turn_id,
                                            "item": { "type": "agentMessage", "id": "msg_review", "text": "Review: no problems found." }
                                        }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                                out.send(
                                    json!({
                                        "method": "turn/completed",
                                        "params": { "threadId": "thr_mock", "turn": { "id": turn_id, "status": "completed", "items": [], "error": null } }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                            });
                        }
                        "thread/compact/start" => {
                            // Compaction reports back with a notification, not a turn.
                            out_tx.send(reply(json!({}))).unwrap();
                            out_tx
                                .send(json!({ "method": "thread/compacted", "params": { "threadId": "thr_mock" } }).to_string())
                                .unwrap();
                        }
                        "thread/fork" => {
                            // A fork is a thread of its own, with a new id.
                            let forked = format!("thr_fork_{}", turn_counter.fetch_add(1, Ordering::Relaxed));
                            out_tx
                                .send(reply(json!({ "thread": { "id": forked } })))
                                .unwrap();
                        }
                        "fuzzyFileSearch" => {
                            // Matches a fixed file list; the query is recorded with the request.
                            let query = msg["params"]["query"].as_str().unwrap_or_default().to_string();
                            let files = if query.is_empty() {
                                vec![]
                            } else {
                                vec![json!({ "root": "", "path": "src/cache.rs", "matchType": "file", "fileName": "cache.rs", "score": 90, "indices": null })]
                            };
                            out_tx.send(reply(json!({ "files": files }))).unwrap();
                        }
                        "fs/readDirectory" => {
                            // Lists a fixed tree: the root has a folder, a file and git data; src has one file.
                            let path = msg["params"]["path"].as_str().unwrap_or_default();
                            let entries = if path.ends_with("/src") {
                                json!([{ "fileName": "cache.rs", "isDirectory": false, "isFile": true }])
                            } else {
                                json!([
                                    { "fileName": "README.md", "isDirectory": false, "isFile": true },
                                    { "fileName": "src", "isDirectory": true, "isFile": false },
                                    { "fileName": ".git", "isDirectory": true, "isFile": false }
                                ])
                            };
                            out_tx.send(reply(json!({ "entries": entries }))).unwrap();
                        }
                        "fs/readFile" => {
                            use base64::{Engine as _, engine::general_purpose::STANDARD};
                            let path = msg["params"]["path"].as_str().unwrap_or_default();
                            let bytes: Vec<u8> = if path.ends_with("logo.bin") {
                                vec![0xff, 0xfe, 0x00]
                            } else if path.ends_with("big.txt") {
                                vec![b'a'; 300_000]
                            } else {
                                b"hello\n".to_vec()
                            };
                            out_tx.send(reply(json!({ "dataBase64": STANDARD.encode(bytes) }))).unwrap();
                        }
                        "command/exec" => {
                            // Streams output the way codex does: deltas for the process, then a reply with no stdout.
                            // A command that mentions "fail" exits with 3 and writes to stderr; one that mentions
                            // "forever" runs until it is terminated.
                            use base64::{Engine as _, engine::general_purpose::STANDARD};
                            let process_id = msg["params"]["processId"].as_str().unwrap_or_default().to_string();
                            let command = msg["params"]["command"][2].as_str().unwrap_or_default().to_string();
                            let delta = |stream: &str, text: &str| {
                                json!({ "method": "command/exec/outputDelta", "params": {
                                    "processId": process_id, "stream": stream,
                                    "deltaBase64": STANDARD.encode(text), "capReached": false } })
                                .to_string()
                            };
                            if command.contains("forever") {
                                out_tx.send(delta("stdout", "started\n")).unwrap();
                                let requests = Arc::clone(&requests);
                                let out = out_tx.clone();
                                let id = id.clone();
                                tokio::spawn(async move {
                                    for _ in 0..500 {
                                        if requests.lock().unwrap().iter().any(|(m, _)| m == "command/exec/terminate") {
                                            break;
                                        }
                                        tokio::time::sleep(Duration::from_millis(10)).await;
                                    }
                                    out.send(json!({ "id": id, "result": { "exitCode": -1, "stdout": "", "stderr": "" } }).to_string()).unwrap();
                                });
                            } else if command.contains("fail") {
                                out_tx.send(delta("stderr", "nope\n")).unwrap();
                                out_tx.send(reply(json!({ "exitCode": 3, "stdout": "", "stderr": "" }))).unwrap();
                            } else {
                                out_tx.send(delta("stdout", &format!("ran: {command}\n"))).unwrap();
                                out_tx.send(reply(json!({ "exitCode": 0, "stdout": "", "stderr": "" }))).unwrap();
                            }
                        }
                        "command/exec/terminate" => {
                            out_tx.send(reply(json!({}))).unwrap();
                        }
                        "account/rateLimits/read" => {
                            // The codex account's limits: a five-hour window and a weekly one, with credits.
                            out_tx
                                .send(reply(json!({ "rateLimits": {
                                    "limitName": null,
                                    "planType": "plus",
                                    "primary": { "usedPercent": 42, "windowDurationMins": 300, "resetsAt": 1800000000 },
                                    "secondary": { "usedPercent": 7, "windowDurationMins": 10080, "resetsAt": null },
                                    "credits": { "hasCredits": true, "unlimited": false, "balance": "12.50" },
                                    "rateLimitReachedType": null
                                } })))
                                .unwrap();
                        }
                        "mcpServerStatus/list" => {
                            // Two pages: the first server on page one, the second and a failed one on page two.
                            let (data, next) = match msg["params"]["cursor"].as_str() {
                                None => (
                                    json!([{ "name": "alphaxiv", "runtimeStatus": "connected", "authStatus": "unsupported",
                                             "tools": { "search_papers": {}, "get_paper": {} }, "toolsError": null,
                                             "resources": [], "resourceTemplates": [] }]),
                                    json!("page-2"),
                                ),
                                Some(_) => (
                                    json!([{ "name": "notion", "runtimeStatus": "authenticationRequired", "authStatus": "notLoggedIn",
                                             "tools": {}, "toolsError": "sign in first", "resources": [], "resourceTemplates": [] }]),
                                    Value::Null,
                                ),
                            };
                            out_tx.send(reply(json!({ "data": data, "nextCursor": next }))).unwrap();
                        }
                        "memory/status" => {
                            out_tx.send(reply(json!({ "v2ConsolidatedThreads": 12, "v2Ready": true }))).unwrap();
                        }
                        "skills/list" => {
                            // Two skills in the codex's user folder, one of them switched off, and a folder that failed to read.
                            out_tx
                                .send(reply(json!({ "data": [{
                                    "cwd": "/work/repo",
                                    "skills": [
                                        { "name": "zeta", "description": "Last one.", "path": "/home/x/.codex/gateway-skills/zeta/SKILL.md", "scope": "user", "enabled": true },
                                        { "name": "alpha", "description": "First one.", "path": "/home/x/.codex/skills/alpha/SKILL.md", "scope": "user", "enabled": false }
                                    ],
                                    "errors": [{ "path": "/home/x/.codex/skills/broken/SKILL.md", "message": "missing name" }]
                                }] })))
                                .unwrap();
                        }
                        "plugin/installed" => {
                            // Two marketplaces: one with an installed plugin and an uninstalled one, one that failed to load.
                            out_tx
                                .send(reply(json!({
                                    "marketplaces": [
                                        { "name": "openai-curated", "path": null, "interface": null, "plugins": [
                                            { "id": "docs@openai-curated", "name": "docs", "version": "1.2.0", "installed": true, "enabled": true, "source": {"type": "local", "path": "/p/docs"}, "installPolicy": "AVAILABLE" },
                                            { "id": "slides@openai-curated", "name": "slides", "version": null, "installed": false, "enabled": false, "source": {"type": "local", "path": "/p/slides"}, "installPolicy": "AVAILABLE" }
                                        ] },
                                        { "name": "local", "path": null, "interface": null, "plugins": [
                                            { "id": "notes@local", "name": "notes", "version": null, "installed": true, "enabled": false, "source": {"type": "local", "path": "/p/notes"}, "installPolicy": "AVAILABLE" }
                                        ] }
                                    ],
                                    "marketplaceLoadErrors": [{ "marketplacePath": "/home/x/.codex/plugins/broken", "message": "bad manifest" }]
                                })))
                                .unwrap();
                        }
                        "turn/steer" => {
                            out_tx
                                .send(reply(json!({ "turnId": msg["params"]["expectedTurnId"] })))
                                .unwrap();
                        }
                        "turn/start" => {
                            let input = msg["params"]["input"][0]["text"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            if input.contains("FAIL_START") {
                                out_tx
                                    .send(json!({ "id": id, "error": { "code": -32000, "message": "model unavailable" } }).to_string())
                                    .unwrap();
                                continue;
                            }
                            let turn_id =
                                format!("turn_{}", turn_counter.fetch_add(1, Ordering::Relaxed));
                            let now_in_flight = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                            max_in_flight.fetch_max(now_in_flight, Ordering::SeqCst);
                            out_tx
                                .send(reply(json!({ "turn": { "id": turn_id } })))
                                .unwrap();

                            let out = out_tx.clone();
                            let in_flight = Arc::clone(&in_flight);
                            let responses = Arc::clone(&responses);
                            tokio::spawn(async move {
                                if input.contains("APPROVAL") {
                                    out.send(
                                        json!({
                                            "id": APPROVAL_ID,
                                            "method": "item/commandExecution/requestApproval",
                                            "params": {
                                                "threadId": "thr_mock",
                                                "turnId": turn_id,
                                                "itemId": "item_1",
                                                "startedAtMs": 0
                                            }
                                        })
                                        .to_string(),
                                    )
                                    .unwrap();
                                    wait_for_response(&responses, APPROVAL_ID).await;
                                }
                                if input.contains("ELICIT") {
                                    // An MCP server asks the person for a choice, with a form.
                                    out.send(
                                        json!({
                                            "id": ELICIT_ID,
                                            "method": "mcpServer/elicitation/request",
                                            "params": {
                                                "threadId": "thr_mock",
                                                "turnId": turn_id,
                                                "serverName": "docs",
                                                "mode": "form",
                                                "message": "Which region?",
                                                "requestedSchema": {
                                                    "type": "object",
                                                    "properties": { "region": { "type": "string", "enum": ["eu", "us"] } },
                                                    "required": ["region"]
                                                }
                                            }
                                        })
                                        .to_string(),
                                    )
                                    .unwrap();
                                    wait_for_response(&responses, ELICIT_ID).await;
                                }
                                if input.contains("CLOCK") {
                                    out.send(json!({ "id": CLOCK_ID, "method": "currentTime/read", "params": { "threadId": "thr_mock" } }).to_string()).unwrap();
                                    wait_for_response(&responses, CLOCK_ID).await;
                                }
                                if input.contains("TOOL") || input.contains("EDIT_DOC") || input.contains("SAVE_DOC") || input.contains("SAVE_EMPTY") || input.contains("请把下面的回答整理成一份结构清楚的文档") {
                                    out.send(
                                        json!({
                                            "method": "item/completed",
                                            "params": {
                                                "threadId": "thr_mock",
                                                "turnId": turn_id,
                                                "item": {"type": "webSearch", "id": "ws_1", "query": "alphafold weights", "action": null, "results": []}
                                            }
                                        })
                                        .to_string(),
                                    )
                                    .unwrap();
                                    out.send(
                                        json!({
                                            "method": "item/completed",
                                            "params": {
                                                "threadId": "thr_mock",
                                                "turnId": turn_id,
                                                "item": {"type": "agentMessage", "id": "msg_1", "text": agent_text(&input), "delivery": null, "memoryCitation": null, "phase": null, "questions": null}
                                            }
                                        })
                                        .to_string(),
                                    )
                                    .unwrap();
                                }
                                if input.contains("DIFF") {
                                    // Codex reports the whole turn's diff each time it changes; the second report replaces the first.
                                    for diff in ["--- a/notes.md\n+++ b/notes.md\n+first\n", "--- a/notes.md\n+++ b/notes.md\n+second\n"] {
                                        out.send(
                                            json!({
                                                "method": "turn/diff/updated",
                                                "params": { "threadId": "thr_mock", "turnId": turn_id, "diff": diff }
                                            })
                                            .to_string(),
                                        )
                                        .unwrap();
                                    }
                                }
                                if input.contains("SLOW") {
                                    tokio::time::sleep(Duration::from_millis(400)).await;
                                }
                                let status = if input.contains("FAIL_TURN") {
                                    "failed"
                                } else {
                                    "completed"
                                };
                                let error = if status == "failed" {
                                    json!({ "message": "boom" })
                                } else {
                                    Value::Null
                                };
                                out.send(
                                    json!({
                                        "method": "thread/tokenUsage/updated",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turnId": turn_id,
                                            "tokenUsage": {
                                                "last": { "inputTokens": 10, "outputTokens": 5 },
                                                "total": { "inputTokens": 10, "outputTokens": 5 }
                                            }
                                        }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                                in_flight.fetch_sub(1, Ordering::SeqCst);
                                out.send(
                                    json!({
                                        "method": "turn/completed",
                                        "params": {
                                            "threadId": "thr_mock",
                                            "turn": { "id": turn_id, "status": status, "items": [], "error": error }
                                        }
                                    })
                                    .to_string(),
                                )
                                .unwrap();
                            });
                        }
                        _ => out_tx.send(reply(json!({}))).unwrap(),
                    }
                }
            });
        }
    });

    MockServer {
        url,
        seen,
        requests,
        responses,
        max_in_flight,
    }
}

async fn wait_for_response(responses: &Mutex<Vec<Value>>, id: i64) {
    for _ in 0..500 {
        if responses.lock().unwrap().iter().any(|r| r["id"] == id) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = app(state.clone());
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn login(state: &AppState, email: &str) -> String {
    let (_, body) = call(
        state,
        "POST",
        "/api/v1/auth/sessions",
        "",
        Some(json!({ "email": email })),
    )
    .await;
    body["token"].as_str().unwrap().to_string()
}

/// Creates a workspace, an environment with the given endpoint, and a thread bound to it.
/// Returns `(workspace_id, thread_id)`.
pub async fn setup(state: &AppState, token: &str, endpoint: Option<&str>) -> (String, String) {
    let (_, ws) = call(
        state,
        "POST",
        "/api/v1/workspaces",
        token,
        Some(json!({ "slug": "runtime", "name": "Runtime" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let (_, env) = call(
        state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/environments"),
        token,
        Some(json!({ "name": "env", "endpoint": endpoint })),
    )
    .await;
    let (_, thread) = call(
        state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        token,
        Some(json!({ "environment_id": env["id"] })),
    )
    .await;
    (ws_id, thread["id"].as_str().unwrap().to_string())
}

/// Polls the turn list until the turn at `index` leaves `queued` / `running`.
pub async fn wait_for_turn(state: &AppState, token: &str, thread_id: &str, index: usize) -> Value {
    for _ in 0..400 {
        let (_, turns) = call(
            state,
            "GET",
            &format!("/api/v1/threads/{thread_id}/turns"),
            token,
            None,
        )
        .await;
        if let Some(turn) = turns.as_array().and_then(|t| t.get(index))
            && !matches!(turn["status"].as_str(), Some("queued" | "running"))
        {
            return turn.clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("turn did not finish in time");
}

/// Serves the gateway on a real TCP port, for WebSocket tests. Returns its address.
pub async fn spawn_gateway(state: AppState) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    addr
}

/// Sends a request with a raw body and returns the status, content type and bytes.
pub async fn call_raw(
    state: &AppState,
    method: &str,
    path: &str,
    token: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let router = app(state.clone());
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, content_type, bytes)
}

/// The mock's assistant text: a document edit when the message asks for one, else a fixed reply.
/// `EDIT_DOC <id>` asks for the document with that id to be replaced with the text below.
fn agent_text(input: &str) -> String {
    if let Some(rest) = input.split("EDIT_DOC ").nth(1) {
        let id: String = rest.chars().take_while(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        return format!("Changed it.\n\n````document-edit id={id}\n# 修订\n\n模型改写的内容。\n````");
    }
    if input.contains("SAVE_EMPTY") {
        return "Saved the previous reply.\n\n````document-create title=\"上一条\"\n````".to_string();
    }
    if input.contains("SAVE_DOC") {
        return "Saved it.\n\n````document-create title=\"保存的文档\"\n# 保存的内容\n\n正文一句。\n````".to_string();
    }
    "ignored".to_string()
}

/// A browser's WebSocket to the gateway, as the tests open it.
pub type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Gets a ticket for the session and opens the WebSocket with it.
pub async fn connect(state: &AppState, addr: SocketAddr, token: &str) -> Ws {
    let (status, body) = call(state, "POST", "/api/v1/ws-tickets", token, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    connect_async(format!(
        "ws://{addr}/ws?ticket={}",
        body["ticket"].as_str().unwrap()
    ))
    .await
    .expect("websocket connect")
    .0
}

pub async fn send(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

pub async fn next_json(ws: &mut Ws) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("timed out waiting for a frame")
        .expect("socket closed")
        .expect("socket error");
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

/// Reads frames until one has the given `type`.
pub async fn next_of_type(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let frame = next_json(ws).await;
        if frame["type"] == kind {
            return frame;
        }
    }
}

