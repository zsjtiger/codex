//! SSH repositories, end to end: the gateway makes a key and pins the host, the key is authorized on a
//! real local sshd, the fingerprint is confirmed, and a session commits to its branch over SSH.
//!
//! Needs `sshd`, `ssh`, `ssh-keygen` and `ssh-keyscan` on the machine; without them the test says so
//! and passes without checking anything.

mod common;

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::Duration,
};

use axum::http::StatusCode;
use codex_gateway::{AppState, code::CodeConfig, github::GitHubConfig, runtime::RuntimeConfig};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const SSHD: &str = "/usr/sbin/sshd";

fn tools_present() -> bool {
    [SSHD, "/usr/bin/ssh", "/usr/bin/ssh-keygen", "/usr/bin/ssh-keyscan"]
        .iter()
        .all(|path| Path::new(path).exists())
}

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

/// `<root>/team/app.git` with one commit on `main`.
fn make_remote(root: &Path) -> PathBuf {
    let remote = root.join("team").join("app.git");
    std::fs::create_dir_all(&remote).unwrap();
    let remote_text = remote.to_string_lossy().to_string();
    git(Some(&remote), &["init", "--bare", "-q", "-b", "main"]);
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(Some(&seed), &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(Some(&seed), &["add", "."]);
    git(Some(&seed), &["-c", "user.name=seed", "-c", "user.email=seed@example.test", "commit", "-q", "-m", "init"]);
    git(Some(&seed), &["push", "-q", &remote_text, "main"]);
    remote
}

/// A real sshd on a free local port, with its own host key and authorized keys. Stopped on drop.
struct Sshd {
    child: Child,
    port: u16,
    authorized_keys: PathBuf,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_sshd(dir: &Path) -> Sshd {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let host_key = dir.join("hostkey");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&host_key)
        .status()
        .unwrap();
    assert!(status.success());
    let authorized_keys = dir.join("authorized_keys");
    std::fs::write(&authorized_keys, "").unwrap();
    let config = dir.join("sshd_config");
    std::fs::write(
        &config,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\n\
             PubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n\
             UsePAM no\nPermitRootLogin yes\nStrictModes no\nLogLevel ERROR\n",
            host_key.display(),
            dir.join("sshd.pid").display(),
            authorized_keys.display(),
        ),
    )
    .unwrap();
    std::fs::create_dir_all("/run/sshd").ok();
    let child = Command::new(SSHD).args(["-D", "-e", "-f"]).arg(&config).spawn().unwrap();
    // Wait until it accepts connections.
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Sshd { child, port, authorized_keys };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("sshd did not start");
}

#[sqlx::test(migrations = "./migrations")]
async fn an_ssh_repository_is_added_checked_and_committed_to_over_ssh(pool: PgPool) {
    if !tools_present() {
        eprintln!("skipping: sshd or the OpenSSH client tools are not installed");
        return;
    }
    let root = std::env::temp_dir().join(format!("ssh-remote-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let remote = make_remote(&root);
    let sshd = start_sshd(&root);
    let mock = mock_app_server().await;

    let config = CodeConfig {
        root: root.join("data"),
        git_base: "https://github.com".to_string(),
        key: Some([3u8; 32]),
        github: GitHubConfig::default(),
        allow_file_remotes: false,
    };
    let state = AppState::new(pool.clone(), true, RuntimeConfig::default()).with_code(config);
    let token = login(&state, "ssh@example.com").await;
    let (ws_id, chat_id) = setup(&state, &token, Some(&mock.url)).await;
    let environment_id = call(&state, "GET", &format!("/api/v1/threads/{chat_id}"), &token, None).await.1["environment_id"].clone();

    // Adding it makes a key and pins the host; the public key and fingerprint are for the person to use.
    let url = format!("ssh://root@127.0.0.1:{}{}", sshd.port, remote.display());
    let (status, created) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/repos/ssh"),
        &token,
        Some(json!({ "url": url })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["repo"]["ready"], false);
    let public_key = created["repo"]["public_key"].as_str().unwrap().to_string();
    assert!(public_key.starts_with("ssh-ed25519 "), "{public_key}");
    let fingerprint = created["host_fingerprints"].as_str().unwrap().to_string();
    assert!(fingerprint.contains("SHA256:"), "{fingerprint}");
    let repo_id = created["repo"]["id"].as_str().unwrap().to_string();
    assert!(!created.to_string().contains("PRIVATE KEY"), "the private key is never returned");

    // A pending repository cannot start a session.
    let (status, refused) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        &token,
        Some(json!({ "title": "Too early", "environment_id": environment_id, "repo_id": repo_id })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // A wrong fingerprint is refused before anything is cloned.
    let (status, mismatch) = call(
        &state,
        "POST",
        &format!("/api/v1/repos/{repo_id}/check"),
        &token,
        Some(json!({ "fingerprint": "SHA256:not-the-server" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mismatch}");
    assert!(mismatch.to_string().contains("does not match"), "{mismatch}");

    // Before the key is a deploy key, the clone is refused, and the repository stays pending.
    let (status, early) = call(
        &state,
        "POST",
        &format!("/api/v1/repos/{repo_id}/check"),
        &token,
        Some(json!({ "fingerprint": fingerprint })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{early}");
    assert!(early.to_string().contains("deploy key"), "{early}");

    // The person adds the public key as a deploy key (simulated here by authorizing it on the server).
    std::fs::write(&sshd.authorized_keys, format!("{public_key}\n")).unwrap();
    let (status, checked) = call(
        &state,
        "POST",
        &format!("/api/v1/repos/{repo_id}/check"),
        &token,
        Some(json!({ "fingerprint": fingerprint })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["ready"], true);
    assert!(checked["public_key"].is_null(), "the public key is shown only while pending");

    // Now a session starts, runs in its checkout over SSH, and its change is committed and pushed.
    let (status, code_chat) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/threads"),
        &token,
        Some(json!({ "title": "Over SSH", "environment_id": environment_id, "repo_id": repo_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{code_chat}");
    let code_chat_id = code_chat["id"].as_str().unwrap().to_string();
    let branch = code_chat["work_branch"].as_str().unwrap().to_string();
    call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/turns"), &token, Some(json!({ "input": "start" }))).await;
    assert_eq!(wait_for_turn(&state, &token, &code_chat_id, 0).await["status"], "completed");
    let worktree = root.join("data").join("worktrees").join(&code_chat_id);
    assert_eq!(git(Some(&worktree), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);

    std::fs::write(worktree.join("README.md"), "changed over ssh\n").unwrap();
    let (status, result) = call(&state, "POST", &format!("/api/v1/threads/{code_chat_id}/commit"), &token, Some(json!({ "message": "Change over SSH" }))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(git(Some(&remote), &["log", "-1", "--format=%s", &branch]), "Change over SSH");
    assert_eq!(git(Some(&remote), &["log", "-1", "--format=%s", "main"]), "init", "main is untouched");

    let _ = std::fs::remove_dir_all(&root);
}
