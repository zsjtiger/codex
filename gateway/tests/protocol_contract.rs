//! Contract tests: what the gateway sends to codex, and what the mock sends back, must match codex's own
//! protocol schema (`codex-rs/app-server-protocol/schema/json`). The other tests check the gateway's behaviour;
//! these check that its messages are the ones codex defines, so a mistyped field is caught here rather than
//! only by the mock accepting it.

mod common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::CodeConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

/// The protocol's JSON schema bundle, from the codex-rs checkout in this repository.
fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.schemas.json")
}

/// A validator for one of the protocol's message unions (`ClientRequest`, `ServerNotification`, ...). Each union
/// lists every message by its `method`, so a message is valid when it matches exactly one of them.
fn union_validator(union: &str) -> jsonschema::Validator {
    let document: Value = serde_json::from_str(&std::fs::read_to_string(schema_path()).expect("the protocol schema is in the repository")).unwrap();
    // The protocol's schema is draft-07; the wrapper must say so, or `$ref` is read under newer rules.
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": document["definitions"],
        "$ref": format!("#/definitions/{union}"),
    });
    jsonschema::validator_for(&schema).expect("the protocol schema compiles")
}

/// Methods the gateway uses that the bundled schema does not describe: the experimental ones, and the v1
/// `gitDiffToRemote`. They are listed rather than skipped silently, so the list is updated when the schema is.
const NOT_IN_SCHEMA: &[&str] = &[
    "gitDiffToRemote",
    "memory/reset",
    "memory/status",
    "thread/backgroundTerminals/clean",
    "thread/backgroundTerminals/list",
    "thread/backgroundTerminals/terminate",
    "thread/prediction/request",
];

