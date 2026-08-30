//! L2 snapshot rebuild checksum (public API).

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_market_data::{
    BookApplyOutcome, BookChange, BookDelta, BookEngine, BookEvent, BookLevel, BookSide,
    BookSnapshot, BookStatus,
};

fn px(v: i64) -> PriceTicks {
    PriceTicks::from_scaled(v)
}

fn qty(v: i64) -> QuantityLots {
    QuantityLots::from_lots(v)
}

#[test]
fn snapshot_checksum_cross_check_after_rebuild() {
    let id = InstrumentId::from_u64(1);
    let snap = BookSnapshot::new(
        id,
        42,
        0,
        vec![
            BookLevel::new(px(10), qty(4)),
            BookLevel::new(px(9), qty(1)),
        ],
        vec![BookLevel::new(px(11), qty(2))],
    );
    let expect = snap.checksum();

    let mut engine = BookEngine::new();
    engine.invalidate(id);
    assert_eq!(
        engine.book(id).expect("inv").status(),
        BookStatus::Invalidated
    );

    let outcome = engine.apply(&BookEvent::Snapshot(snap)).expect("rebuild");
    assert_eq!(outcome, BookApplyOutcome::Applied { checksum: expect });
    assert_eq!(engine.book(id).expect("b").checksum(), expect);
}

#[test]
fn delta_then_rebuild_matches_fresh_snapshot_plus_deltas() {
    let id = InstrumentId::from_u64(2);
    let snap = BookSnapshot::new(
        id,
        1,
        0,
        vec![BookLevel::new(px(50), qty(3))],
        vec![BookLevel::new(px(51), qty(3))],
    );
    let delta = BookDelta::new(
        id,
        None,
        1,
        vec![BookChange::new(BookSide::Bid, px(50), qty(8))],
    );

    let mut a = BookEngine::new();
    a.apply(&BookEvent::Snapshot(snap.clone())).expect("s");
    a.apply(&BookEvent::Delta(delta.clone())).expect("d");

    let mut b = BookEngine::new();
    b.invalidate(id);
    b.apply(&BookEvent::Snapshot(snap)).expect("s2");
    b.apply(&BookEvent::Delta(delta)).expect("d2");

    assert_eq!(
        a.book(id).expect("a").checksum(),
        b.book(id).expect("b").checksum()
    );
}
