use sqlx::PgPool;

use crate::runtime::Runtime;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub dev_login: bool,
    pub runtime: std::sync::Arc<Runtime>,
}

impl AppState {
    pub fn new(pool: PgPool, dev_login: bool) -> Self {
        Self {
            pool,
            dev_login,
            runtime: std::sync::Arc::new(Runtime::default()),
        }
    }
}
