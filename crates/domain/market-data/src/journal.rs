//! Append-only market-data journal.

use crate::error::MdError;
use crate::record::MdRecord;

/// In-memory append-only market-data log (Phase 1).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MdJournal {
    records: Vec<MdRecord>,
}

impl MdJournal {
    /// Empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Appends a validated record.
    ///
    /// # Errors
    ///
    /// Returns [`MdError::InvalidSequence`] if `seq == 0`.
    pub fn append(&mut self, record: MdRecord) -> Result<(), MdError> {
        record.validate()?;
        self.records.push(record);
        Ok(())
    }

    /// Records in insertion order.
    #[must_use]
    pub fn records(&self) -> &[MdRecord] {
        &self.records
    }

    /// Builds a journal from an iterator of records.
    ///
    /// # Errors
    ///
    /// Returns the first validation error.
    pub fn from_records(records: impl IntoIterator<Item = MdRecord>) -> Result<Self, MdError> {
        let mut journal = Self::new();
        for r in records {
            journal.append(r)?;
        }
        Ok(journal)
    }
}
