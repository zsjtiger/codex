use std::{path::PathBuf, sync::Arc, time::Duration};

use sqlx::PgPool;

use crate::{
    ratelimit::SlidingWindow,
    runtime::{Runtime, RuntimeConfig},
    storage::ObjectStore,
    tickets::Tickets,
};

pub const DEFAULT_TURNS_PER_MINUTE: usize = 60;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub dev_login: bool,
    pub runtime: Arc<Runtime>,
    pub tickets: Arc<Tickets>,
    /// Caps how many turns one user can create per minute.
    pub turn_limiter: Arc<SlidingWindow>,
    /// Where attachment bytes are stored, under the keys recorded in the database.
    pub storage: Arc<ObjectStore>,
}

impl AppState {
    pub fn new(pool: PgPool, dev_login: bool, runtime_config: RuntimeConfig) -> Self {
        let runtime = Runtime::new(pool.clone(), runtime_config);
        Self {
            pool,
            dev_login,
            runtime,
            tickets: Arc::default(),
            turn_limiter: Arc::new(SlidingWindow::new(
                DEFAULT_TURNS_PER_MINUTE,
                Duration::from_secs(60),
            )),
            storage: Arc::new(ObjectStore::local("storage")),
        }
    }

    pub fn with_storage(self, dir: PathBuf) -> Self {
        self.with_object_store(ObjectStore::local(dir))
    }

    pub fn with_object_store(mut self, store: ObjectStore) -> Self {
        self.storage = Arc::new(store);
        self
    }

    pub fn with_turn_limit(mut self, turns_per_minute: usize) -> Self {
        self.turn_limiter = Arc::new(SlidingWindow::new(
            turns_per_minute,
            Duration::from_secs(60),
        ));
        self
    }
}
