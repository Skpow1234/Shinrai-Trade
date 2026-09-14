//! Optional live Coinbase smoke test (network). Not run in CI by default.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shinrai_md_protocol::{CoinbaseExchange, MarketDataVendor};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Connects to the public Coinbase WS, sends subscribe, waits for any text frame.
///
/// Run manually:
/// `cargo test -p shinrai-md-gateway --test coinbase_live -- --ignored --nocapture`
#[tokio::test]
#[ignore = "opens a live Coinbase WebSocket; not for CI"]
async fn live_coinbase_subscribe_receives_frame() {
    let vendor = CoinbaseExchange;
    let url = vendor.websocket_url();
    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(15), connect_async(url))
        .await
        .expect("connect timeout")
        .expect("connect");

    let sub = vendor.subscribe_message(&["BTC-USD".into()]);
    let text = String::from_utf8(sub).expect("utf8");
    ws.send(Message::Text(text.into()))
        .await
        .expect("subscribe");

    let frame = tokio::time::timeout(Duration::from_secs(20), ws.next())
        .await
        .expect("frame timeout")
        .expect("stream ended")
        .expect("ws error");
    let text = frame.into_text().expect("text frame");
    assert!(
        text.contains("subscriptions")
            || text.contains("ticker")
            || text.contains("heartbeat")
            || text.contains("snapshot")
            || text.contains("match"),
        "unexpected first frame: {text}"
    );
}
