use std::{sync::Arc, time::Duration};

use sqlx::PgPool;

use crate::{
    ratelimit::SlidingWindow,
    runtime::{Runtime, RuntimeConfig},
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
        }
    }

    pub fn with_turn_limit(mut self, turns_per_minute: usize) -> Self {
        self.turn_limiter = Arc::new(SlidingWindow::new(
            turns_per_minute,
            Duration::from_secs(60),
        ));
        self
    }
}
