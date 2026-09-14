//! Order gateway process.

use std::env;
use std::time::Duration;

use shinrai_order_gateway::{router, AppState, GatewayConfig};
use shinrai_store::{connect_from_env, migrate, StoreError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _telemetry = shinrai_telemetry::init("shinrai-order-gateway")?;

    let bind = env::var("SHINRAI_OG_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let config = GatewayConfig::from_env();
    let mut state = AppState::from_config(&config);

    match connect_from_env().await {
        Ok(pool) => {
            if env_truthy("SHINRAI_RUN_MIGRATIONS") {
                migrate(&pool).await?;
                eprintln!("shinrai-order-gateway: migrations applied");
            }
            if shinrai_store::has_durable_state(&pool).await? {
                state.hydrate_from_store(pool).await?;
                eprintln!("shinrai-order-gateway: hydrated PaperEngine from Postgres");
            } else {
                state.attach_store(pool).await;
                state.persist_bootstrap().await?;
                eprintln!(
                    "shinrai-order-gateway: Postgres write-through enabled (fresh bootstrap)"
                );
            }
            if let Some(pool) = state.store_pool() {
                let metrics = state.outbox_metrics();
                let poll_ms: u64 = env::var("SHINRAI_OUTBOX_POLL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1_000);
                tokio::spawn(shinrai_order_gateway::run_outbox_publisher(
                    pool,
                    metrics,
                    Duration::from_millis(poll_ms),
                ));
                eprintln!("shinrai-order-gateway: outbox publisher started (poll {poll_ms}ms)");
            }
        }
        Err(StoreError::MissingDatabaseUrl) => {
            eprintln!("shinrai-order-gateway: no SHINRAI_DATABASE_URL; in-memory only");
        }
        Err(err) => return Err(err.into()),
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("shinrai-order-gateway listening on {bind}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn env_truthy(key: &str) -> bool {
    matches!(
        env::var(key).ok().as_deref().map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}
