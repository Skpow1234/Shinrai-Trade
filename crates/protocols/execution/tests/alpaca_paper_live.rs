//! Ignored live smoke against Alpaca paper Trading API.
//!
//! ```bash
//! SHINRAI_OG_ALPACA_KEY=... SHINRAI_OG_ALPACA_SECRET=... \
//!   cargo test -p shinrai-execution --test alpaca_paper_live -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use shinrai_execution::{AlpacaConfig, AlpacaPaperVenue, ExecType, ExecutionVenue, NewVenueOrder};
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
    let order_id = OrderId::from_u64(u64::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .subsec_nanos(),
    ));
    venue
        .submit(&NewVenueOrder {
            order_id,
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            // Far below market — expect New / Rejected; still exercises wire.
            price: PriceTicks::from_scaled(1),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
    let reports = venue.poll();
    assert!(!reports.is_empty(), "expected New or Rejected report");
    let _ = venue.cancel(order_id);
}

#[test]
#[ignore = "requires SHINRAI_OG_ALPACA_KEY and SHINRAI_OG_ALPACA_SECRET"]
fn alpaca_paper_async_fill_poll() {
    let config = AlpacaConfig::from_env().expect("Alpaca env credentials");
    let mut symbols = HashMap::new();
    symbols.insert(InstrumentId::from_u64(1), "AAPL".into());
    let mut venue = AlpacaPaperVenue::remote(config, symbols).expect("client");
    let order_id = OrderId::from_u64(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_millis()
            % u128::from(u64::MAX),
    )
    .unwrap_or(1));
    // Limit well above a typical AAPL print so paper fills quickly.
    venue
        .submit(&NewVenueOrder {
            order_id,
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            price: PriceTicks::from_scaled(500_000), // $5,000.00 — above market for paper fill
            tif: TimeInForce::Gtc,
        })
        .expect("submit");

    let mut saw_new = false;
    let mut saw_trade = false;
    for _ in 0..30 {
        for r in venue.poll() {
            match r.exec_type() {
                ExecType::New => saw_new = true,
                ExecType::Trade => saw_trade = true,
                ExecType::Rejected { reason } => {
                    panic!("order rejected: {reason}");
                }
                _ => {}
            }
        }
        if saw_trade {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    assert!(saw_new || saw_trade, "expected New ack before timeout");
    assert!(
        saw_trade,
        "expected Trade via async GET fill polling within ~15s"
    );
    let snap = venue.venue_order(order_id).expect("inflight");
    assert!(snap.cum_qty >= 1);
}
