//! OHLCV bars aggregated from trade prints.
//!
//! Bars are stored **separately** from the tick journal. Only [`MdKind::Trade`]
//! prints update candles (BBO / snapshots do not).
//!
//! # Time base and sessions
//!
//! Bar open time is `floor(ts / duration) * duration` in the timestamp's own
//! units.
//!
//! - Logical clocks (simulator): `duration` is in the same ticks as
//!   [`crate::MdRecord::ts_logical`].
//! - Vendor wall clocks: use Unix seconds in **UTC**. `BarInterval::minute()`
//!   is then a UTC minute; `BarInterval::day()` is UTC midnight, **not** an
//!   exchange session (XNAS RTH, CME pit hours, etc.).
//!
//! empty intervals are **not** filled. Prints that arrive after a later
//! bucket has already opened amend the historical bar's high/low/volume and
//! leave open/close unchanged.

use std::collections::{BTreeMap, HashMap};

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};

use crate::error::MdError;
use crate::journal::MdJournal;
use crate::record::{MdKind, MdRecord};

/// Aggregation window. `duration` is in the same units as the trade timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BarInterval {
    duration: u64,
}

impl BarInterval {
    /// One timestamp unit (1s if `ts` is Unix seconds; 1 tick if logical).
    pub const SECOND: Self = Self { duration: 1 };
    /// 60 timestamp units (UTC minute when `ts` is Unix seconds).
    pub const MINUTE: Self = Self { duration: 60 };
    /// `3600` timestamp units (UTC hour).
    pub const HOUR: Self = Self { duration: 3_600 };
    /// `86400` timestamp units (UTC day).
    pub const DAY: Self = Self { duration: 86_400 };

    /// Creates an interval.
    ///
    /// # Errors
    ///
    /// Returns [`MdError::InvalidInterval`] if `duration == 0`.
    pub const fn try_new(duration: u64) -> Result<Self, MdError> {
        if duration == 0 {
            return Err(MdError::InvalidInterval);
        }
        Ok(Self { duration })
    }

    /// Window length.
    #[must_use]
    pub const fn duration(self) -> u64 {
        self.duration
    }

    /// Inclusive bar open timestamp for `ts`.
    #[must_use]
    pub const fn bucket(self, ts: u64) -> u64 {
        (ts / self.duration).saturating_mul(self.duration)
    }
}

/// One OHLCV candle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OhlcvBar {
    instrument_id: InstrumentId,
    interval: BarInterval,
    open_ts: u64,
    open: PriceTicks,
    high: PriceTicks,
    low: PriceTicks,
    close: PriceTicks,
    volume: QuantityLots,
    trade_count: u64,
}

impl OhlcvBar {
    fn new(
        instrument_id: InstrumentId,
        interval: BarInterval,
        open_ts: u64,
        price: PriceTicks,
        qty: QuantityLots,
    ) -> Self {
        Self {
            instrument_id,
            interval,
            open_ts,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: qty,
            trade_count: 1,
        }
    }

    /// Instrument.
    #[must_use]
    pub const fn instrument_id(self) -> InstrumentId {
        self.instrument_id
    }

    /// Aggregation interval.
    #[must_use]
    pub const fn interval(self) -> BarInterval {
        self.interval
    }

    /// Bucket open (inclusive).
    #[must_use]
    pub const fn open_ts(self) -> u64 {
        self.open_ts
    }

    /// First print in the bucket.
    #[must_use]
    pub const fn open(self) -> PriceTicks {
        self.open
    }

    /// Highest print.
    #[must_use]
    pub const fn high(self) -> PriceTicks {
        self.high
    }

    /// Lowest print.
    #[must_use]
    pub const fn low(self) -> PriceTicks {
        self.low
    }

    /// Last print in the bucket.
    #[must_use]
    pub const fn close(self) -> PriceTicks {
        self.close
    }

    /// Sum of trade quantities in lot units.
    #[must_use]
    pub const fn volume(self) -> QuantityLots {
        self.volume
    }

    /// Number of prints.
    #[must_use]
    pub const fn trade_count(self) -> u64 {
        self.trade_count
    }

