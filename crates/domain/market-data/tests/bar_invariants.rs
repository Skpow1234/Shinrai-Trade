//! Property tests for OHLCV aggregation.

use proptest::prelude::*;
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_market_data::{BarAggregator, BarInterval, MdKind, MdRecord};

proptest! {
    #[test]
    fn closed_bars_obey_ohlcv_invariants(
        trades in proptest::collection::vec(
            (0_u64..200, 1_i64..10_000, 1_i64..50),
            1..40,
        )
    ) {
        let inst = InstrumentId::from_u64(1);
        let interval = BarInterval::try_new(10).expect("i");
        let mut agg = BarAggregator::new([interval]);
        let mut expected_vol: std::collections::BTreeMap<u64, i64> = std::collections::BTreeMap::new();
        for (i, (ts, px, qty)) in trades.iter().enumerate() {
            let seq = u64::try_from(i + 1).expect("seq");
            let record = MdRecord::new(
                inst,
                seq,
                *ts,
                MdKind::Trade,
                PriceTicks::from_scaled(*px),
            )
            .with_qty(QuantityLots::from_lots(*qty));
            agg.apply(record).expect("apply");
            let bucket = interval.bucket(*ts);
            *expected_vol.entry(bucket).or_insert(0) += *qty;
        }
        agg.close_open();
        let mut seen = 0_usize;
        for bar in agg.store().series(inst, interval) {
            bar.assert_invariants().expect("inv");
            assert_eq!(
                bar.volume().lots(),
                expected_vol.get(&bar.open_ts()).copied().unwrap_or(0)
            );
            seen += 1;
        }
        assert_eq!(seen, expected_vol.len());
    }
}
