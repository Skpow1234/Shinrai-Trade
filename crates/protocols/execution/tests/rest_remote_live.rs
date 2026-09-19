//! Remote REST paper venue smoke (licensed/sandbox broker speaking Shinrai wire JSON).
//!
//! Ignored by default. Point `SHINRAI_OG_REST_URL` at a broker that implements the
//! paper REST surface (`POST /v1/orders`, etc.), then:
//! `cargo test -p shinrai-execution --test rest_remote_live -- --ignored --nocapture`

use shinrai_execution::{ExecutionVenue, NewVenueOrder, RestPaperVenue};
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{OrderId, Side, TimeInForce};

#[test]
#[ignore = "requires SHINRAI_OG_REST_URL pointing at a paper/broker REST sandbox"]
fn remote_rest_submit_and_poll() {
    let url = std::env::var("SHINRAI_OG_REST_URL").expect("SHINRAI_OG_REST_URL");
    let bearer = std::env::var("SHINRAI_OG_REST_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());
    let mut venue = RestPaperVenue::remote(url, bearer).expect("remote client");

    let order = NewVenueOrder {
        order_id: OrderId::from_u64(9_001),
        instrument_id: InstrumentId::from_u64(1),
        side: Side::Buy,
        qty: QuantityLots::from_lots(1),
        price: PriceTicks::from_scaled(10_000),
        tif: TimeInForce::Gtc,
    };
    venue.submit(&order).expect("submit");
    let reports = venue.poll();
    assert!(
        !reports.is_empty(),
        "expected at least one execution report from remote venue"
    );
}
