//! Futures MD smokes (paper in CI; licensed REST ignored).
//!
//! ```bash
//! SHINRAI_MD_FUTURES_KEY=… SHINRAI_MD_FUTURES_REST=… \
//!   cargo test -p shinrai-md-gateway --test futures_md_live -- --ignored --nocapture
//! ```

use std::time::Duration;

use shinrai_instruments::phase1_master;
use shinrai_md_protocol::{DatabentoGlbx, MarketDataVendor};

#[test]
fn paper_esz5_snapshot_decodes() {
    let vendor = DatabentoGlbx::paper();
    let master = phase1_master();
    let body = DatabentoGlbx::paper_snapshot_body("ESZ5", 1, "5200.00");
    let rec = vendor
        .decode_snapshot(&body, 1, "ESZ5", &master)
        .expect("paper snap");
    assert_eq!(rec.price().scaled(), 520_000);
}

#[tokio::test]
#[ignore = "licensed Databento/CME REST; requires SHINRAI_MD_FUTURES_KEY + SHINRAI_MD_FUTURES_REST"]
async fn live_futures_snapshot_esz5() {
    let key = std::env::var("SHINRAI_MD_FUTURES_KEY")
        .or_else(|_| std::env::var("DATABENTO_API_KEY"))
        .expect("SHINRAI_MD_FUTURES_KEY");
    let rest_origin = std::env::var("SHINRAI_MD_FUTURES_REST")
        .unwrap_or_else(|_| DatabentoGlbx::REST_DEFAULT.into());
    let vendor = DatabentoGlbx::live(key).with_endpoints(DatabentoGlbx::WS_DEFAULT, rest_origin);
    let spec = vendor.snapshot_spec("ESZ5");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("client");
    let mut req = client.get(spec.url());
    for (k, v) in spec.headers() {
        req = req.header(k.as_str(), v.as_str());
    }
    let http_resp = req.send().await.expect("http");
    assert!(
        http_resp.status().is_success() || http_resp.status().as_u16() == 422,
        "unexpected status {}",
        http_resp.status()
    );
}
