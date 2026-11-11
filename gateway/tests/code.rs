//! Code sessions: linking a repository, a session's checkout, and commit and push.
//! Uses a local bare repository as the "GitHub" remote, through `GATEWAY_GIT_BASE`-style config.

mod common;

use std::{path::PathBuf, process::Command};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::{self, CodeConfig}, runtime::RuntimeConfig};
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

/// Creates `<base>/acme/demo.git` with one commit on `main`.
fn make_remote(base: &PathBuf) {
    let remote = base.join("acme").join("demo.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(Some(&remote), &["init", "--bare", "-q", "-b", "main"]);
    let seed = base.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(Some(&seed), &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(Some(&seed), &["add", "."]);
    git(Some(&seed), &["-c", "user.name=seed", "-c", "user.email=seed@example.test", "commit", "-q", "-m", "init"]);
    git(Some(&seed), &["push", "-q", remote.to_str().unwrap(), "main"]);
}

fn code_config(root: &PathBuf) -> CodeConfig {
    CodeConfig {
        root: root.join("data"),
        git_base: format!("file://{}", root.display()),
        key: Some([7u8; 32]),
        github: Default::default(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn code_session_runs_in_its_checkout_and_commits_to_its_branch(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("code-test-{}", Uuid::new_v4()));
    make_remote(&root);
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(code_config(&root));
    let mock = mock_app_server().await;
    let token = login(&state, "coder@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, Some(&mock.url)).await;

    // The person connects a GitHub account. Linking then uses that account's token, stored encrypted.
    let me = call(&state, "GET", "/api/v1/me", &token, None).await.1;
    let user_id = Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();
    let (ciphertext, nonce) = code::encrypt_token(&state.code, "ghp_supersecret").unwrap();
    sqlx::query("INSERT INTO github_accounts (user_id, login, token_ciphertext, token_nonce) VALUES ($1, 'coder', $2, $3)")
        .bind(user_id)
        .bind(ciphertext)
        .bind(nonce)
        .execute(&pool)
        .await
        .unwrap();

    // Linking clones the repository with that token.
    let (status, repo) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/repos"),
        &token,
        Some(json!({ "full_name": "acme/demo" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    assert_eq!(repo["has_token"], true);
    assert!(!repo.to_string().contains("ghp_supersecret"), "token must not be returned");
    let repo_id = repo["id"].as_str().unwrap().to_string();

    // A bad name is refused before anything is cloned.
    let (status, _) = call(&state, "POST", &format!("/api/v1/workspaces/{ws_id}/repos"), &token, Some(json!({ "full_name": "../etc" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The clone's config must not hold the token.
    let clone = root.join("data").join("repos").join(&repo_id);
    let config = std::fs::read_to_string(clone.join(".git").join("config")).unwrap();
    assert!(!config.contains("ghp_supersecret"));

    // A code chat: its own branch, and a turn that runs in a worktree.
    let environment_id = {
        let (_, thread) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}"), &token, None).await;
        thread["environment_id"].clone()
    };
    let (status, code_chat) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        &token,
        Some(json!({ "title": "Fix it", "environment_id": environment_id, "repo_id": repo_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{code_chat}");
    let code_chat_id = code_chat["id"].as_str().unwrap().to_string();
    let branch = code_chat["work_branch"].as_str().unwrap().to_string();
    assert!(branch.starts_with("baize/"), "{branch}");

    // Committing before any turn has no checkout to commit from.
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/commit"), &token, Some(json!({ "message": "x" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, turn) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/turns"), &token, Some(json!({ "input": "change the readme" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{turn}");
    let done = wait_for_turn(&state, &token, &code_chat_id, 0).await;
    assert_eq!(done["status"], "completed", "{done}");

    let worktree = root.join("data").join("worktrees").join(&code_chat_id);
    assert!(worktree.join(".git").exists(), "the turn should have created the worktree");
    assert_eq!(git(Some(&worktree), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);

    // Nothing changed yet: committing is refused.
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/commit"), &token, Some(json!({ "message": "nothing" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The session edits a file, then commits and pushes.
    std::fs::write(worktree.join("README.md"), "hello from the session\n").unwrap();
    let (status, result) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/commit"), &token, Some(json!({ "message": "Update the readme" }))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["branch"], branch);
    let sha = result["sha"].as_str().unwrap();
    assert_eq!(sha.len(), 40);

    // The remote now has the branch, and main is untouched.
    let remote = root.join("acme").join("demo.git");
    let refs = git(Some(&remote), &["for-each-ref", "--format=%(refname:short) %(objectname)"]);
    assert!(refs.contains(&format!("{branch} {sha}")), "{refs}");
    assert!(refs.contains("main "), "{refs}");
    let main_log = git(Some(&remote), &["log", "-1", "--format=%s", "main"]);
    assert_eq!(main_log, "init");

    // A chat without a repository cannot commit.
    let (status, _) = call(&state, "POST", &format!("/api/v1/threads/{chat_id}/commit"), &token, Some(json!({ "message": "x" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let _ = std::fs::remove_dir_all(&root);
}