    /// OHLC / volume invariants.
    ///
    /// # Errors
    ///
    /// Returns [`MdError::InvalidQuantity`] if the bar is internally inconsistent.
    pub fn assert_invariants(self) -> Result<(), MdError> {
        if self.volume.lots() < 0 || self.trade_count == 0 {
            return Err(MdError::InvalidQuantity);
        }
        if self.high < self.open || self.high < self.close || self.high < self.low {
            return Err(MdError::InvalidQuantity);
        }
        if self.low > self.open || self.low > self.close {
            return Err(MdError::InvalidQuantity);
        }
        Ok(())
    }

    fn apply_print(&mut self, price: PriceTicks, qty: QuantityLots) -> Result<(), MdError> {
        self.add_volume(qty)?;
        self.trade_count = self.trade_count.saturating_add(1);
        self.close = price;
        self.widen(price);
        Ok(())
    }

    /// Late print: volume / high / low only. Open and close stay as first/last in-order.
    fn apply_late(&mut self, price: PriceTicks, qty: QuantityLots) -> Result<(), MdError> {
        self.add_volume(qty)?;
        self.trade_count = self.trade_count.saturating_add(1);
        self.widen(price);
        Ok(())
    }

    fn widen(&mut self, price: PriceTicks) {
        if price > self.high {
            self.high = price;
        }
        if price < self.low {
            self.low = price;
        }
    }

    fn add_volume(&mut self, qty: QuantityLots) -> Result<(), MdError> {
        if qty.lots() < 0 {
            return Err(MdError::InvalidQuantity);
        }
        let vol = self
            .volume
            .lots()
            .checked_add(qty.lots())
            .ok_or(MdError::Overflow)?;
        self.volume = QuantityLots::from_lots(vol);
        Ok(())
    }
}

/// Historical closed bars (not the tick journal).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BarStore {
    bars: BTreeMap<(InstrumentId, BarInterval, u64), OhlcvBar>,
}

impl BarStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of closed bars.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bars.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// Closed bar at an exact open timestamp, if any.
    #[must_use]
    pub fn get(
        &self,
        instrument_id: InstrumentId,
        interval: BarInterval,
        open_ts: u64,
    ) -> Option<&OhlcvBar> {
        self.bars.get(&(instrument_id, interval, open_ts))
    }

    /// Closed bars for an instrument/interval in open-time order.
    pub fn series(
        &self,
        instrument_id: InstrumentId,
        interval: BarInterval,
    ) -> impl Iterator<Item = &OhlcvBar> {
        self.bars
            .range((instrument_id, interval, 0)..=(instrument_id, interval, u64::MAX))
            .map(|(_, bar)| bar)
    }

    fn insert(&mut self, bar: OhlcvBar) {
        self.bars
            .insert((bar.instrument_id, bar.interval, bar.open_ts), bar);
    }

    fn apply_late(
        &mut self,
        instrument_id: InstrumentId,
        interval: BarInterval,
        open_ts: u64,
        price: PriceTicks,
        qty: QuantityLots,
    ) -> Result<bool, MdError> {
        let Some(bar) = self.bars.get_mut(&(instrument_id, interval, open_ts)) else {
            return Ok(false);
        };
        bar.apply_late(price, qty)?;
        Ok(true)
    }
}

/// Builds closed bars from a stream of records.
#[derive(Debug, Clone)]
pub struct BarAggregator {
    intervals: Vec<BarInterval>,
    working: HashMap<(InstrumentId, BarInterval), OhlcvBar>,
    store: BarStore,
}

impl BarAggregator {
    /// Aggregates the given intervals (duplicates dropped, sorted by duration).
    #[must_use]
    pub fn new(intervals: impl IntoIterator<Item = BarInterval>) -> Self {
        let mut intervals: Vec<_> = intervals.into_iter().collect();
        intervals.sort_unstable();
        intervals.dedup();
        Self {
            intervals,
            working: HashMap::new(),
            store: BarStore::new(),
        }
    }

    /// Closed historical bars.
    #[must_use]
    pub const fn store(&self) -> &BarStore {
        &self.store
    }

    /// Consumes the aggregator and returns closed bars (working bars discarded
    /// unless [`Self::close_open`] was called).
    #[must_use]
    pub fn into_store(self) -> BarStore {
        self.store
    }

