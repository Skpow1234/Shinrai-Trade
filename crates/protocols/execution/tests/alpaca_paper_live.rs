//! Ignored live smoke against Alpaca paper Trading API.
//!
//! ```bash
//! SHINRAI_OG_ALPACA_KEY=... SHINRAI_OG_ALPACA_SECRET=... \
//!   cargo test -p shinrai-execution --test alpaca_paper_live -- --ignored --nocapture
//! ```

use std::collections::HashMap;

use shinrai_execution::{AlpacaConfig, AlpacaPaperVenue, ExecutionVenue, NewVenueOrder};
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{OrderId, Side, TimeInForce};

#[test]
fn local_alpaca_mock_smoke() {
    let mut symbols = HashMap::new();
    symbols.insert(InstrumentId::from_u64(1), "AAPL".into());
    let mut venue = AlpacaPaperVenue::local_mock(symbols);
    venue
        .submit(&NewVenueOrder {
            order_id: OrderId::from_u64(1),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
    assert!(!venue.poll().is_empty());
}

#[test]
#[ignore = "requires SHINRAI_OG_ALPACA_KEY and SHINRAI_OG_ALPACA_SECRET"]
fn alpaca_paper_submit_and_poll() {
    let config = AlpacaConfig::from_env().expect("Alpaca env credentials");
    let mut symbols = HashMap::new();
    symbols.insert(InstrumentId::from_u64(1), "AAPL".into());
    let mut venue = AlpacaPaperVenue::remote(config, symbols).expect("client");
    venue
        .submit(&NewVenueOrder {
            order_id: OrderId::from_u64(u64::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time")
                    .subsec_nanos(),
            )),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            price: PriceTicks::from_scaled(1), // $0.01 — may reject; still exercises wire
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
    let reports = venue.poll();
    assert!(!reports.is_empty(), "expected New or Rejected report");
}
