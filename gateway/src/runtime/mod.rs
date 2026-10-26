//! Connections from the gateway to `codex app-server` instances.
//!
//! An environment is reached either through its configured `endpoint`, or, when
//! local launching is configured, through an app-server child process that the
//! gateway starts with its own `CODEX_HOME`. Callers hold a `Lease` while they use
//! a connection; idle local processes are stopped once no lease is held.

pub mod dispatch;
pub mod hub;
pub mod rpc;
pub mod schedule;

use std::{
    collections::HashMap,
    fs::File,
    net::TcpListener,
    ops::Deref,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use sqlx::PgPool;
use tokio::{process::Child, sync::Mutex};
use uuid::Uuid;

pub use hub::Hub;
pub use rpc::{Notification, RpcConnection, RpcError};

const LAUNCH_ATTEMPTS: u32 = 120;
const LAUNCH_RETRY: Duration = Duration::from_millis(250);
const REAP_INTERVAL: Duration = Duration::from_secs(60);

/// How to start a local app-server for an environment without an endpoint.
#[derive(Debug, Clone)]
pub struct LocalLaunch {
    pub codex_bin: PathBuf,
    /// Each environment gets `<home_root>/<environment id>` as its `CODEX_HOME`.
    pub home_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub local: Option<LocalLaunch>,
    /// Local app-server processes are stopped after this long without a lease.
    pub idle_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            local: None,
            idle_timeout: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    NoEndpoint,
    Launch(String),
    Rpc(RpcError),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEndpoint => {
                f.write_str("environment has no endpoint and local launching is disabled")
            }
            Self::Launch(msg) => write!(f, "could not start codex app-server: {msg}"),
            Self::Rpc(err) => write!(f, "codex app-server error: {err}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<RpcError> for RuntimeError {
    fn from(err: RpcError) -> Self {
        Self::Rpc(err)
    }
}

#[derive(Default)]
struct Usage {
    busy: AtomicUsize,
    last_used: StdMutex<Option<Instant>>,
}

struct Entry {
    endpoint: Option<String>,
    connection: Arc<RpcConnection>,
    child: Option<Child>,
    usage: Arc<Usage>,
}

/// A borrowed connection. While it exists, the environment's local process is kept alive.
pub struct Lease {
    connection: Arc<RpcConnection>,
    usage: Arc<Usage>,
}

impl Deref for Lease {
    type Target = RpcConnection;

    fn deref(&self) -> &RpcConnection {
        &self.connection
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.usage.busy.fetch_sub(1, Ordering::AcqRel);
        *self.usage.last_used.lock().expect("usage lock poisoned") = Some(Instant::now());
    }
}

pub struct Runtime {
    config: RuntimeConfig,
    pool: PgPool,
    hub: Arc<Hub>,
    entries: Mutex<HashMap<Uuid, Entry>>,
    thread_locks: StdMutex<HashMap<Uuid, Arc<Mutex<()>>>>,
}

impl Runtime {
    pub fn new(pool: PgPool, config: RuntimeConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            pool,
            hub: Arc::new(Hub::default()),
            entries: Mutex::new(HashMap::new()),
            thread_locks: StdMutex::new(HashMap::new()),
        })
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }

    /// Serializes turns on one thread. Held for the whole turn so turns run in order.
    pub fn thread_lock(&self, thread_id: Uuid) -> Arc<Mutex<()>> {
        let mut locks = self.thread_locks.lock().expect("thread_locks poisoned");
        Arc::clone(locks.entry(thread_id).or_default())
    }

    /// Returns a connection to an environment, starting or reconnecting as needed.
    pub async fn lease(
        &self,
        environment_id: Uuid,
        endpoint: Option<&str>,
    ) -> Result<Lease, RuntimeError> {
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(&environment_id) {
            let child_alive = match entry.child.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => true,
            };
            let same_target = entry.endpoint.as_deref() == endpoint;
            if child_alive && !entry.connection.is_closed() && same_target {
                return Ok(self.lend(entry));
            }
            entries.remove(&environment_id);
        }

        let entry = match endpoint {
            Some(url) => {
                let connection = RpcConnection::connect(url).await?;
                self.spawn_pump(&connection);
                Entry {
                    endpoint: Some(url.to_string()),
                    connection,
                    child: None,
                    usage: Arc::default(),
                }
            }
            None => {
                let local = self.config.local.clone().ok_or(RuntimeError::NoEndpoint)?;
                self.launch_local(environment_id, &local).await?
            }
        };
        let lease = self.lend(&entry);
        entries.insert(environment_id, entry);
        Ok(lease)
    }

    fn lend(&self, entry: &Entry) -> Lease {
        entry.usage.busy.fetch_add(1, Ordering::AcqRel);
        Lease {
            connection: Arc::clone(&entry.connection),
            usage: Arc::clone(&entry.usage),
        }
    }

    async fn launch_local(
        &self,
        environment_id: Uuid,
        local: &LocalLaunch,
    ) -> Result<Entry, RuntimeError> {
        let home = local.home_root.join(environment_id.to_string());
        // The environment's files live in its own workspace, so environments do not share a cwd.
        let workspace = home.join("workspace");
        std::fs::create_dir_all(&workspace).map_err(|err| RuntimeError::Launch(err.to_string()))?;
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0")
                .map_err(|err| RuntimeError::Launch(err.to_string()))?;
            listener
                .local_addr()
                .map_err(|err| RuntimeError::Launch(err.to_string()))?
                .port()
        };
        let url = format!("ws://127.0.0.1:{port}");

        let log = File::create(home.join("app-server.log"))
            .map_err(|err| RuntimeError::Launch(err.to_string()))?;
        let log_err = log
            .try_clone()
            .map_err(|err| RuntimeError::Launch(err.to_string()))?;
        let mut child = tokio::process::Command::new(&local.codex_bin)
            .args(["app-server", "--listen", &url])
            .env("CODEX_HOME", &home)
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| RuntimeError::Launch(format!("{}: {err}", local.codex_bin.display())))?;

        for _ in 0..LAUNCH_ATTEMPTS {
            if let Ok(Some(status)) = child.try_wait() {
                self.set_status(environment_id, "failed").await;
                return Err(RuntimeError::Launch(format!(
                    "process exited with {status}"
                )));
            }
            match RpcConnection::connect(&url).await {
                Ok(connection) => {
                    self.set_status(environment_id, "ready").await;
                    self.spawn_pump(&connection);
                    return Ok(Entry {
                        endpoint: None,
                        connection,
                        child: Some(child),
                        usage: Arc::default(),
                    });
                }
                Err(_) => tokio::time::sleep(LAUNCH_RETRY).await,
            }
        }
        let _ = child.kill().await;
        self.set_status(environment_id, "failed").await;
        Err(RuntimeError::Launch(format!("no response on {url}")))
    }

    async fn set_status(&self, environment_id: Uuid, status: &str) {
        let _ =
            sqlx::query("UPDATE environments SET status = $2, updated_at = now() WHERE id = $1")
                .bind(environment_id)
                .bind(status)
                .execute(&self.pool)
                .await;
    }

    /// Starts the hub pump for every connection, so events and approvals are routed.
    pub fn spawn_pump(&self, connection: &Arc<RpcConnection>) {
        let hub = Arc::clone(&self.hub);
        let pool = self.pool.clone();
        let connection = Arc::clone(connection);
        tokio::spawn(async move { hub.pump(pool, connection).await });
    }

    /// Stops local processes that have been idle longer than the configured timeout.
    pub async fn reap_idle(&self) -> usize {
        let mut entries = self.entries.lock().await;
        let idle_timeout = self.config.idle_timeout;
        let stale: Vec<Uuid> = entries
            .iter()
            .filter(|(_, entry)| {
                entry.child.is_some()
                    && entry.usage.busy.load(Ordering::Acquire) == 0
                    && entry
                        .usage
                        .last_used
                        .lock()
                        .expect("usage lock poisoned")
                        .is_some_and(|at| at.elapsed() >= idle_timeout)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            entries.remove(id);
        }
        stale.len()
    }

    /// Runs `reap_idle` periodically for the life of the process.
    pub fn spawn_reaper(self: &Arc<Self>) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(REAP_INTERVAL);
            loop {
                tick.tick().await;
                runtime.reap_idle().await;
            }
        });
    }

    /// Number of environments that currently have a live connection.
    pub async fn live_connections(&self) -> usize {
        self.entries.lock().await.len()
    }
}
