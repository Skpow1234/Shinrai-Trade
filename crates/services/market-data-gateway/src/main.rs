//! Market-data gateway process.

use std::env;

use shinrai_md_gateway::{router, AppState, GatewayConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind = env::var("SHINRAI_MD_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let config = GatewayConfig::from_env();
    let synth = config.synth();
    let state = AppState::from_config(&config);
    if synth {
        state.spawn_synth();
    }
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("shinrai-md-gateway listening on {bind}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
