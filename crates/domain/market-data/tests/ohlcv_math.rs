//! Known OHLCV vectors (public API).

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_market_data::{BarAggregator, BarInterval, MdJournal, MdKind, MdRecord};

fn print(inst: InstrumentId, seq: u64, ts: u64, px: i64, qty: i64) -> MdRecord {
    MdRecord::new(inst, seq, ts, MdKind::Trade, PriceTicks::from_scaled(px))
        .with_qty(QuantityLots::from_lots(qty))
}

#[test]
fn one_second_and_one_minute_from_same_prints() {
    let inst = InstrumentId::from_u64(3);
    let journal = MdJournal::from_records([
        print(inst, 1, 60, 100, 1),
        print(inst, 2, 61, 120, 2),
        print(inst, 3, 119, 90, 1),
        print(inst, 4, 120, 95, 4),
    ])
    .expect("journal");

    let agg = BarAggregator::from_journal(&journal, [BarInterval::SECOND, BarInterval::MINUTE])
        .expect("agg");

    let seconds: Vec<_> = agg
        .store()
        .series(inst, BarInterval::SECOND)
        .copied()
        .collect();
    assert_eq!(seconds.len(), 4);
    assert_eq!(seconds[0].open_ts(), 60);
    assert_eq!(seconds[0].volume().lots(), 1);
    assert_eq!(seconds[2].open_ts(), 119);
    assert_eq!(seconds[2].close().scaled(), 90);
    assert_eq!(seconds[3].open_ts(), 120);
    assert_eq!(seconds[3].volume().lots(), 4);

    let minutes: Vec<_> = agg
        .store()
        .series(inst, BarInterval::MINUTE)
        .copied()
        .collect();
    assert_eq!(minutes.len(), 2);
    assert_eq!(minutes[0].open_ts(), 60);
    assert_eq!(minutes[0].open().scaled(), 100);
    assert_eq!(minutes[0].high().scaled(), 120);
    assert_eq!(minutes[0].low().scaled(), 90);
    assert_eq!(minutes[0].close().scaled(), 90);
    assert_eq!(minutes[0].volume().lots(), 4);
    assert_eq!(minutes[0].trade_count(), 3);
    assert_eq!(minutes[1].open_ts(), 120);
    assert_eq!(minutes[1].open().scaled(), 95);
    assert_eq!(minutes[1].close().scaled(), 95);
    assert_eq!(minutes[1].volume().lots(), 4);
}
