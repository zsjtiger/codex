//! Connecting GitHub: the OAuth round trip, the repository and branch lists, and linking a repository
//! with the connected account. GitHub is a `wiremock` server; git clones a local bare repository.

mod common;

use std::{path::PathBuf, process::Command};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use codex_gateway::{AppState, app, code::CodeConfig, github::GitHubConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn git(dir: Option<&PathBuf>, args: &[&str]) {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out = command.output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// `<root>/acme/demo.git` with one commit on `main`; the local stand-in for github.com.
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

async fn github_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "gho_test_token" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "login": "octo" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "full_name": "acme/demo", "default_branch": "main", "private": false },
            { "full_name": "acme/other", "default_branch": "trunk", "private": true },
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/demo/branches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "name": "main" }, { "name": "dev" }])))
        .mount(&server)
        .await;
    server
}

fn state_for(pool: PgPool, root: &PathBuf, github: GitHubConfig) -> AppState {
    let code = CodeConfig {
        root: root.join("data"),
        git_base: format!("file://{}", root.display()),
        key: Some([9u8; 32]),
        github,
        allow_file_remotes: false,
    };
    AppState::new(pool, true, RuntimeConfig::default()).with_code(code)
}

/// Sends a request with no session, as GitHub's browser redirect does.
async fn browse(state: &AppState, uri: &str) -> (StatusCode, Option<String>) {
    let response = app(state.clone())
        .oneshot(Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    (response.status(), location)
}

#[sqlx::test(migrations = "./migrations")]
async fn connect_list_and_link_with_the_connected_account(pool: PgPool) {
    // The public address the callback is built from, and the web app the flow returns to.
    unsafe { std::env::set_var("GATEWAY_PUBLIC_URL", "http://gateway.test") };
    let root = std::env::temp_dir().join(format!("github-test-{}", Uuid::new_v4()));
    make_remote(&root);
    let mock = github_mock().await;
    let github = GitHubConfig {
        client_id: Some("test-client".to_string()),
        client_secret: Some("test-secret".to_string()),
        oauth_base: format!("{}/login/oauth", mock.uri()),
        api_base: mock.uri(),
    };
    let state = state_for(pool.clone(), &root, github);
    let token = login(&state, "octo-owner@example.com").await;

    // Connecting returns an authorize address with a state that belongs to this person.
    let (status, connect) = call(&state, "POST", "/api/v1/github/connect", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{connect}");
    let url = connect["url"].as_str().unwrap().to_string();
    assert!(url.contains("client_id=test-client"), "{url}");
    assert!(url.contains("redirect_uri=http%3A%2F%2Fgateway.test%2Fapi%2Fv1%2Fgithub%2Fcallback"), "{url}");
    let value = url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();

    // A callback with a state nobody issued is refused.
    let (_, location) = browse(&state, "/api/v1/github/callback?code=abc&state=not-issued").await;
    assert_eq!(location.as_deref(), Some("http://gateway.test/?github=expired"));

    // The real callback exchanges the code, reads the login, and stores the token encrypted.
    let (_, location) = browse(&state, &format!("/api/v1/github/callback?code=abc&state={value}")).await;
    assert_eq!(location.as_deref(), Some("http://gateway.test/?github=connected"));
    // The same state cannot be used twice.
    let (_, location) = browse(&state, &format!("/api/v1/github/callback?code=abc&state={value}")).await;
    assert_eq!(location.as_deref(), Some("http://gateway.test/?github=expired"));

    let (_, status_body) = call(&state, "GET", "/api/v1/github/status", &token, None).await;
    assert_eq!(status_body, json!({ "connected": true, "login": "octo" }));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_ciphertext FROM github_accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("gho_test_token"), "the token is stored encrypted");

    // The person's repositories, filtered by a search, and a repository's branches.
    let (status, repos) = call(&state, "GET", "/api/v1/github/repos?q=demo", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{repos}");
    assert_eq!(repos, json!([{ "full_name": "acme/demo", "default_branch": "main", "private": false }]));
    let (status, branches) = call(&state, "GET", "/api/v1/github/repos/acme/demo/branches", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{branches}");
    assert_eq!(branches, json!(["main", "dev"]));
    let (status, _) = call(&state, "GET", "/api/v1/github/repos/acme/bad%21name/branches", &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Linking a repository needs no token from the browser; it uses the account's.
    let (ws_id, _) = setup(&state, &token, None).await;
    let (status, linked) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/repos"),
        &token,
        Some(json!({ "full_name": "acme/demo", "default_branch": "main" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{linked}");
    assert_eq!(linked["has_token"], true);
    assert!(!linked.to_string().contains("gho_test_token"));

    // Disconnecting removes the account; the lists then ask the person to connect again.
    let (status, _) = call(&state, "DELETE", "/api/v1/github", &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, status_body) = call(&state, "GET", "/api/v1/github/status", &token, None).await;
    assert_eq!(status_body, json!({ "connected": false, "login": null }));
    let (status, _) = call(&state, "GET", "/api/v1/github/repos", &token, None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let _ = std::fs::remove_dir_all(&root);
}

#[sqlx::test(migrations = "./migrations")]
async fn connecting_needs_the_oauth_app_to_be_configured(pool: PgPool) {
    unsafe { std::env::set_var("GATEWAY_PUBLIC_URL", "http://gateway.test") };
    let root = std::env::temp_dir().join(format!("github-test-{}", Uuid::new_v4()));
    let state = state_for(pool, &root, GitHubConfig::default());
    let token = login(&state, "unconfigured@example.com").await;
    let (status, body) = call(&state, "POST", "/api/v1/github/connect", &token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("not configured"), "{body}");
}
