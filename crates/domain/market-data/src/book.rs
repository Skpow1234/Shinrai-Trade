//! Local L2 order book: snapshot + deltas, invalidate on gap, checksum rebuild.
//!
//! On a sequence gap the book is **cleared** and marked invalidated. Deltas are
//! not applied until a snapshot rebuilds it. After rebuild,
//! [`L2Book::checksum`] matches [`BookSnapshot::checksum`].

use std::collections::{BTreeMap, HashMap};

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};

use crate::checksum::{mix, mix_i64, mix_u64, FNV_OFFSET};
use crate::error::MdError;

/// Bid or ask side of the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookSide {
    /// Buy side (bids, highest first).
    Bid,
    /// Sell side (asks, lowest first).
    Ask,
}

/// One price level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookLevel {
    price: PriceTicks,
    qty: QuantityLots,
}

impl BookLevel {
    /// Creates a level.
    #[must_use]
    pub const fn new(price: PriceTicks, qty: QuantityLots) -> Self {
        Self { price, qty }
    }

    /// Price.
    #[must_use]
    pub const fn price(self) -> PriceTicks {
        self.price
    }

    /// Size in lots (`0` means delete when used as a delta).
    #[must_use]
    pub const fn qty(self) -> QuantityLots {
        self.qty
    }
}

/// One size update at a price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookChange {
    side: BookSide,
    price: PriceTicks,
    qty: QuantityLots,
}

impl BookChange {
    /// Creates a change. `qty == 0` deletes the level.
    #[must_use]
    pub const fn new(side: BookSide, price: PriceTicks, qty: QuantityLots) -> Self {
        Self { side, price, qty }
    }

    /// Side.
    #[must_use]
    pub const fn side(self) -> BookSide {
        self.side
    }

    /// Price.
    #[must_use]
    pub const fn price(self) -> PriceTicks {
        self.price
    }

    /// Size.
    #[must_use]
    pub const fn qty(self) -> QuantityLots {
        self.qty
    }
}

/// Full-book replacement (REST or WS snapshot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSnapshot {
    instrument_id: InstrumentId,
    seq: u64,
    ts_logical: u64,
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
}

impl BookSnapshot {
    /// Creates a snapshot.
    #[must_use]
    pub fn new(
        instrument_id: InstrumentId,
        seq: u64,
        ts_logical: u64,
        bids: Vec<BookLevel>,
        asks: Vec<BookLevel>,
    ) -> Self {
        Self {
            instrument_id,
            seq,
            ts_logical,
            bids,
            asks,
        }
    }

    /// Instrument.
    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument_id
    }

    /// Vendor sequence at the snapshot.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Logical time.
    #[must_use]
    pub const fn ts_logical(&self) -> u64 {
        self.ts_logical
    }

    /// Bid levels.
    #[must_use]
    pub fn bids(&self) -> &[BookLevel] {
        &self.bids
    }

    /// Ask levels.
    #[must_use]
    pub fn asks(&self) -> &[BookLevel] {
        &self.asks
    }

    /// Digest of this snapshot applied to an empty book.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        let mut book = L2Book::new(self.instrument_id);
        book.replace(self);
        book.checksum()
    }
}

/// Incremental book update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookDelta {
    instrument_id: InstrumentId,
    seq: Option<u64>,
    ts_logical: u64,
    changes: Vec<BookChange>,
}

impl BookDelta {
    /// Creates a delta. `seq` is `None` for unsequenced vendor channels.
    #[must_use]
    pub fn new(
        instrument_id: InstrumentId,
        seq: Option<u64>,
        ts_logical: u64,
        changes: Vec<BookChange>,
    ) -> Self {
        Self {
            instrument_id,
            seq,
            ts_logical,
            changes,
        }
    }

    /// Instrument.
    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument_id
    }

    /// Optional vendor sequence.
    #[must_use]
    pub const fn seq(&self) -> Option<u64> {
        self.seq
    }

    /// Logical time.
    #[must_use]
    pub const fn ts_logical(&self) -> u64 {
        self.ts_logical
    }

    /// Level changes.
    #[must_use]
    pub fn changes(&self) -> &[BookChange] {
        &self.changes
    }
}

/// Snapshot or delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookEvent {
    /// Replace the book.
    Snapshot(BookSnapshot),
    /// Apply level changes.
    Delta(BookDelta),
}

