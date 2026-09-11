//! Order gateway process.

use std::env;

use shinrai_order_gateway::{router, AppState, GatewayConfig};
use shinrai_store::{connect_from_env, migrate, StoreError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind = env::var("SHINRAI_OG_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let config = GatewayConfig::from_env();
    let mut state = AppState::from_config(&config);

    match connect_from_env().await {
        Ok(pool) => {
            if env_truthy("SHINRAI_RUN_MIGRATIONS") {
                migrate(&pool).await?;
                eprintln!("shinrai-order-gateway: migrations applied");
            }
            state.attach_store(pool);
            state.persist_bootstrap().await;
            eprintln!("shinrai-order-gateway: Postgres dual-write enabled");
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