    /// Incomplete bar for the current bucket, if any.
    #[must_use]
    pub fn working(&self, instrument_id: InstrumentId, interval: BarInterval) -> Option<&OhlcvBar> {
        self.working.get(&(instrument_id, interval))
    }

    /// Applies one record. Non-trades are ignored. Returns bars that closed.
    ///
    /// # Errors
    ///
    /// Returns quantity / overflow errors.
    pub fn apply(&mut self, record: MdRecord) -> Result<Vec<OhlcvBar>, MdError> {
        if record.kind() != MdKind::Trade {
            return Ok(Vec::new());
        }
        if record.qty().lots() < 0 {
            return Err(MdError::InvalidQuantity);
        }
        let mut closed = Vec::new();
        let intervals = self.intervals.clone();
        for interval in intervals {
            if let Some(bar) = self.apply_interval(record, interval)? {
                closed.push(bar);
            }
        }
        closed.sort_by_key(|b| (b.interval, b.open_ts));
        Ok(closed)
    }

    /// Closes every working bar into the store (end of replay).
    pub fn close_open(&mut self) -> Vec<OhlcvBar> {
        let mut closed: Vec<_> = self.working.drain().map(|(_, bar)| bar).collect();
        closed.sort_by_key(|b| (b.instrument_id, b.interval, b.open_ts));
        for bar in &closed {
            self.store.insert(*bar);
        }
        closed
    }

    /// Replays a journal into a new aggregator and finalizes working bars.
    ///
    /// # Errors
    ///
    /// Returns the first apply error.
    pub fn from_journal(
        journal: &MdJournal,
        intervals: impl IntoIterator<Item = BarInterval>,
    ) -> Result<Self, MdError> {
        let mut agg = Self::new(intervals);
        for record in journal.records() {
            agg.apply(*record)?;
        }
        agg.close_open();
        Ok(agg)
    }

    fn apply_interval(
        &mut self,
        record: MdRecord,
        interval: BarInterval,
    ) -> Result<Option<OhlcvBar>, MdError> {
        let instrument_id = record.instrument_id();
        let key = (instrument_id, interval);
        let bucket = interval.bucket(record.ts_logical());
        match self.working.get(&key).copied() {
            Some(mut working) if working.open_ts == bucket => {
                working.apply_print(record.price(), record.qty())?;
                self.working.insert(key, working);
                Ok(None)
            }
            Some(working) if bucket > working.open_ts => {
                self.store.insert(working);
                self.working.insert(
                    key,
                    OhlcvBar::new(
                        instrument_id,
                        interval,
                        bucket,
                        record.price(),
                        record.qty(),
                    ),
                );
                Ok(Some(working))
            }
            Some(_) => {
                self.apply_historical(instrument_id, interval, bucket, record)?;
                Ok(None)
            }
            None => {
                if self.store.apply_late(
                    instrument_id,
                    interval,
                    bucket,
                    record.price(),
                    record.qty(),
                )? {
                    return Ok(None);
                }
                self.working.insert(
                    key,
                    OhlcvBar::new(
                        instrument_id,
                        interval,
                        bucket,
                        record.price(),
                        record.qty(),
                    ),
                );
                Ok(None)
            }
        }
    }

