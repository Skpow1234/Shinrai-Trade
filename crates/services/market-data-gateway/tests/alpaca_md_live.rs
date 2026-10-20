//! Ignored live smoke against Alpaca stock market data (needs API keys).
//!
//! ```bash
//! SHINRAI_MD_ALPACA_KEY=… SHINRAI_MD_ALPACA_SECRET=… \
//!   cargo test -p shinrai-md-gateway --test alpaca_md_live -- --ignored --nocapture
//! ```

use std::time::Duration;

use shinrai_instruments::phase1_master;
use shinrai_md_protocol::{AlpacaMarkets, MarketDataVendor};

#[tokio::test]
#[ignore = "live Alpaca market data; requires SHINRAI_MD_ALPACA_KEY/SECRET"]
async fn alpaca_snapshot_aapl() {
    let key = std::env::var("SHINRAI_MD_ALPACA_KEY")
        .or_else(|_| std::env::var("SHINRAI_OG_ALPACA_KEY"))
        .expect("SHINRAI_MD_ALPACA_KEY");
    let secret = std::env::var("SHINRAI_MD_ALPACA_SECRET")
        .or_else(|_| std::env::var("SHINRAI_OG_ALPACA_SECRET"))
        .expect("SHINRAI_MD_ALPACA_SECRET");
    let vendor = AlpacaMarkets::iex(key, secret);
    let spec = vendor.snapshot_spec("AAPL");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("client");
    let mut req = client.get(spec.url());
    for (k, v) in spec.headers() {
        req = req.header(k.as_str(), v.as_str());
    }
    let body = req
        .send()
        .await
        .expect("http")
        .bytes()
        .await
        .expect("bytes");
    let master = phase1_master();
    let snapshot = vendor
        .decode_snapshot(&body, 1, "AAPL", &master)
        .expect("decode snapshot");
    assert!(snapshot.price().scaled() > 0, "expected positive AAPL mark");
}
