//! Order gateway process.

use std::env;

use shinrai_order_gateway::{router, AppState, GatewayConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind = env::var("SHINRAI_OG_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let config = GatewayConfig::from_env();
    let state = AppState::from_config(&config);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("shinrai-order-gateway listening on {bind}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
