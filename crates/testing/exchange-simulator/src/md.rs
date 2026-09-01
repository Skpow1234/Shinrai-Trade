//! Minimal market-data ticks for gap-detection tests.

use shinrai_instruments::{InstrumentId, PriceTicks};

/// One synthetic market-data tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdTick {
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Per-instrument sequence.
    pub seq: u64,
    /// Last / mid price in ticks.
    pub price: PriceTicks,
}