/// Health of a local L2 book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookStatus {
    /// No snapshot yet.
    Empty,
    /// Snapshot applied; deltas may be applied.
    Healthy,
    /// Gap or disconnect; book is cleared until snapshot rebuild.
    Invalidated,
}

/// Per-instrument aggregated L2 book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2Book {
    instrument_id: InstrumentId,
    status: BookStatus,
    seq: u64,
    bids: BTreeMap<PriceTicks, QuantityLots>,
    asks: BTreeMap<PriceTicks, QuantityLots>,
}

impl L2Book {
    /// Empty book waiting for a snapshot.
    #[must_use]
    pub fn new(instrument_id: InstrumentId) -> Self {
        Self {
            instrument_id,
            status: BookStatus::Empty,
            seq: 0,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    /// Instrument.
    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument_id
    }

    /// Book health (UI should hide levels when not [`BookStatus::Healthy`]).
    #[must_use]
    pub const fn status(&self) -> BookStatus {
        self.status
    }

    /// Last applied snapshot/delta sequence (`0` if none).
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Best bid, if any.
    #[must_use]
    pub fn best_bid(&self) -> Option<BookLevel> {
        self.bids
            .iter()
            .next_back()
            .map(|(price, qty)| BookLevel::new(*price, *qty))
    }

    /// Best ask, if any.
    #[must_use]
    pub fn best_ask(&self) -> Option<BookLevel> {
        self.asks
            .iter()
            .next()
            .map(|(price, qty)| BookLevel::new(*price, *qty))
    }

    /// Bids, best first (descending price).
    pub fn bids(&self) -> impl Iterator<Item = BookLevel> + '_ {
        self.bids
            .iter()
            .rev()
            .map(|(price, qty)| BookLevel::new(*price, *qty))
    }

    /// Asks, best first (ascending price).
    pub fn asks(&self) -> impl Iterator<Item = BookLevel> + '_ {
        self.asks
            .iter()
            .map(|(price, qty)| BookLevel::new(*price, *qty))
    }

    /// Clears levels and marks invalidated (gap / disconnect).
    pub fn invalidate(&mut self) {
        self.bids.clear();
        self.asks.clear();
        self.status = BookStatus::Invalidated;
    }

    /// FNV-1a digest of status, seq, and ordered levels.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        let mut hash = FNV_OFFSET;
        mix_u64(&mut hash, self.instrument_id.get());
        mix(
            &mut hash,
            match self.status {
                BookStatus::Empty => 0,
                BookStatus::Healthy => 1,
                BookStatus::Invalidated => 2,
            },
        );
        mix_u64(&mut hash, self.seq);
        mix_u64(&mut hash, self.bids.len() as u64);
        for (price, qty) in &self.bids {
            mix_i64(&mut hash, price.scaled());
            mix_i64(&mut hash, qty.lots());
        }
        mix_u64(&mut hash, self.asks.len() as u64);
        for (price, qty) in &self.asks {
            mix_i64(&mut hash, price.scaled());
            mix_i64(&mut hash, qty.lots());
        }
        hash
    }

    fn replace(&mut self, snap: &BookSnapshot) {
        self.bids.clear();
        self.asks.clear();
        for level in &snap.bids {
            self.set_level(BookSide::Bid, level.price, level.qty);
        }
        for level in &snap.asks {
            self.set_level(BookSide::Ask, level.price, level.qty);
        }
        self.seq = snap.seq;
        self.status = BookStatus::Healthy;
    }

    fn set_level(&mut self, side: BookSide, price: PriceTicks, qty: QuantityLots) {
        let map = match side {
            BookSide::Bid => &mut self.bids,
            BookSide::Ask => &mut self.asks,
        };
        if qty.lots() == 0 {
            map.remove(&price);
        } else {
            map.insert(price, qty);
        }
    }
}

/// Result of applying a book event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookApplyOutcome {
    /// Snapshot or delta applied.
    Applied {
        /// Current book digest.
        checksum: u64,
    },
    /// Duplicate sequenced delta ignored.
    Duplicate,
    /// Book is invalidated; delta skipped.
    IgnoredInvalidated,
    /// Sequenced delta jumped; book cleared.
    GapInvalidated {
        /// Expected sequence.
        expected: u64,
        /// Received sequence.
        got: u64,
    },
}

/// Local books for many instruments.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BookEngine {
    books: HashMap<InstrumentId, L2Book>,
}

