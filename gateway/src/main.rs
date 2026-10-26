use codex_gateway::{AppState, app, config::Config};
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

    let listener = TcpListener::bind(&config.addr).await?;
    info!(addr = %config.addr, dev_login = config.dev_login, "gateway listening");
    axum::serve(listener, app(AppState::new(pool, config.dev_login))).await?;
    Ok(())
}
