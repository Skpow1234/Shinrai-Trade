//! Historical bar and trade queries (pagination, time range).

use shinrai_instruments::InstrumentId;

use crate::bar::{BarAggregator, BarInterval, BarStore, OhlcvBar};
use crate::error::MdError;
use crate::journal::MdJournal;
use crate::record::{MdKind, MdRecord};

/// Default page size for historical queries.
pub const DEFAULT_PAGE_LIMIT: usize = 100;

/// Maximum page size for historical queries.
pub const MAX_PAGE_LIMIT: usize = 1_000;

/// Pagination parameters (cursor is opaque to HTTP: bar `open_ts` or trade `seq`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageParams {
    limit: usize,
    cursor: Option<u64>,
}

impl PageParams {
    /// Builds page params, clamping `limit` to [`MAX_PAGE_LIMIT`].
    #[must_use]
    pub fn new(limit: Option<usize>, cursor: Option<u64>) -> Self {
        let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT);
        Self { limit, cursor }
    }

    /// Page size.
    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }

    /// Start after this cursor (exclusive).
    #[must_use]
    pub const fn cursor(self) -> Option<u64> {
        self.cursor
    }
}

/// Bar history filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarHistoryQuery {
    instrument_id: InstrumentId,
    interval: BarInterval,
    start_ts: Option<u64>,
    end_ts: Option<u64>,
    page: PageParams,
}

impl BarHistoryQuery {
    /// Creates a bar query.
    #[must_use]
    pub const fn new(
        instrument_id: InstrumentId,
        interval: BarInterval,
        start_ts: Option<u64>,
        end_ts: Option<u64>,
        page: PageParams,
    ) -> Self {
        Self {
            instrument_id,
            interval,
            start_ts,
            end_ts,
            page,
        }
    }
}

/// Trade history filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeHistoryQuery {
    instrument_id: InstrumentId,
    start_ts: Option<u64>,
    end_ts: Option<u64>,
    page: PageParams,
}

impl TradeHistoryQuery {
    /// Creates a trade query.
    #[must_use]
    pub const fn new(
        instrument_id: InstrumentId,
        start_ts: Option<u64>,
        end_ts: Option<u64>,
        page: PageParams,
    ) -> Self {
        Self {
            instrument_id,
            start_ts,
            end_ts,
            page,
        }
    }
}

/// One page of historical bars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarHistoryPage {
    bars: Vec<OhlcvBar>,
    next_cursor: Option<u64>,
}

impl BarHistoryPage {
    /// Bars in ascending `open_ts` order.
    #[must_use]
    pub fn bars(&self) -> &[OhlcvBar] {
        &self.bars
    }

    /// Pass as the next request `cursor` when present.
    #[must_use]
    pub const fn next_cursor(&self) -> Option<u64> {
        self.next_cursor
    }
}

/// One page of trade prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeHistoryPage {
    trades: Vec<MdRecord>,
    next_cursor: Option<u64>,
}

impl TradeHistoryPage {
    /// Trades in ascending `(ts_logical, seq)` order.
    #[must_use]
    pub fn trades(&self) -> &[MdRecord] {
        &self.trades
    }

    /// Pass as the next request `cursor` (last trade `seq`).
    #[must_use]
    pub const fn next_cursor(&self) -> Option<u64> {
        self.next_cursor
    }
}

/// Closed bars plus an open working bucket, fed by trade prints.
#[derive(Debug, Clone)]
pub struct HistoricalArchive {
    journal: MdJournal,
    bars: BarAggregator,
}

impl HistoricalArchive {
    /// Creates an archive aggregating the given bar intervals.
    #[must_use]
    pub fn new(intervals: impl IntoIterator<Item = BarInterval>) -> Self {
        Self {
            journal: MdJournal::new(),
            bars: BarAggregator::new(intervals),
        }
    }

    /// Default UTC second/minute/hour/day buckets.
    #[must_use]
    pub fn default_intervals() -> Self {
        Self::new([
            BarInterval::SECOND,
            BarInterval::MINUTE,
            BarInterval::HOUR,
            BarInterval::DAY,
        ])
    }

