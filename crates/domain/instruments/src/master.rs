//! In-memory golden-copy instrument master.

use std::collections::HashMap;

use crate::error::InstrumentError;
use crate::ids::{ExternalId, InstrumentId};
use crate::instrument::Instrument;

/// Authoritative in-memory instrument store with alias resolution.
#[derive(Debug, Default, Clone)]
pub struct InstrumentMaster {
    by_id: HashMap<InstrumentId, Instrument>,
    by_alias: HashMap<ExternalId, InstrumentId>,
}

impl InstrumentMaster {
    /// Creates an empty master.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts an instrument and indexes all of its aliases.
    ///
    /// # Errors
    ///
    /// Returns duplicate id / alias errors.
    pub fn insert(&mut self, instrument: Instrument) -> Result<(), InstrumentError> {
        let id = instrument.id();
        if self.by_id.contains_key(&id) {
            return Err(InstrumentError::DuplicateInstrument { id });
        }
        for alias in instrument.identifiers() {
            if let Some(existing) = self.by_alias.get(alias) {
                return Err(InstrumentError::DuplicateAlias {
                    existing: *existing,
                });
            }
        }
        for alias in instrument.identifiers() {
            self.by_alias.insert(alias.clone(), id);
        }
        self.by_id.insert(id, instrument);
        Ok(())
    }

    /// Resolves an external alias to an internal id.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::UnknownAlias`] if not found.
    pub fn resolve_alias(&self, alias: &ExternalId) -> Result<InstrumentId, InstrumentError> {
        self.by_alias
            .get(alias)
            .copied()
            .ok_or(InstrumentError::UnknownAlias)
    }

    /// Returns an instrument by internal id.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::UnknownInstrument`] if missing.
    pub fn get(&self, id: InstrumentId) -> Result<&Instrument, InstrumentError> {
        self.by_id
            .get(&id)
            .ok_or(InstrumentError::UnknownInstrument { id })
    }

    /// Number of instruments in the master.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Returns true if the master has no instruments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Iterates all instruments.
    pub fn iter(&self) -> impl Iterator<Item = &Instrument> {
        self.by_id.values()
    }

    /// Returns a deterministic sorted list of ids (for snapshot tests).
    #[must_use]
    pub fn sorted_ids(&self) -> Vec<InstrumentId> {
        let mut ids: Vec<_> = self.by_id.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::aapl;
    use crate::ids::{ExternalId, IdType};

    #[test]
    fn resolves_ticker_and_isin() {
        let mut master = InstrumentMaster::new();
        master.insert(aapl()).expect("insert");
        let ticker = ExternalId::ticker("AAPL").expect("ticker");
        let isin = ExternalId::isin("US0378331005").expect("isin");
        assert_eq!(master.resolve_alias(&ticker).expect("t"), aapl().id());
        assert_eq!(master.resolve_alias(&isin).expect("i"), aapl().id());
    }

    #[test]
    fn rejects_duplicate_alias() {
        let mut master = InstrumentMaster::new();
        master.insert(aapl()).expect("aapl");
        let dup = Instrument::new(
            crate::ids::InstrumentId::from_u64(99),
            "DUP",
            crate::types::AssetClass::Equity,
            crate::types::InstrumentType::CommonStock,
            shinrai_money::Currency::usd(),
            None,
            aapl().tick_table().clone(),
            aapl().lot_spec(),
            1,
            crate::types::InstrumentStatus::Active,
            Some("XNAS".into()),
            vec![ExternalId::new(IdType::Ticker, "AAPL", None).expect("alias")],
        )
        .expect("dup");
        let err = master.insert(dup).expect_err("dup alias");
        assert!(matches!(err, InstrumentError::DuplicateAlias { .. }));
    }
}
