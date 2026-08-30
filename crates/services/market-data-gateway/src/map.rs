//! Map supervisor events onto client fanout events.

use shinrai_md_fanout::MarketEvent;
use shinrai_md_protocol::SupervisorEvent;

/// Converts a feed event into something clients should see.
///
/// Deltas, duplicates, and vendor heartbeats are not fanned out (too chatty).
/// Sequence gaps and stale clocks mark the instrument degraded.
#[must_use]
pub fn to_market_event(event: &SupervisorEvent) -> Option<MarketEvent> {
    match event {
        SupervisorEvent::Applied(record) => Some(MarketEvent::Tick(*record)),
        SupervisorEvent::Gap { instrument_id, .. }
        | SupervisorEvent::Stale { instrument_id, .. } => Some(MarketEvent::Degraded {
            instrument_id: *instrument_id,
        }),
        SupervisorEvent::BookRebuilt {
            instrument_id,
            checksum,
        } => Some(MarketEvent::BookReady {
            instrument_id: *instrument_id,
            checksum: *checksum,
        }),
        SupervisorEvent::Duplicate { .. }
        | SupervisorEvent::SnapshotRecovered { .. }
        | SupervisorEvent::IgnoredDegraded { .. }
        | SupervisorEvent::Heartbeat { .. }
        | SupervisorEvent::BookDeltaApplied { .. }
        | SupervisorEvent::Skipped => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::{btc_usd, PriceTicks};
    use shinrai_market_data::{MdKind, MdRecord};

    #[test]
    fn applied_becomes_tick() {
        let record = MdRecord::new(
            btc_usd().id(),
            1,
            1,
            MdKind::Trade,
            PriceTicks::from_scaled(1),
        );
        assert!(matches!(
            to_market_event(&SupervisorEvent::Applied(record)),
            Some(MarketEvent::Tick(_))
        ));
    }

    #[test]
    fn gap_and_stale_degrade() {
        let id = btc_usd().id();
        assert!(matches!(
            to_market_event(&SupervisorEvent::Gap {
                instrument_id: id,
                expected: 2,
                got: 9,
            }),
            Some(MarketEvent::Degraded { instrument_id }) if instrument_id == id
        ));
        assert!(matches!(
            to_market_event(&SupervisorEvent::Stale {
                instrument_id: id,
                silent_for: 31,
            }),
            Some(MarketEvent::Degraded { .. })
        ));
    }

    #[test]
    fn skipped_is_silent() {
        assert!(to_market_event(&SupervisorEvent::Skipped).is_none());
    }
}