impl BookEngine {
    /// Empty engine.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Book for an instrument, if any event has been seen.
    #[must_use]
    pub fn book(&self, instrument_id: InstrumentId) -> Option<&L2Book> {
        self.books.get(&instrument_id)
    }

    /// Invalidates (clears) the book until a snapshot.
    pub fn invalidate(&mut self, instrument_id: InstrumentId) {
        self.books
            .entry(instrument_id)
            .or_insert_with(|| L2Book::new(instrument_id))
            .invalidate();
    }

    /// Applies a snapshot or delta.
    ///
    /// # Errors
    ///
    /// Returns [`MdError::InvalidSequence`] if a snapshot seq is 0, or
    /// [`MdError::InvalidQuantity`] if a level size is negative.
    pub fn apply(&mut self, event: &BookEvent) -> Result<BookApplyOutcome, MdError> {
        match event {
            BookEvent::Snapshot(snap) => self.apply_snapshot(snap),
            BookEvent::Delta(delta) => self.apply_delta(delta),
        }
    }

    fn apply_snapshot(&mut self, snap: &BookSnapshot) -> Result<BookApplyOutcome, MdError> {
        if snap.seq == 0 {
            return Err(MdError::InvalidSequence);
        }
        validate_levels(&snap.bids)?;
        validate_levels(&snap.asks)?;
        let book = self
            .books
            .entry(snap.instrument_id)
            .or_insert_with(|| L2Book::new(snap.instrument_id));
        book.replace(snap);
        Ok(BookApplyOutcome::Applied {
            checksum: book.checksum(),
        })
    }

    fn apply_delta(&mut self, delta: &BookDelta) -> Result<BookApplyOutcome, MdError> {
        for change in &delta.changes {
            if change.qty.lots() < 0 {
                return Err(MdError::InvalidQuantity);
            }
        }
        let book = self
            .books
            .entry(delta.instrument_id)
            .or_insert_with(|| L2Book::new(delta.instrument_id));
        if book.status != BookStatus::Healthy {
            return Ok(BookApplyOutcome::IgnoredInvalidated);
        }
        if let Some(seq) = delta.seq {
            if seq == 0 {
                return Err(MdError::InvalidSequence);
            }
            let expected = book.seq.saturating_add(1);
            if seq < expected {
                return Ok(BookApplyOutcome::Duplicate);
            }
            if seq > expected {
                book.invalidate();
                return Ok(BookApplyOutcome::GapInvalidated { expected, got: seq });
            }
            book.seq = seq;
        }
        for change in &delta.changes {
            book.set_level(change.side, change.price, change.qty);
        }
        Ok(BookApplyOutcome::Applied {
            checksum: book.checksum(),
        })
    }
}

