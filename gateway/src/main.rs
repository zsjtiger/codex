use codex_gateway::{AppState, app, config::Config, runtime::dispatch};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codex_gateway=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let storage = codex_gateway::storage::ObjectStore::from_config(&config.storage).await;
    let state = AppState::new(pool, config.dev_login, config.runtime_config())
        .with_turn_limit(config.turns_per_minute)
        .with_object_store(storage);
    state.runtime.spawn_reaper();
    codex_gateway::runtime::schedule::spawn_scheduler(state.clone());
    let interrupted = dispatch::recover(&state).await?;
    info!(interrupted, "recovered turns from a previous run");

    let listener = TcpListener::bind(&config.addr).await?;
    info!(addr = %config.addr, dev_login = config.dev_login, local_runtime = config.codex_bin.is_some(), "gateway listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