    /// Trade journal (all kinds stored; queries filter to prints).
    #[must_use]
    pub fn journal(&self) -> &MdJournal {
        &self.journal
    }

    /// Bar aggregator (closed store + working buckets).
    #[must_use]
    pub fn bars(&self) -> &BarAggregator {
        &self.bars
    }

    /// Appends a record: trades update OHLCV; other kinds are stored only.
    ///
    /// # Errors
    ///
    /// Returns journal / aggregation errors.
    pub fn ingest(&mut self, record: MdRecord) -> Result<(), MdError> {
        record.validate()?;
        if record.kind() == MdKind::Trade {
            self.bars.apply(record)?;
        }
        self.journal.append(record)
    }

    /// Loads a journal and finalizes working bars into the store.
    ///
    /// # Errors
    ///
    /// Returns the first ingest error.
    pub fn from_journal(
        journal: &MdJournal,
        intervals: impl IntoIterator<Item = BarInterval>,
    ) -> Result<Self, MdError> {
        let mut archive = Self::new(intervals);
        for record in journal.records() {
            archive.ingest(*record)?;
        }
        archive.bars.close_open();
        Ok(archive)
    }

    /// Paginated closed + in-progress bars for an instrument/interval.
    #[must_use]
    pub fn query_bars(&self, query: BarHistoryQuery) -> BarHistoryPage {
        page_bars(
            self.bars.store(),
            self.bars.working(query.instrument_id, query.interval),
            &query,
        )
    }

    /// Paginated trade prints.
    #[must_use]
    pub fn query_trades(&self, query: TradeHistoryQuery) -> TradeHistoryPage {
        page_trades(self.journal.records(), &query)
    }

    /// Replays a journal into this archive and closes working bars.
    ///
    /// # Errors
    ///
    /// Returns the first ingest error.
    pub fn load_journal(&mut self, journal: &MdJournal) -> Result<(), MdError> {
        for record in journal.records() {
            self.ingest(*record)?;
        }
        self.bars.close_open();
        Ok(())
    }
}

fn page_bars(
    store: &BarStore,
    working: Option<&OhlcvBar>,
    query: &BarHistoryQuery,
) -> BarHistoryPage {
    let start = query.start_ts.unwrap_or(0);
    let end = query.end_ts.unwrap_or(u64::MAX);
    let cursor = query.page.cursor;

    let mut items: Vec<OhlcvBar> = store
        .series(query.instrument_id, query.interval)
        .filter(|b| {
            b.open_ts() >= start && b.open_ts() <= end && cursor.is_none_or(|c| b.open_ts() > c)
        })
        .copied()
        .collect();

    if let Some(open) = working {
        if open.instrument_id() == query.instrument_id
            && open.interval() == query.interval
            && open.open_ts() >= start
            && open.open_ts() <= end
            && cursor.is_none_or(|c| open.open_ts() > c)
            && !items.iter().any(|b| b.open_ts() == open.open_ts())
        {
            items.push(*open);
        }
    }

    items.sort_by_key(|b| b.open_ts());
    let has_more = items.len() > query.page.limit;
    items.truncate(query.page.limit);
    let next_cursor = if has_more {
        items.last().map(|b| b.open_ts())
    } else {
        None
    };
    BarHistoryPage {
        bars: items,
        next_cursor,
    }
}