    fn apply_historical(
        &mut self,
        instrument_id: InstrumentId,
        interval: BarInterval,
        bucket: u64,
        record: MdRecord,
    ) -> Result<(), MdError> {
        if self.store.apply_late(
            instrument_id,
            interval,
            bucket,
            record.price(),
            record.qty(),
        )? {
            return Ok(());
        }
        self.store.insert(OhlcvBar::new(
            instrument_id,
            interval,
            bucket,
            record.price(),
            record.qty(),
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::MdRecord;
    use shinrai_instruments::PriceTicks;

    fn trade(inst: InstrumentId, seq: u64, ts: u64, px: i64, qty: i64) -> MdRecord {
        MdRecord::new(inst, seq, ts, MdKind::Trade, PriceTicks::from_scaled(px))
            .with_qty(QuantityLots::from_lots(qty))
    }

    #[test]
    fn known_fixture_two_five_tick_bars() {
        let inst = InstrumentId::from_u64(1);
        let interval = BarInterval::try_new(5).expect("i");
        let journal = MdJournal::from_records([
            trade(inst, 1, 0, 100, 1),
            trade(inst, 2, 1, 110, 2),
            trade(inst, 3, 5, 105, 1),
            trade(inst, 4, 6, 90, 3),
        ])
        .expect("j");
        let agg = BarAggregator::from_journal(&journal, [interval]).expect("agg");
        let bars: Vec<_> = agg.store().series(inst, interval).copied().collect();
        assert_eq!(bars.len(), 2);

        assert_eq!(bars[0].open_ts(), 0);
        assert_eq!(bars[0].open().scaled(), 100);
        assert_eq!(bars[0].high().scaled(), 110);
        assert_eq!(bars[0].low().scaled(), 100);
        assert_eq!(bars[0].close().scaled(), 110);
        assert_eq!(bars[0].volume().lots(), 3);
        assert_eq!(bars[0].trade_count(), 2);

        assert_eq!(bars[1].open_ts(), 5);
        assert_eq!(bars[1].open().scaled(), 105);
        assert_eq!(bars[1].high().scaled(), 105);
        assert_eq!(bars[1].low().scaled(), 90);
        assert_eq!(bars[1].close().scaled(), 90);
        assert_eq!(bars[1].volume().lots(), 4);
        assert_eq!(bars[1].trade_count(), 2);
        bars[0].assert_invariants().expect("b0");
        bars[1].assert_invariants().expect("b1");
    }

    #[test]
    fn bbo_does_not_form_candles() {
        let inst = InstrumentId::from_u64(1);
        let mut agg = BarAggregator::new([BarInterval::SECOND]);
        let bbo = MdRecord::new(inst, 1, 0, MdKind::Bbo, PriceTicks::from_scaled(100))
            .with_qty(QuantityLots::from_lots(9));
        assert!(agg.apply(bbo).expect("a").is_empty());
        assert!(agg.store().is_empty());
        assert!(agg.working(inst, BarInterval::SECOND).is_none());
    }

    #[test]
    fn empty_bucket_is_not_synthesized() {
        let inst = InstrumentId::from_u64(1);
        let interval = BarInterval::try_new(10).expect("i");
        let journal =
            MdJournal::from_records([trade(inst, 1, 0, 100, 1), trade(inst, 2, 25, 101, 1)])
                .expect("j");
        let agg = BarAggregator::from_journal(&journal, [interval]).expect("agg");
        let bars: Vec<_> = agg.store().series(inst, interval).collect();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].open_ts(), 0);
        assert_eq!(bars[1].open_ts(), 20);
    }

    #[test]
    fn utc_minute_bucket() {
        let interval = BarInterval::MINUTE;
        assert_eq!(interval.bucket(1_700_000_059), 1_700_000_040);
        assert_eq!(interval.bucket(1_700_000_040), 1_700_000_040);
    }

    #[test]
    fn late_print_amends_volume_not_close() {
        let inst = InstrumentId::from_u64(1);
        let interval = BarInterval::try_new(10).expect("i");
        let mut agg = BarAggregator::new([interval]);
        agg.apply(trade(inst, 1, 20, 100, 1)).expect("t1");
        agg.apply(trade(inst, 2, 5, 80, 2)).expect("late");
        agg.apply(trade(inst, 3, 21, 110, 1)).expect("t3");
        agg.close_open();
        let early = agg.store().get(inst, interval, 0).expect("0");
        assert_eq!(early.open().scaled(), 80);
        assert_eq!(early.close().scaled(), 80);
        assert_eq!(early.volume().lots(), 2);
        let later = agg.store().get(inst, interval, 20).expect("20");
        assert_eq!(later.open().scaled(), 100);
        assert_eq!(later.close().scaled(), 110);
        assert_eq!(later.high().scaled(), 110);
        assert_eq!(later.volume().lots(), 2);
    }

    #[test]
    fn rejects_zero_interval() {
        assert_eq!(
            BarInterval::try_new(0).expect_err("z"),
            MdError::InvalidInterval
        );
    }
}
