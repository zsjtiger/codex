//! Repositories added by git address: the probe, the refusals, linking with a token, and a session that
//! commits to its branch on such a remote. A local bare repository stands in for a git host.

mod common;

use std::{path::PathBuf, process::Command};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::CodeConfig, github::GitHubConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn git(dir: Option<&PathBuf>, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out = command.output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `<root>/team/app.git` with `main` and a `dev` branch.
fn make_remote(root: &PathBuf) -> PathBuf {
    let remote = root.join("team").join("app.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(Some(&remote), &["init", "--bare", "-q", "-b", "main"]);
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(Some(&seed), &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(Some(&seed), &["add", "."]);
    git(Some(&seed), &["-c", "user.name=seed", "-c", "user.email=seed@example.test", "commit", "-q", "-m", "init"]);
    git(Some(&seed), &["push", "-q", remote.to_str().unwrap(), "main"]);
    git(Some(&seed), &["push", "-q", remote.to_str().unwrap(), "main:dev"]);
    remote
}

fn config(root: &PathBuf, allow_file: bool) -> CodeConfig {
    CodeConfig {
        root: root.join("data"),
        git_base: "https://github.com".to_string(),
        key: Some([5u8; 32]),
        github: GitHubConfig::default(),
        allow_file_remotes: allow_file,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_git_remote_is_probed_linked_and_committed_to_by_address(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("git-remote-test-{}", Uuid::new_v4()));
    make_remote(&root);
    let url = format!("file://{}/team/app.git", root.display());
    let mock = mock_app_server().await;

    // Without file remotes enabled, a local address is refused, as it is in production.
    let refused = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config(&root, false));
    let token = login(&refused, "remote@example.com").await;
    let (ws_id, chat_id) = setup(&refused, &token, Some(&mock.url)).await;
    let (status, body) = call(&refused, "POST", &format!("/api/v1/workspaces/{ws_id}/repos/probe"), &token, Some(json!({ "url": url }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("https://"), "{body}");

    // With them enabled (as the tests do), the probe reads the remote's branches and default.
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config(&root, true));
    let (status, probe) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/repos/probe"), &token, Some(json!({ "url": url }))).await;
    assert_eq!(status, StatusCode::OK, "{probe}");
    assert_eq!(probe["name"], "team/app");
    assert_eq!(probe["default_branch"], "main");
    assert_eq!(probe["branches"], json!(["dev", "main"]));

    // Addresses that carry credentials, or that are not https, are refused before git runs.
    for bad in ["http://example.com/team/app.git", "https://user:secret@example.com/team/app.git"] {
        let (status, _) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/repos/probe"), &token, Some(json!({ "url": bad }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    // Linking stores the address and the token encrypted; the token is never returned.
    let (status, repo) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/repos"),
        &token,
        Some(json!({ "url": url, "token": "glpat-secret", "default_branch": "main" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    assert_eq!(repo["by_url"], true);
    assert_eq!(repo["has_token"], true);
    assert!(!repo.to_string().contains("glpat-secret"));
    let repo_id = repo["id"].as_str().unwrap().to_string();
    let clone_config = std::fs::read_to_string(root.join("data").join("repos").join(&repo_id).join(".git").join("config")).unwrap();
    assert!(!clone_config.contains("glpat-secret"), "the token stays out of the clone's config");

    // A session on the repository commits its change to its own branch, on the remote by address.
    let environment_id = call(&state, "GET", &format!("/api/v1/threads/{chat_id}"), &token, None).await.1["environment_id"].clone();
    let (_, code_chat) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        &token,
        Some(json!({ "title": "Remote work", "environment_id": environment_id, "repo_id": repo_id })),
    )
    .await;
    let code_chat_id = code_chat["id"].as_str().unwrap().to_string();
    let branch = code_chat["work_branch"].as_str().unwrap().to_string();
    call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/turns"), &token, Some(json!({ "input": "start" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &code_chat_id, 0).await["status"], "completed");
    let worktree = root.join("data").join("worktrees").join(&code_chat_id);
    assert_eq!(git(Some(&worktree), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);

    std::fs::write(worktree.join("README.md"), "changed through the address\n").unwrap();
    let (status, result) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/commit"), &token, Some(json!({ "message": "Change via address" }))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let remote = root.join("team").join("app.git");
    assert_eq!(git(Some(&remote), &["log", "-1", "--format=%s", &branch]), "Change via address");
    assert_eq!(git(Some(&remote), &["log", "-1", "--format=%s", "main"]), "init", "main is untouched");

    let _ = std::fs::remove_dir_all(&root);
}
