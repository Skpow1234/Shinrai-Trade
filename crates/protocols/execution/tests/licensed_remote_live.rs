//! Ignored live smoke against a remote licensed-shaped paper venue.
//!
//! Requires `SHINRAI_OG_REST_URL` (same remote JSON contract as REST paper).
//! The licensed venue itself is in-process; this smoke exercises remote REST
//! recovery used beside licensed session semantics.
//!
//! ```bash
//! cargo test -p shinrai-execution --test licensed_remote_live -- --ignored --nocapture
//! ```

use shinrai_execution::{
    ExecutionVenue, LicensedSandboxConfig, LicensedSandboxVenue, NewVenueOrder,
};
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{OrderId, Side, TimeInForce};

#[test]
#[ignore = "requires SHINRAI_OG_REST_URL remote paper venue"]
fn licensed_session_logon_and_fill_local() {
    // Local session-shaped path (always runnable when forced with --ignored alone
    // if URL unset — still validates licensed venue).
    let mut venue = LicensedSandboxVenue::new(LicensedSandboxConfig::happy_path());
    assert!(venue.submit(&sample_order(1)).is_err());
    venue.logon().expect("logon");
    venue.submit(&sample_order(1)).expect("submit");
    let reports = venue.poll();
    assert!(!reports.is_empty());
}

#[test]
#[ignore = "requires SHINRAI_OG_REST_URL"]
fn licensed_adjacent_remote_rest_smoke() {
    let url = std::env::var("SHINRAI_OG_REST_URL").expect("SHINRAI_OG_REST_URL");
    let bearer = std::env::var("SHINRAI_OG_REST_TOKEN").ok();
    let mut venue = shinrai_execution::RestPaperVenue::remote(url, bearer).expect("remote client");
    venue
        .submit(&NewVenueOrder {
            order_id: OrderId::from_u64(42),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        })
        .expect("remote submit");
    let reports = venue.poll();
    assert!(!reports.is_empty(), "expected at least one report");
}

fn sample_order(id: u64) -> NewVenueOrder {
    NewVenueOrder {
        order_id: OrderId::from_u64(id),
        instrument_id: InstrumentId::from_u64(1),
        side: Side::Buy,
        qty: QuantityLots::from_lots(1),
        price: PriceTicks::from_scaled(10_000),
        tif: TimeInForce::Gtc,
    }
}
