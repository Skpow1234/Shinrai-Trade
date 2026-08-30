//! Coinbase Exchange adapter against recorded vendor JSON.

use shinrai_instruments::{btc_usd, phase1_master};
use shinrai_market_data::{FeedStatus, MdKind};
use shinrai_md_protocol::{
    CoinbaseExchange, DecodedFrame, FeedSupervisor, MarketDataVendor, SupervisorEvent,
};

#[test]
fn recorded_ticker_normalizes_on_grid() {
    let vendor = CoinbaseExchange;
    let master = phase1_master();
    let raw = include_str!("fixtures/ticker.json");
    let frame = vendor
        .decode_stream(raw.as_bytes(), 1, &master)
        .expect("decode");
    let DecodedFrame::Record(record) = frame else {
        panic!("ticker must decode to a record");
    };
    assert_eq!(record.instrument_id(), btc_usd().id());
    assert_eq!(record.kind(), MdKind::Bbo);
    assert_eq!(record.price().scaled(), 6_500_012);
}

#[test]
fn snapshot_then_ticker_then_heartbeat() {
    let vendor = CoinbaseExchange;
    let master = phase1_master();
    let mut sup = FeedSupervisor::new(30);
    sup.watch(btc_usd().id(), "BTC-USD");
    let _ = sup.on_connected(&vendor, 0);

    let snap = include_str!("fixtures/book_snapshot.json");
    let recovered = sup
        .ingest_snapshot(&vendor, &master, 1, "BTC-USD", snap.as_bytes())
        .expect("snap");
    assert!(matches!(
        recovered.events[0],
        SupervisorEvent::SnapshotRecovered { .. }
    ));

    let ticker = include_str!("fixtures/ticker.json");
    let applied = sup.ingest(&vendor, &master, 2, ticker.as_bytes());
    assert!(matches!(applied.events[0], SupervisorEvent::Applied(_)));

    let hb = include_str!("fixtures/heartbeat.json");
    let beat = sup.ingest(&vendor, &master, 3, hb.as_bytes());
    assert!(matches!(beat.events[0], SupervisorEvent::Heartbeat { .. }));
    assert_eq!(
        sup.consumer().feed_status(btc_usd().id()),
        FeedStatus::Healthy
    );
    assert_eq!(sup.raw().len(), 3);
    assert_eq!(vendor.websocket_url(), CoinbaseExchange::WS_URL);
}
