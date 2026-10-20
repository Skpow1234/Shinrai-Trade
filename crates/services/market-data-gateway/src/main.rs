//! Market-data gateway process.

use std::env;

use shinrai_md_gateway::{router, AppState, FeedMode, GatewayConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind = env::var("SHINRAI_MD_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let config = GatewayConfig::from_env();
    if config.coinbase() && (config.synth() || config.alpaca()) {
        eprintln!(
            "shinrai-md-gateway: SHINRAI_MD_COINBASE set; Coinbase wins over synth/alpaca for feed mode"
        );
    }
    let state = AppState::from_config(&config);
    match state.feed_mode() {
        FeedMode::Coinbase => {
            state.spawn_coinbase();
            eprintln!("shinrai-md-gateway: live Coinbase feed enabled (BTC-USD)");
        }
        FeedMode::Alpaca => {
            state.spawn_alpaca();
            eprintln!(
                "shinrai-md-gateway: live Alpaca stock feed enabled (products={:?})",
                config.products()
            );
        }
        FeedMode::Synth => {
            state.spawn_synth();
            eprintln!(
                "shinrai-md-gateway: synthetic publisher enabled (products={:?})",
                config.products()
            );
        }
        FeedMode::None => {
            eprintln!("shinrai-md-gateway: no live/synth publisher (seed history only)");
        }
    }
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("shinrai-md-gateway listening on {bind}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