fn validate_levels(levels: &[BookLevel]) -> Result<(), MdError> {
    for level in levels {
        if level.qty.lots() < 0 {
            return Err(MdError::InvalidQuantity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::PriceTicks;

    fn px(v: i64) -> PriceTicks {
        PriceTicks::from_scaled(v)
    }

    fn qty(v: i64) -> QuantityLots {
        QuantityLots::from_lots(v)
    }

    fn inst() -> InstrumentId {
        InstrumentId::from_u64(3)
    }

    fn snap(seq: u64, bids: Vec<(i64, i64)>, asks: Vec<(i64, i64)>) -> BookSnapshot {
        BookSnapshot::new(
            inst(),
            seq,
            0,
            bids.into_iter()
                .map(|(p, q)| BookLevel::new(px(p), qty(q)))
                .collect(),
            asks.into_iter()
                .map(|(p, q)| BookLevel::new(px(p), qty(q)))
                .collect(),
        )
    }

    #[test]
    fn snapshot_checksum_matches_book() {
        let snap = snap(10, vec![(100, 5), (99, 2)], vec![(101, 3)]);
        let expected = snap.checksum();
        let mut engine = BookEngine::new();
        let outcome = engine
            .apply(&BookEvent::Snapshot(snap.clone()))
            .expect("apply");
        assert_eq!(outcome, BookApplyOutcome::Applied { checksum: expected });
        let book = engine.book(inst()).expect("book");
        assert_eq!(book.status(), BookStatus::Healthy);
        assert_eq!(book.checksum(), expected);
        assert_eq!(book.best_bid().expect("bb").price(), px(100));
        assert_eq!(book.best_ask().expect("ba").price(), px(101));
    }

    #[test]
    fn delta_updates_and_deletes() {
        let mut engine = BookEngine::new();
        engine
            .apply(&BookEvent::Snapshot(snap(
                1,
                vec![(100, 5)],
                vec![(101, 3)],
            )))
            .expect("s");
        engine
            .apply(&BookEvent::Delta(BookDelta::new(
                inst(),
                Some(2),
                1,
                vec![
                    BookChange::new(BookSide::Bid, px(100), qty(8)),
                    BookChange::new(BookSide::Ask, px(101), qty(0)),
                    BookChange::new(BookSide::Ask, px(102), qty(1)),
                ],
            )))
            .expect("d");
        let book = engine.book(inst()).expect("book");
        assert_eq!(book.best_bid().expect("bb").qty(), qty(8));
        assert_eq!(book.best_ask().expect("ba").price(), px(102));
        assert_eq!(book.asks().count(), 1);
    }

    #[test]
    fn gap_invalidates_and_delta_skipped_until_snapshot() {
        let mut engine = BookEngine::new();
        engine
            .apply(&BookEvent::Snapshot(snap(
                1,
                vec![(100, 1)],
                vec![(101, 1)],
            )))
            .expect("s");
        let gap = engine
            .apply(&BookEvent::Delta(BookDelta::new(
                inst(),
                Some(4),
                2,
                vec![BookChange::new(BookSide::Bid, px(100), qty(9))],
            )))
            .expect("gap");
        assert!(matches!(
            gap,
            BookApplyOutcome::GapInvalidated {
                expected: 2,
                got: 4
            }
        ));
        let book = engine.book(inst()).expect("book");
        assert_eq!(book.status(), BookStatus::Invalidated);
        assert!(book.best_bid().is_none());
        let skipped = engine
            .apply(&BookEvent::Delta(BookDelta::new(
                inst(),
                Some(5),
                3,
                vec![BookChange::new(BookSide::Bid, px(99), qty(1))],
            )))
            .expect("skip");
        assert_eq!(skipped, BookApplyOutcome::IgnoredInvalidated);
        let rebuild = snap(5, vec![(98, 4)], vec![(103, 2)]);
        let checksum = rebuild.checksum();
        engine
            .apply(&BookEvent::Snapshot(rebuild))
            .expect("rebuild");
        let book = engine.book(inst()).expect("book");
        assert_eq!(book.status(), BookStatus::Healthy);
        assert_eq!(book.checksum(), checksum);
        assert_eq!(book.best_bid().expect("bb").price(), px(98));
    }

    #[test]
    fn unsequenced_delta_applies_when_healthy() {
        let mut engine = BookEngine::new();
        engine
            .apply(&BookEvent::Snapshot(snap(
                10,
                vec![(100, 1)],
                vec![(101, 1)],
            )))
            .expect("s");
        engine
            .apply(&BookEvent::Delta(BookDelta::new(
                inst(),
                None,
                1,
                vec![BookChange::new(BookSide::Bid, px(100), qty(7))],
            )))
            .expect("d");
        assert_eq!(
            engine
                .book(inst())
                .expect("b")
                .best_bid()
                .expect("bb")
                .qty(),
            qty(7)
        );
    }

    #[test]
    fn rebuild_replay_same_checksum() {
        let snap = snap(3, vec![(100, 2), (99, 1)], vec![(101, 4)]);
        let deltas = [
            BookDelta::new(
                inst(),
                Some(4),
                1,
                vec![BookChange::new(BookSide::Bid, px(100), qty(3))],
            ),
            BookDelta::new(
                inst(),
                Some(5),
                2,
                vec![BookChange::new(BookSide::Ask, px(102), qty(1))],
            ),
        ];
        let mut live = BookEngine::new();
        live.apply(&BookEvent::Snapshot(snap.clone())).expect("s");
        for d in &deltas {
            live.apply(&BookEvent::Delta(d.clone())).expect("d");
        }
        let mut replay = BookEngine::new();
        replay.apply(&BookEvent::Snapshot(snap)).expect("s2");
        for d in &deltas {
            replay.apply(&BookEvent::Delta(d.clone())).expect("d2");
        }
        assert_eq!(
            live.book(inst()).expect("a").checksum(),
            replay.book(inst()).expect("b").checksum()
        );
    }
}
