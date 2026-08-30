//! Deterministic synthetic tick generator (seeded LCG).

use shinrai_instruments::{InstrumentId, PriceTicks};

use crate::error::MdError;
use crate::journal::MdJournal;
use crate::record::{MdKind, MdRecord};

/// Simple seeded pseudo-random feed for Phase 1 replay tests.
#[derive(Debug, Clone)]
pub struct SyntheticFeed {
    seed: u64,
    instrument_id: InstrumentId,
    seq: u64,
    ts: u64,
    price: i64,
}

impl SyntheticFeed {
    /// Creates a generator with the given seed and instrument.
    #[must_use]
    pub const fn new(seed: u64, instrument_id: InstrumentId, start_price: i64) -> Self {
        Self {
            seed,
            instrument_id,
            seq: 0,
            ts: 0,
            price: start_price,
        }
    }

    /// Emits the next trade tick.
    pub fn next_trade(&mut self) -> MdRecord {
        self.seq = self.seq.saturating_add(1);
        self.ts = self.ts.saturating_add(1);
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let delta = i64::from((self.seed % 5) as u8).saturating_sub(2);
        self.price = self.price.saturating_add(delta);
        MdRecord::new(
            self.instrument_id,
            self.seq,
            self.ts,
            MdKind::Trade,
            PriceTicks::from_scaled(self.price),
        )
    }

    /// Records `count` synthetic trades into a new journal.
    ///
    /// # Errors
    ///
    /// Returns validation errors (should not happen).
    pub fn record_trades(seed: u64, instrument_id: InstrumentId, count: usize) -> Result<MdJournal, MdError> {
        let mut feed = Self::new(seed, instrument_id, 10_000);
        let mut journal = MdJournal::new();
        for _ in 0..count {
            journal.append(feed.next_trade())?;
        }
        Ok(journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::replay;
    use shinrai_instruments::InstrumentId;

    #[test]
    fn same_seed_same_digest() {
        let inst = InstrumentId::from_u64(42);
        let j1 = SyntheticFeed::record_trades(123, inst, 50).expect("j1");
        let j2 = SyntheticFeed::record_trades(123, inst, 50).expect("j2");
        assert_eq!(replay(&j1).digest, replay(&j2).digest);
    }
}