fn page_trades(records: &[MdRecord], query: &TradeHistoryQuery) -> TradeHistoryPage {
    let start = query.start_ts.unwrap_or(0);
    let end = query.end_ts.unwrap_or(u64::MAX);
    let cursor = query.page.cursor;

    let mut items: Vec<MdRecord> = records
        .iter()
        .filter(|r| {
            r.kind() == MdKind::Trade
                && r.instrument_id() == query.instrument_id
                && r.ts_logical() >= start
                && r.ts_logical() <= end
                && cursor.is_none_or(|c| r.seq() > c)
        })
        .copied()
        .collect();

    items.sort_by(|a, b| {
        a.ts_logical()
            .cmp(&b.ts_logical())
            .then(a.seq().cmp(&b.seq()))
    });

    let has_more = items.len() > query.page.limit;
    items.truncate(query.page.limit);
    let next_cursor = if has_more {
        items.last().map(|t| t.seq())
    } else {
        None
    };
    TradeHistoryPage {
        trades: items,
        next_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::MdRecord;
    use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};

    fn trade(inst: InstrumentId, seq: u64, ts: u64, px: i64, qty: i64) -> MdRecord {
        MdRecord::new(inst, seq, ts, MdKind::Trade, PriceTicks::from_scaled(px))
            .with_qty(QuantityLots::from_lots(qty))
    }

    #[test]
    fn replay_journal_matches_bar_store() {
        let inst = InstrumentId::from_u64(3);
        let journal = MdJournal::from_records([
            trade(inst, 1, 60, 100, 1),
            trade(inst, 2, 61, 120, 2),
            trade(inst, 3, 119, 90, 1),
            trade(inst, 4, 120, 95, 4),
        ])
        .expect("journal");

        let direct = BarAggregator::from_journal(&journal, [BarInterval::MINUTE]).expect("direct");
        let archive =
            HistoricalArchive::from_journal(&journal, [BarInterval::MINUTE]).expect("archive");

        let direct_bars: Vec<_> = direct
            .store()
            .series(inst, BarInterval::MINUTE)
            .copied()
            .collect();
        let page = archive.query_bars(BarHistoryQuery::new(
            inst,
            BarInterval::MINUTE,
            None,
            None,
            PageParams::new(Some(100), None),
        ));
        assert_eq!(page.bars(), direct_bars.as_slice());
        assert!(page.next_cursor().is_none());
    }

    #[test]
    fn bar_pagination_by_open_ts() {
        let inst = InstrumentId::from_u64(1);
        let interval = BarInterval::try_new(10).expect("i");
        let journal = MdJournal::from_records([
            trade(inst, 1, 0, 100, 1),
            trade(inst, 2, 10, 101, 1),
            trade(inst, 3, 20, 102, 1),
            trade(inst, 4, 30, 103, 1),
        ])
        .expect("j");
        let archive = HistoricalArchive::from_journal(&journal, [interval]).expect("a");
        assert_eq!(
            archive.bars().store().series(inst, interval).count(),
            4,
            "expected four closed bars"
        );

        let p1 = archive.query_bars(BarHistoryQuery::new(
            inst,
            interval,
            None,
            None,
            PageParams::new(Some(2), None),
        ));
        assert_eq!(p1.bars().len(), 2);
        assert_eq!(p1.bars()[0].open_ts(), 0);
        assert_eq!(p1.bars()[1].open_ts(), 10);
        assert_eq!(p1.next_cursor(), Some(10));

        let p2 = archive.query_bars(BarHistoryQuery::new(
            inst,
            interval,
            None,
            None,
            PageParams::new(Some(2), p1.next_cursor()),
        ));
        assert_eq!(p2.bars().len(), 2);
        assert_eq!(p2.bars()[0].open_ts(), 20);
        assert_eq!(p2.bars()[1].open_ts(), 30);
        assert!(p2.next_cursor().is_none());
    }

    #[test]
    fn trade_pagination_by_seq() {
        let inst = InstrumentId::from_u64(1);
        let mut archive = HistoricalArchive::default_intervals();
        for seq in 1u64..=5 {
            let px = 100 + i64::try_from(seq).expect("seq fits i64");
            archive
                .ingest(trade(inst, seq, seq * 10, px, 1))
                .expect("ingest");
        }
        let p1 = archive.query_trades(TradeHistoryQuery::new(
            inst,
            None,
            None,
            PageParams::new(Some(2), None),
        ));
        assert_eq!(p1.trades().len(), 2);
        assert_eq!(p1.next_cursor(), Some(2));

        let p2 = archive.query_trades(TradeHistoryQuery::new(
            inst,
            None,
            None,
            PageParams::new(Some(10), p1.next_cursor()),
        ));
        assert_eq!(p2.trades().len(), 3);
        assert_eq!(p2.trades()[0].seq(), 3);
    }
}