/// The methods a union lists, by their `method` name.
fn methods_of(union: &str) -> std::collections::BTreeSet<String> {
    let document: Value = serde_json::from_str(&std::fs::read_to_string(schema_path()).unwrap()).unwrap();
    document["definitions"][union]["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variant| variant["properties"]["method"]["enum"][0].as_str().map(str::to_string))
        .collect()
}

/// What is wrong with a message, one line per problem; empty when the message matches.
fn problems(validator: &jsonschema::Validator, message: &Value) -> Vec<String> {
    validator.iter_errors(message).map(|error| error.to_string()).collect()
}

/// A code chat whose checkout is on disk, as the checkout tests make them.
async fn code_chat(pool: &PgPool, state: &AppState, token: &str, url: &str, root: &Path) -> String {
    let (ws_id, chat_id) = setup(state, token, Some(url)).await;
    let repo_id = Uuid::new_v4();
    sqlx::query("INSERT INTO code_repos (id, workspace_id, full_name) VALUES ($1, $2::uuid, 'acme/contract')")
        .bind(repo_id)
        .bind(&ws_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE threads SET repo_id = $2 WHERE id = $1::uuid").bind(&chat_id).bind(repo_id).execute(pool).await.unwrap();
    let worktree = root.join("data").join("worktrees").join(&chat_id);
    std::fs::create_dir_all(worktree.join(".git")).unwrap();
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::write(worktree.join("src").join("cache.rs"), "fn main() {}\n").unwrap();
    std::fs::write(worktree.join("README.md"), "hello\n").unwrap();
    chat_id
}

/// Waits until codex has been asked to run a command with this process id.
async fn wait_for_process(mock: &MockServer, process_id: &str) {
    for _ in 0..500 {
        if mock.requests.lock().unwrap().iter().any(|(m, p)| m == "command/exec" && p["processId"] == process_id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the command never started");
}

/// Drives the gateway's main codex-facing features once each, so the messages they send are recorded.
async fn exercise_features(state: &AppState, pool: &PgPool, token: &str, mock: &MockServer, root: &Path) {
    let chat = code_chat(pool, state, token, &mock.url, root).await;
    let api = |method: &'static str, path: String, body: Option<Value>| async move {
        let (status, body) = call(state, method, &format!("/api/v1{path}"), token, body).await;
        (status, body)
    };

    // A first turn starts codex's thread; a second and a third give turns to act on.
    post_turn(state, token, &chat, "first").await;
    wait_for_turn(state, token, &chat, 0).await;
    post_turn(state, token, &chat, "second").await;
    wait_for_turn(state, token, &chat, 1).await;
    let (_, turns) = api("GET", format!("/threads/{chat}/turns"), None).await;
    let first = turns[0]["id"].as_str().unwrap().to_string();

    // Chat-level codex features.
    api("POST", format!("/threads/{chat}/review"), Some(json!({ "target": { "type": "uncommittedChanges" } }))).await;
    wait_for_turn(state, token, &chat, 2).await;
    api("POST", format!("/threads/{chat}/compact"), None).await;
    wait_for_turn(state, token, &chat, 3).await;
    post_turn(state, token, &chat, "SLOW long").await;
    api("POST", format!("/threads/{chat}/steer"), Some(json!({ "input": "also cover the edges" }))).await;
    let (_, running_turns) = api("GET", format!("/threads/{chat}/turns"), None).await;
    let running_turn = running_turns[4]["id"].as_str().unwrap().to_string();
    api("POST", format!("/turns/{running_turn}/interrupt"), None).await;
    wait_for_turn(state, token, &chat, 4).await;
    api("POST", format!("/turns/{first}/prediction"), None).await;
    api("PUT", format!("/threads/{chat}/goal"), Some(json!({ "objective": "ship it", "status": "paused", "token_budget": 900 }))).await;
    api("GET", format!("/threads/{chat}/goal"), None).await;
    api("DELETE", format!("/threads/{chat}/goal"), None).await;
    api("GET", format!("/threads/{chat}/background-terminals"), None).await;
    api("POST", format!("/threads/{chat}/background-terminals/clean"), None).await;
    api("POST", format!("/threads/{chat}/background-terminals/p1/terminate"), None).await;
    api("GET", format!("/threads/{chat}/workspace-diff"), None).await;
    api("POST", format!("/turns/{first}/revert"), None).await;
    // Forking is for plain chats; a code chat cannot be forked.
    let workspace = state_workspace(pool, &chat).await;
    let (_, envs) = api("GET", format!("/workspaces/{workspace}/environments"), None).await;
    let (_, plain) = api("POST", format!("/workspaces/{workspace}/threads"), Some(json!({ "title": "plain", "environment_id": envs[0]["id"] }))).await;
    let plain = plain["id"].as_str().unwrap().to_string();
    post_turn(state, token, &plain, "plain first").await;
    wait_for_turn(state, token, &plain, 0).await;
    let (_, plain_turns) = api("GET", format!("/threads/{plain}/turns"), None).await;
    let plain_turn = plain_turns[0]["id"].as_str().unwrap().to_string();
    api("POST", format!("/threads/{plain}/fork"), Some(json!({ "turn_id": plain_turn }))).await;

    // Command runs: plain, interactive, and in a terminal.
    let pid = Uuid::new_v4().to_string();
    api("POST", format!("/threads/{chat}/run"), Some(json!({ "command": "echo hi", "process_id": pid }))).await;
    // An interactive command ends only when its input is closed, so it runs on its own task while it is driven.
    let interactive = Uuid::new_v4().to_string();
    let (run_state, run_token, run_chat, run_pid) = (state.clone(), token.to_string(), chat.clone(), interactive.clone());
    let running = tokio::spawn(async move {
        call(&run_state, "POST", &format!("/api/v1/threads/{run_chat}/run"), &run_token, Some(json!({ "command": "prompt", "process_id": run_pid, "interactive": true, "tty": true }))).await
    });
    wait_for_process(mock, &interactive).await;
    api("POST", format!("/threads/{chat}/run/{interactive}/input"), Some(json!({ "text": "yes\n", "close_stdin": true }))).await;
    api("POST", format!("/threads/{chat}/run/{interactive}/resize"), Some(json!({ "rows": 40, "cols": 120 }))).await;
    running.await.unwrap();
    api("POST", format!("/threads/{chat}/run/{pid}/stop"), None).await;

    // Files in the checkout.
    api("GET", format!("/threads/{chat}/files?q=cache"), None).await;
    api("GET", format!("/threads/{chat}/files/list?path=src"), None).await;
    api("GET", format!("/threads/{chat}/files/read?path=README.md"), None).await;
    api("PUT", format!("/threads/{chat}/files/content"), Some(json!({ "path": "notes.md", "content": "n\n" }))).await;
    api("POST", format!("/threads/{chat}/files/folder"), Some(json!({ "path": "docs" }))).await;
    api("POST", format!("/threads/{chat}/files/move"), Some(json!({ "from": "notes.md", "to": "docs/notes.md" }))).await;
    api("DELETE", format!("/threads/{chat}/files?path=docs"), None).await;
    let (_, watch) = api("POST", format!("/threads/{chat}/files/watch"), None).await;
    if let Some(watch_id) = watch["watch_id"].as_str() {
        api("DELETE", format!("/threads/{chat}/files/watch/{watch_id}"), None).await;
    }

    // Environment-wide codex features.
    let (_, envs) = api("GET", format!("/workspaces/{}/environments", state_workspace(pool, &chat).await), None).await;
    let env = envs[0]["id"].as_str().unwrap().to_string();
    api("GET", format!("/environments/{env}/models"), None).await;
    api("GET", format!("/environments/{env}/rate-limits"), None).await;
    api("GET", format!("/environments/{env}/mcp-servers"), None).await;
    api("GET", format!("/environments/{env}/codex-skills"), None).await;
    api("GET", format!("/environments/{env}/codex-plugins"), None).await;
    api("GET", format!("/environments/{env}/memory"), None).await;
    api("POST", format!("/environments/{env}/memory/reset"), None).await;
    api("GET", format!("/threads/{chat}/diffs"), None).await;
    let _ = StatusCode::OK;
}

async fn state_workspace(pool: &PgPool, chat: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT workspace_id::text FROM threads WHERE id = $1::uuid").bind(chat).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn every_request_the_gateway_sends_codex_matches_the_protocol(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("contract-{}", Uuid::new_v4()));
    let config = CodeConfig { root: root.join("data"), ..Default::default() };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let mock = mock_app_server().await;
    let token = login(&state, "contract@example.com").await;
    exercise_features(&state, &pool, &token, &mock, &root).await;

    let requests = mock.requests.lock().unwrap().clone();
    let requests_validator = union_validator("ClientRequest");
    let notices_validator = union_validator("ClientNotification");
    let known = methods_of("ClientRequest");
    let mut wrong = Vec::new();
    let mut methods = std::collections::BTreeSet::new();
    let mut uncovered = std::collections::BTreeSet::new();
    for (method, params) in &requests {
        methods.insert(method.clone());
        if method != "initialized" && !known.contains(method) {
            uncovered.insert(method.clone());
            continue;
        }
        let found = if method == "initialized" {
            problems(&notices_validator, &json!({ "method": method, "params": params }))
        } else {
            problems(&requests_validator, &json!({ "id": 1, "method": method, "params": params }))
        };
        if !found.is_empty() {
            wrong.push(format!("{method}: {}", found.join("; ")));
        }
    }
    println!("methods the gateway sent: {}", methods.into_iter().collect::<Vec<_>>().join(", "));
    let _ = std::fs::remove_dir_all(&root);
    let expected: std::collections::BTreeSet<String> = NOT_IN_SCHEMA.iter().map(|m| m.to_string()).collect();
    assert_eq!(uncovered, expected, "the methods the schema does not describe changed; update NOT_IN_SCHEMA");
    assert!(wrong.is_empty(), "requests that do not match the protocol:\n{}", wrong.join("\n"));
}

#[sqlx::test(migrations = "./migrations")]
async fn every_message_the_mock_sends_matches_the_protocol(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("contract-{}", Uuid::new_v4()));
    let config = CodeConfig { root: root.join("data"), ..Default::default() };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let mock = mock_app_server().await;
    let token = login(&state, "contract-frames@example.com").await;
    exercise_features(&state, &pool, &token, &mock, &root).await;

    let sent = mock.sent.lock().unwrap().clone();
    let notifications = union_validator("ServerNotification");
    let requests = union_validator("ServerRequest");
    let known: std::collections::BTreeSet<String> = methods_of("ServerNotification").union(&methods_of("ServerRequest")).cloned().collect();
    let mut wrong = Vec::new();
    let mut checked = 0;
    for frame in &sent {
        // Responses to the gateway's requests have no method; the messages codex starts do.
        let Some(method) = frame.get("method").and_then(Value::as_str) else { continue };
        if !known.contains(method) {
            wrong.push(format!("{method}: not a message the protocol defines"));
            continue;
        }
        checked += 1;
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        let found = if frame.get("id").is_some() {
            problems(&requests, &json!({ "id": frame["id"], "method": method, "params": params }))
        } else {
            problems(&notifications, &json!({ "method": method, "params": params }))
        };
        if !found.is_empty() {
            wrong.push(format!("{method}: {}", found.join("; ")));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(checked > 0, "the mock sent messages to check");
    assert!(wrong.is_empty(), "messages that do not match the protocol:\n{}", wrong.join("\n"));
}

/// The harness itself: messages that are wrong must be reported, or the tests above prove nothing.
#[test]
fn the_schema_check_rejects_messages_codex_would_refuse() {
    let requests = union_validator("ClientRequest");
    let notifications = union_validator("ClientNotification");
    let notifications_server = union_validator("ServerNotification");
    // An unknown method.
    assert!(!problems(&requests, &json!({ "id": 1, "method": "thread/nonexistent", "params": {} })).is_empty());
    // A required field missing (a goal needs its thread).
    assert!(!problems(&requests, &json!({ "id": 1, "method": "thread/goal/set", "params": { "objective": "x" } })).is_empty());
    // A misspelled field in place of the required one.
    assert!(!problems(&requests, &json!({ "id": 1, "method": "command/exec/resize", "params": { "process_id": "p", "size": { "rows": 1, "cols": 1 } } })).is_empty());
    // The wrong type for a number.
    assert!(!problems(&requests, &json!({ "id": 1, "method": "command/exec/resize", "params": { "processId": "p", "size": { "rows": "tall", "cols": 1 } } })).is_empty());
    // A notification sent as a request, or a request sent as a notification.
    assert!(!problems(&notifications, &json!({ "method": "thread/goal/get", "params": {} })).is_empty());
    assert!(!problems(&notifications_server, &json!({ "method": "turn/completed", "params": {} })).is_empty());
    // And a message that is right is accepted.
    assert_eq!(
        problems(&requests, &json!({ "id": 1, "method": "thread/goal/get", "params": { "threadId": "thr" } })),
        Vec::<String>::new()
    );
}

/// The response type each method answers with, by its name in the schema. Some names do not follow the method's
/// (`account/rateLimits/read` answers `GetAccountRateLimitsResponse`), so the table is written out. A test below
/// checks that every name exists in the schema.
const RESPONSE_TYPES: &[(&str, &str)] = &[
    ("initialize", "InitializeResponse"),
    ("thread/start", "ThreadStartResponse"),
    ("turn/start", "TurnStartResponse"),
    ("thread/resume", "ThreadResumeResponse"),
    ("thread/fork", "ThreadForkResponse"),
    ("turn/steer", "TurnSteerResponse"),
    ("turn/interrupt", "TurnInterruptResponse"),
    ("review/start", "ReviewStartResponse"),
    ("thread/compact/start", "ThreadCompactStartResponse"),
    ("thread/goal/get", "ThreadGoalGetResponse"),
    ("thread/goal/set", "ThreadGoalSetResponse"),
    ("thread/goal/clear", "ThreadGoalClearResponse"),
    ("thread/revert", "ThreadRevertResponse"),
    ("command/exec", "CommandExecResponse"),
    ("command/exec/write", "CommandExecWriteResponse"),
    ("command/exec/resize", "CommandExecResizeResponse"),
    ("fs/readFile", "FsReadFileResponse"),
    ("fs/writeFile", "FsWriteFileResponse"),
    ("fs/createDirectory", "FsCreateDirectoryResponse"),
    ("fs/remove", "FsRemoveResponse"),
    ("fs/copy", "FsCopyResponse"),
    ("fs/readDirectory", "FsReadDirectoryResponse"),
    ("fs/watch", "FsWatchResponse"),
    ("fs/unwatch", "FsUnwatchResponse"),
    ("fuzzyFileSearch", "FuzzyFileSearchResponse"),
    ("config/read", "ConfigReadResponse"),
    ("model/list", "ModelListResponse"),
    ("account/rateLimits/read", "GetAccountRateLimitsResponse"),
    ("mcpServerStatus/list", "ListMcpServerStatusResponse"),
    ("skills/list", "SkillsListResponse"),
    ("plugin/installed", "PluginInstalledResponse"),
];

/// Removes every `required` list from a schema, at every depth.
fn without_required(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("required");
            for child in map.values_mut() {
                without_required(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(without_required),
        _ => {}
    }
}

/// A validator for one response type, by its name in the schema, found under `v2` or at the top.
///
/// The mock answers with the fields the gateway reads, not every field codex sends, so presence is not checked
/// here: `required` is dropped. What the mock does send must still have the right types and values, which is
/// what a mistyped answer breaks. Presence of the fields the gateway reads is covered by the gateway's own tests.
fn response_validator(name: &str) -> Option<jsonschema::Validator> {
    let document: Value = serde_json::from_str(&std::fs::read_to_string(schema_path()).unwrap()).unwrap();
    let mut definitions = document["definitions"].clone();
    let path = if definitions["v2"].get(name).is_some() {
        format!("v2/{name}")
    } else if definitions.get(name).is_some() {
        name.to_string()
    } else {
        return None;
    };
    without_required(&mut definitions);
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": definitions,
        "$ref": format!("#/definitions/{path}"),
    });
    Some(jsonschema::validator_for(&schema).expect("the response schema compiles"))
}

#[sqlx::test(migrations = "./migrations")]
async fn every_answer_the_mock_gives_matches_the_protocol(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("contract-{}", Uuid::new_v4()));
    let config = CodeConfig { root: root.join("data"), ..Default::default() };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let mock = mock_app_server().await;
    let token = login(&state, "contract-answers@example.com").await;
    exercise_features(&state, &pool, &token, &mock, &root).await;

    let answers = mock.answers.lock().unwrap().clone();
    let table: std::collections::BTreeMap<&str, &str> = RESPONSE_TYPES.iter().copied().collect();
    let mut wrong = Vec::new();
    let mut checked = 0;
    let mut validators = std::collections::HashMap::new();
    for (method, outcome) in &answers {
        // An error answer is not a result; the mock gives those on purpose for some requests.
        if outcome.get("code").is_some() && outcome.get("message").is_some() {
            continue;
        }
        let Some(name) = table.get(method.as_str()) else {
            if NOT_IN_SCHEMA.contains(&method.as_str()) {
                continue;
            }
            wrong.push(format!("{method}: no response type is listed for it in RESPONSE_TYPES"));
            continue;
        };
        let validator = validators.entry(*name).or_insert_with(|| response_validator(name));
        let Some(validator) = validator else {
            wrong.push(format!("{method}: {name} is not in the schema"));
            continue;
        };
        checked += 1;
        let found = problems(validator, outcome);
        if !found.is_empty() {
            wrong.push(format!("{method} answered with {outcome}: {}", found.join("; ")));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(checked > 0, "the mock answered requests to check");
    assert!(wrong.is_empty(), "answers that do not match the protocol:\n{}", wrong.join("\n"));
}

/// The response check itself: a wrong answer must be reported.
#[test]
fn the_response_check_rejects_answers_codex_would_not_send() {
    let goal = response_validator("ThreadGoalGetResponse").expect("the type is in the schema");
    // A goal whose objective is a number is not a goal.
    assert!(!problems(&goal, &json!({ "goal": { "objective": 5 } })).is_empty());
    // An answer that is right is accepted.
    assert_eq!(problems(&goal, &json!({ "goal": null })), Vec::<String>::new());
    // Every type in the table must exist in the schema.
    for (method, name) in RESPONSE_TYPES {
        assert!(response_validator(name).is_some(), "{method} answers with {name}, which the schema does not define");
    }
}

