//! Replay driver.

use crate::checksum::state_digest;
use crate::consumer::{ApplyOutcome, MdConsumerState};
use crate::journal::MdJournal;

/// Summary of a full journal replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Final consumer projection.
    pub state: MdConsumerState,
    /// Deterministic digest of `state`.
    pub digest: u64,
    /// Per-record apply outcomes.
    pub outcomes: Vec<ApplyOutcome>,
}

/// Replays a journal from scratch into a fresh consumer.
#[must_use]
pub fn replay(journal: &MdJournal) -> ReplayReport {
    let mut state = MdConsumerState::new();
    let mut outcomes = Vec::with_capacity(journal.len());
    for record in journal.records() {
        // Validation errors should not occur in stored journals; panic in tests.
        let outcome = state.apply(*record).unwrap_or(ApplyOutcome::Duplicate);
        outcomes.push(outcome);
    }
    let digest = state_digest(&state);
    ReplayReport {
        state,
        digest,
        outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consumer::FeedStatus;
    use crate::journal::MdJournal;
    use crate::record::{MdKind, MdRecord};
    use shinrai_instruments::{InstrumentId, PriceTicks};

    fn trade(inst: InstrumentId, seq: u64, px: i64) -> MdRecord {
        MdRecord::new(inst, seq, seq, MdKind::Trade, PriceTicks::from_scaled(px))
    }

    #[test]
    fn replay_is_deterministic() {
        let inst = InstrumentId::from_u64(1);
        let journal = MdJournal::from_records([
            trade(inst, 1, 100),
            trade(inst, 2, 101),
            trade(inst, 3, 102),
        ])
        .expect("j");
        let a = replay(&journal);
        let b = replay(&journal);
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.state.last_price(inst).expect("p").scaled(), 102);
    }

    #[test]
    fn gap_marks_degraded_and_skips_apply() {
        let inst = InstrumentId::from_u64(1);
        let journal =
            MdJournal::from_records([trade(inst, 1, 100), trade(inst, 3, 102)]).expect("j");
        let report = replay(&journal);
        assert!(matches!(
            report.outcomes[1],
            ApplyOutcome::GapDetected {
                expected: 2,
                got: 3
            }
        ));
        assert!(matches!(
            report.state.feed_status(inst),
            FeedStatus::Degraded { missing_from: 2 }
        ));
        assert_eq!(report.state.last_price(inst).expect("p").scaled(), 100);
    }

    #[test]
    fn snapshot_recovers_after_gap() {
        let inst = InstrumentId::from_u64(1);
        let journal = MdJournal::from_records([
            trade(inst, 1, 100),
            trade(inst, 3, 102),
            MdRecord::new(inst, 3, 3, MdKind::Snapshot, PriceTicks::from_scaled(150)),
            trade(inst, 4, 151),
        ])
        .expect("j");
        let report = replay(&journal);
        assert!(matches!(
            report.outcomes[2],
            ApplyOutcome::SnapshotRecovered
        ));
        assert_eq!(report.state.feed_status(inst), FeedStatus::Healthy);
        assert_eq!(report.state.last_price(inst).expect("p").scaled(), 151);
    }

    #[test]
    fn duplicate_ignored() {
        let inst = InstrumentId::from_u64(1);
        let journal =
            MdJournal::from_records([trade(inst, 1, 100), trade(inst, 1, 999)]).expect("j");
        let report = replay(&journal);
        assert_eq!(report.outcomes[1], ApplyOutcome::Duplicate);
        assert_eq!(report.state.duplicate_count(), 1);
        assert_eq!(report.state.last_price(inst).expect("p").scaled(), 100);
    }
}
