//! Per-instrument consumer state and strict sequential apply.

use std::collections::HashMap;

use shinrai_instruments::{InstrumentId, PriceTicks};

use crate::error::MdError;
use crate::record::{MdKind, MdRecord};

/// Health of a market-data feed for one instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedStatus {
    /// Sequences are contiguous; messages are applied.
    Healthy,
    /// Gap detected; messages are not applied until snapshot recovery.
    Degraded {
        /// First missing sequence number.
        missing_from: u64,
    },
}

/// Deterministic projection built by replaying a journal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MdConsumerState {
    next_seq: HashMap<InstrumentId, u64>,
    last_price: HashMap<InstrumentId, PriceTicks>,
    status: HashMap<InstrumentId, FeedStatus>,
    applied: u64,
    duplicates: u64,
    gaps: u64,
}

impl MdConsumerState {
    /// Fresh consumer (expects first seq == 1 per instrument).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Expected next sequence for an instrument.
    ///
    /// Returns `1` if the instrument has never been applied. Venue feeds whose
    /// first sequence is not `1` still apply on first sight (see [`Self::apply`]).
    #[must_use]
    pub fn expected_seq(&self, instrument_id: InstrumentId) -> u64 {
        self.next_seq.get(&instrument_id).copied().unwrap_or(1)
    }

    /// Returns true if at least one sequenced record has been applied.
    #[must_use]
    pub fn is_synced(&self, instrument_id: InstrumentId) -> bool {
        self.next_seq.contains_key(&instrument_id)
    }

    /// Marks the feed degraded until a snapshot arrives (disconnect / resume).
    pub fn mark_degraded(&mut self, instrument_id: InstrumentId) {
        let missing_from = self.next_seq.get(&instrument_id).copied().unwrap_or(1);
        self.gaps += 1;
        self.status
            .insert(instrument_id, FeedStatus::Degraded { missing_from });
    }

    /// Feed status (healthy if unseen).
    #[must_use]
    pub fn feed_status(&self, instrument_id: InstrumentId) -> FeedStatus {
        self.status
            .get(&instrument_id)
            .copied()
            .unwrap_or(FeedStatus::Healthy)
    }

    /// Last applied price for an instrument, if any.
    #[must_use]
    pub fn last_price(&self, instrument_id: InstrumentId) -> Option<PriceTicks> {
        self.last_price.get(&instrument_id).copied()
    }

    /// Total applied messages.
    #[must_use]
    pub const fn applied_count(&self) -> u64 {
        self.applied
    }

    /// Duplicate messages ignored.
    #[must_use]
    pub const fn duplicate_count(&self) -> u64 {
        self.duplicates
    }

    /// Gap events detected.
    #[must_use]
    pub const fn gap_count(&self) -> u64 {
        self.gaps
    }

    /// Applies one record using strict in-order policy:
    ///
    /// - First record for an instrument (any `seq > 0`) → apply, unless degraded
    /// - `seq == expected` → apply (unless degraded and not snapshot)
    /// - `seq < expected` → duplicate, ignore
    /// - `seq > expected` → gap, mark degraded, do not apply
    ///
    /// Snapshot records recover a degraded feed when `seq >= missing_from`.
    ///
    /// # Errors
    ///
    /// Returns [`MdError::InvalidSequence`] if `seq == 0`.
    pub fn apply(&mut self, record: MdRecord) -> Result<ApplyOutcome, MdError> {
        record.validate()?;
        let instrument = record.instrument_id();
        let current = self.feed_status(instrument);
        let synced = self.next_seq.contains_key(&instrument);

        if !synced {
            return self.apply_unsynced(instrument, record, current);
        }

        let expected = self.expected_seq(instrument);

        if record.seq() < expected {
            self.duplicates += 1;
            return Ok(ApplyOutcome::Duplicate);
        }

        if let FeedStatus::Degraded { missing_from } = current {
            if record.kind() == MdKind::Snapshot {
                if record.seq() < missing_from {
                    return Err(MdError::InvalidSnapshot {
                        instrument_id: instrument,
                        expected: missing_from,
                        snapshot_seq: record.seq(),
                    });
                }
                self.mark_applied(instrument, record);
                self.status.insert(instrument, FeedStatus::Healthy);
                return Ok(ApplyOutcome::SnapshotRecovered);
            }
            if record.seq() > expected {
                self.gaps += 1;
                return Ok(ApplyOutcome::IgnoredDegraded);
            }
            return Ok(ApplyOutcome::IgnoredDegraded);
        }

        if record.seq() > expected {
            self.gaps += 1;
            self.status.insert(
                instrument,
                FeedStatus::Degraded {
                    missing_from: expected,
                },
            );
            return Ok(ApplyOutcome::GapDetected {
                expected,
                got: record.seq(),
            });
        }

        self.mark_applied(instrument, record);
        Ok(ApplyOutcome::Applied)
    }

