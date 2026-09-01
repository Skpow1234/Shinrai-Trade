//! In-memory mark prices for portfolio valuation.

use std::collections::HashMap;

use shinrai_instruments::{InstrumentId, PriceTicks};

/// Last mark per instrument (updated on fills or external feed).
#[derive(Debug, Default, Clone)]
pub struct MarkStore {
    marks: HashMap<InstrumentId, PriceTicks>,
}

impl MarkStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a mark (overwrites).
    pub fn set(&mut self, instrument: InstrumentId, price: PriceTicks) {
        if price.scaled() > 0 {
            self.marks.insert(instrument, price);
        }
    }

    /// Returns the stored mark, if any.
    #[must_use]
    pub fn get(&self, instrument: InstrumentId) -> Option<PriceTicks> {
        self.marks.get(&instrument).copied()
    }

    /// All stored marks.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<InstrumentId, PriceTicks> {
        self.marks.clone()
    }

    /// Merges manual marks over stored marks (manual wins on conflict).
    #[must_use]
    pub fn merged_with<S: std::hash::BuildHasher>(
        &self,
        manual: &HashMap<InstrumentId, PriceTicks, S>,
    ) -> HashMap<InstrumentId, PriceTicks> {
        let mut out = self.marks.clone();
        for (id, px) in manual {
            out.insert(*id, *px);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_overrides_stored() {
        let mut store = MarkStore::new();
        let id = InstrumentId::from_u64(1);
        store.set(id, PriceTicks::from_scaled(100));
        let mut manual = HashMap::new();
        manual.insert(id, PriceTicks::from_scaled(200));
        let merged = store.merged_with(&manual);
        assert_eq!(merged.get(&id).expect("p").scaled(), 200);
    }
}
