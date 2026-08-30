//! Normalized market-data records.

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};

/// Kind of market-data message (Phase 1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MdKind {
    /// Last trade / print.
    Trade,
    /// Best bid/offer update (payload uses mid in `price`).
    Bbo,
    /// Order book delta (not expanded in Phase 1).
    Book,
    /// Instrument / feed status.
    Status,
    /// Snapshot boundary marker (resets gap recovery).
    Snapshot,
}

/// One append-only market-data event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdRecord {
    instrument_id: InstrumentId,
    seq: u64,
    ts_logical: u64,
    kind: MdKind,
    price: PriceTicks,
    qty: QuantityLots,
}

impl MdRecord {
    /// Creates a record.
    ///
    /// # Panics
    ///
    /// Debug builds panic if `seq == 0`.
    #[must_use]
    pub const fn new(
        instrument_id: InstrumentId,
        seq: u64,
        ts_logical: u64,
        kind: MdKind,
        price: PriceTicks,
    ) -> Self {
        Self {
            instrument_id,
            seq,
            ts_logical,
            kind,
            price,
            qty: QuantityLots::from_lots(0),
        }
    }

    /// Sets trade quantity (lot units). Ignored for non-trade kinds in charts.
    #[must_use]
    pub const fn with_qty(self, qty: QuantityLots) -> Self {
        Self { qty, ..self }
    }

    /// Instrument.
    #[must_use]
    pub const fn instrument_id(self) -> InstrumentId {
        self.instrument_id
    }

    /// Per-instrument vendor sequence (must be positive).
    #[must_use]
    pub const fn seq(self) -> u64 {
        self.seq
    }

    /// Logical event time (simulator clock tick, not wall clock).
    #[must_use]
    pub const fn ts_logical(self) -> u64 {
        self.ts_logical
    }

    /// Message kind.
    #[must_use]
    pub const fn kind(self) -> MdKind {
        self.kind
    }

    /// Primary price payload (trade, mid, or status sentinel).
    #[must_use]
    pub const fn price(self) -> PriceTicks {
        self.price
    }

    /// Trade quantity in lot units (`0` when unknown or not a print).
    #[must_use]
    pub const fn qty(self) -> QuantityLots {
        self.qty
    }

    /// Validates sequence is non-zero.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MdError::InvalidSequence`] if `seq == 0`.
    pub fn validate(&self) -> Result<(), crate::MdError> {
        if self.seq == 0 {
            return Err(crate::MdError::InvalidSequence);
        }
        Ok(())
    }
}
