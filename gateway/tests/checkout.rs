//! Browsing and reading a code chat's checkout through Codex's file operations. Every path is kept inside the checkout.

mod common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::CodeConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn state_with_checkouts(pool: &PgPool, root: &Path) -> AppState {
    let config = CodeConfig { root: root.join("data"), ..Default::default() };
    AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config)
}

/// A code chat whose checkout is on disk: README.md, src/cache.rs, git data, a binary, a long file, and a
/// link that points out of the checkout.
async fn code_chat(pool: &PgPool, state: &AppState, token: &str, url: &str, root: &Path) -> (String, PathBuf) {
    let (ws_id, chat_id) = setup(state, token, Some(url)).await;
    let repo_id = Uuid::new_v4();
    sqlx::query("INSERT INTO code_repos (id, workspace_id, full_name) VALUES ($1, $2::uuid, 'acme/demo')")
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
    std::fs::write(worktree.join("logo.bin"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(worktree.join("big.txt"), vec![b'a'; 300_000]).unwrap();
    std::fs::write(root.join("secret.txt"), "outside\n").unwrap();
    std::os::unix::fs::symlink(root.join("secret.txt"), worktree.join("link.txt")).unwrap();
    (chat_id, worktree)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_folder_lists_its_entries_folders_first_and_hides_git_data(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("checkout-{}", Uuid::new_v4()));
    let state = state_with_checkouts(&pool, &root);
    let mock = mock_app_server().await;
    let token = login(&state, "tree@example.com").await;
    let (chat_id, worktree) = code_chat(&pool, &state, &token, &mock.url, &root).await;

    let (status, entries) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/list?path="), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    assert_eq!(entries, json!([{ "name": "src", "is_directory": true }, { "name": "README.md", "is_directory": false }]));
    let requests = mock.requests.lock().unwrap().clone();
    let (_, params) = requests.iter().rev().find(|(m, _)| m == "fs/readDirectory").expect("the listing was asked of codex");
    assert_eq!(params["path"], json!(worktree.canonicalize().unwrap().to_string_lossy()));

    let (status, entries) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/list?path=src"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    assert_eq!(entries, json!([{ "name": "cache.rs", "is_directory": false }]));
    let _ = std::fs::remove_dir_all(&root);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_file_is_read_as_text_and_binary_and_long_files_are_cut(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("checkout-{}", Uuid::new_v4()));
    let state = state_with_checkouts(&pool, &root);
    let mock = mock_app_server().await;
    let token = login(&state, "read@example.com").await;
    let (chat_id, _) = code_chat(&pool, &state, &token, &mock.url, &root).await;

    let (status, file) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path=README.md"), &token, None).await;
    assert_eq!(status, StatusCode::OK, "{file}");
    assert_eq!(file["content"], "hello\n");
    assert_eq!(file["binary"], false);
    assert_eq!(file["truncated"], false);
    assert_eq!(file["bytes"], 6);

    let (_, file) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path=logo.bin"), &token, None).await;
    assert_eq!(file["binary"], true);
    assert!(file["content"].is_null());

    let (_, file) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path=big.txt"), &token, None).await;
    assert_eq!(file["truncated"], true);
    assert_eq!(file["bytes"], 300_000);
    assert_eq!(file["content"].as_str().unwrap().len(), 256 * 1024);
    let _ = std::fs::remove_dir_all(&root);
}

#[sqlx::test(migrations = "./migrations")]
async fn paths_that_leave_the_checkout_or_reach_git_data_are_refused(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("checkout-{}", Uuid::new_v4()));
    let state = state_with_checkouts(&pool, &root);
    let mock = mock_app_server().await;
    let token = login(&state, "escape@example.com").await;
    let (chat_id, _) = code_chat(&pool, &state, &token, &mock.url, &root).await;

    for path in ["../secret.txt", "/etc/passwd", "src/../../secret.txt", "src/../README.md", ".git/config", "link.txt"] {
        let (status, body) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path={path}"), &token, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} should be refused: {body}");
        let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/list?path={path}"), &token, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} should be refused when listing");
    }

    // Only files are read, and only folders are listed.
    let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path=src"), &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/list?path=README.md"), &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nothing outside the checkout was asked of codex.
    let requests = mock.requests.lock().unwrap().clone();
    assert!(!requests.iter().any(|(m, p)| m.starts_with("fs/") && p["path"].as_str().unwrap_or_default().ends_with("secret.txt")));
    let _ = std::fs::remove_dir_all(&root);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_chat_without_a_checkout_has_no_files(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("checkout-{}", Uuid::new_v4()));
    let state = state_with_checkouts(&pool, &root);
    let mock = mock_app_server().await;
    let token = login(&state, "plain@example.com").await;
    let (_, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/list?path="), &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&state, "GET", &format!("/api/v1/threads/{chat_id}/files/read?path=README.md"), &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
