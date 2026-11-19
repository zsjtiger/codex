//! Code-session previews: a page of the checkout is screenshotted by the gateway's screenshot command.
//! Needs `node` with Playwright and Chromium; without them the test says so and passes.

mod common;

use std::{path::PathBuf, process::Command};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::CodeConfig, github::GitHubConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn git(dir: Option<&PathBuf>, args: &[&str]) {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out = command.output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn make_remote(root: &PathBuf) {
    let remote = root.join("acme").join("demo.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(Some(&remote), &["init", "--bare", "-q", "-b", "main"]);
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(Some(&seed), &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(Some(&seed), &["add", "."]);
    git(Some(&seed), &["-c", "user.name=seed", "-c", "user.email=seed@example.test", "commit", "-q", "-m", "init"]);
    git(Some(&seed), &["push", "-q", remote.to_str().unwrap(), "main"]);
}

fn node_present() -> bool {
    Command::new("node").arg("--version").output().is_ok_and(|out| out.status.success())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_checkout_page_is_screenshotted_and_outside_paths_are_refused(pool: PgPool) {
    if !node_present() {
        eprintln!("skipping: node is not installed");
        return;
    }
    let root = std::env::temp_dir().join(format!("preview-test-{}", Uuid::new_v4()));
    make_remote(&root);
    let mock = mock_app_server().await;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools").join("screenshot.cjs");
    let config = CodeConfig {
        root: root.join("data"),
        git_base: format!("file://{}", root.display()),
        key: Some([6u8; 32]),
        github: GitHubConfig::default(),
        allow_file_remotes: true,
        screenshot_cmd: Some(vec!["node".to_string(), script.display().to_string()]),
    };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let token = login(&state, "preview@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    let environment_id = call(&state, "GET", &format!("/api/v1/threads/{chat_id}"), &token, None).await.1["environment_id"].clone();

    // A ready file repository, linked through its address.
    let url = format!("file://{}/acme/demo.git", root.display());
    let (status, repo) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/repos"), &token, Some(json!({ "url": url }))).await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    let repo_id = repo["id"].as_str().unwrap().to_string();

    let (status, code_chat) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        &token,
        Some(json!({ "title": "Preview", "environment_id": environment_id, "repo_id": repo_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{code_chat}");
    let code_chat_id = code_chat["id"].as_str().unwrap().to_string();
    let preview_path = format!("/api/v1/threads/{code_chat_id}/preview");

    // Before any turn there is no checkout to show.
    let body = json!({ "path": "index.html" }).to_string().into_bytes();
    let (status, _, _) = call_raw(&state, "POST", &preview_path, &token, "application/json", body.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The first turn creates the checkout; then a page is written into it.
    call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/turns"), &token, Some(json!({ "input": "start" }))).await;
    let mut done = false;
    for _ in 0..400 {
        let turns = call(&state, "GET", &format!("/api/v1/threads/{code_chat_id}/turns"), &token, None).await.1;
        if turns[0]["status"] == "completed" {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(done, "the first turn finishes");
    let checkout = root.join("data").join("worktrees").join(&code_chat_id);
    std::fs::create_dir_all(checkout.join("site")).unwrap();
    std::fs::write(
        checkout.join("site").join("index.html"),
        "<!doctype html><title>Preview</title><h1 style=\"font-family:sans-serif\">Hello from the checkout</h1>",
    )
    .unwrap();
    std::fs::write(checkout.join("notes.txt"), "not a page").unwrap();
    std::fs::write(root.join("secret.html"), "outside the checkout").unwrap();

    // A page in the checkout comes back as a PNG.
    let body = json!({ "path": "site/index.html" }).to_string().into_bytes();
    let (status, content_type, bytes) = call_raw(&state, "POST", &preview_path, &token, "application/json", body).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
    assert_eq!(content_type.as_deref(), Some("image/png"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "the body is a PNG");

    // Paths that leave the checkout, non-HTML files and missing files are refused.
    for (bad, message) in [
        ("../secret.html", "inside the checkout"),
        ("site/../../secret.html", "inside the checkout"),
        ("notes.txt", ".html"),
        ("missing.html", "no such file"),
    ] {
        let body = json!({ "path": bad }).to_string().into_bytes();
        let (status, _, bytes) = call_raw(&state, "POST", &preview_path, &token, "application/json", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert!(String::from_utf8_lossy(&bytes).contains(message), "{bad}: {}", String::from_utf8_lossy(&bytes));
    }

    // Without the screenshot command, previews are off.
    let off = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(CodeConfig {
        root: root.join("data"),
        git_base: format!("file://{}", root.display()),
        key: Some([6u8; 32]),
        github: GitHubConfig::default(),
        allow_file_remotes: true,
        screenshot_cmd: None,
    });
    let body = json!({ "path": "site/index.html" }).to_string().into_bytes();
    let (status, _, bytes) = call_raw(&off, "POST", &preview_path, &token, "application/json", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&bytes).contains("GATEWAY_SCREENSHOT_CMD"));

    let _ = std::fs::remove_dir_all(&root);
}
