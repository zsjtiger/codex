use std::sync::Arc;

use sqlx::PgPool;

use crate::runtime::{Runtime, RuntimeConfig};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub dev_login: bool,
    pub runtime: Arc<Runtime>,
}

impl AppState {
    pub fn new(pool: PgPool, dev_login: bool, runtime_config: RuntimeConfig) -> Self {
        let runtime = Runtime::new(pool.clone(), runtime_config);
        Self {
            pool,
            dev_login,
            runtime,
        }
    }
}