    fn apply_unsynced(
        &mut self,
        instrument: InstrumentId,
        record: MdRecord,
        current: FeedStatus,
    ) -> Result<ApplyOutcome, MdError> {
        match current {
            FeedStatus::Degraded { missing_from } if record.kind() == MdKind::Snapshot => {
                if record.seq() < missing_from {
                    return Err(MdError::InvalidSnapshot {
                        instrument_id: instrument,
                        expected: missing_from,
                        snapshot_seq: record.seq(),
                    });
                }
                self.mark_applied(instrument, record);
                self.status.insert(instrument, FeedStatus::Healthy);
                Ok(ApplyOutcome::SnapshotRecovered)
            }
            FeedStatus::Degraded { .. } => Ok(ApplyOutcome::IgnoredDegraded),
            FeedStatus::Healthy => {
                self.mark_applied(instrument, record);
                Ok(ApplyOutcome::Applied)
            }
        }
    }

    /// Instrument ids with any tracked state, sorted for stable digests.
    #[must_use]
    pub fn tracked_instruments(&self) -> Vec<InstrumentId> {
        let mut ids: Vec<_> = self
            .next_seq
            .keys()
            .chain(self.last_price.keys())
            .chain(self.status.keys())
            .copied()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn mark_applied(&mut self, instrument: InstrumentId, record: MdRecord) {
        if matches!(
            record.kind(),
            MdKind::Trade | MdKind::Bbo | MdKind::Snapshot
        ) {
            self.last_price.insert(instrument, record.price());
        }
        self.next_seq
            .insert(instrument, record.seq().saturating_add(1));
        self.applied += 1;
        self.status.entry(instrument).or_insert(FeedStatus::Healthy);
    }
}

/// Result of applying one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Record updated consumer state.
    Applied,
    /// Duplicate sequence ignored.
    Duplicate,
    /// Gap detected; feed degraded.
    GapDetected {
        /// Expected sequence.
        expected: u64,
        /// Received sequence.
        got: u64,
    },
    /// Feed degraded; non-snapshot message skipped.
    IgnoredDegraded,
    /// Snapshot restored healthy feed.
    SnapshotRecovered,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::MdRecord;
    use shinrai_instruments::PriceTicks;

    fn trade(seq: u64) -> MdRecord {
        MdRecord::new(
            InstrumentId::from_u64(1),
            seq,
            0,
            MdKind::Trade,
            PriceTicks::from_scaled(100),
        )
    }

    #[test]
    fn unsynced_ticks_ignored_until_snapshot() {
        let inst = InstrumentId::from_u64(1);
        let mut s = MdConsumerState::new();
        s.mark_degraded(inst);
        let skipped = s.apply(trade(9_000)).expect("skip");
        assert_eq!(skipped, ApplyOutcome::IgnoredDegraded);
        let snap = MdRecord::new(
            inst,
            9_000,
            0,
            MdKind::Snapshot,
            PriceTicks::from_scaled(100),
        );
        assert_eq!(
            s.apply(snap).expect("snap"),
            ApplyOutcome::SnapshotRecovered
        );
        assert_eq!(s.feed_status(inst), FeedStatus::Healthy);
        assert_eq!(s.expected_seq(inst), 9_001);
    }
}
