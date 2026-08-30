//! Session identity and client-facing messages.

use shinrai_instruments::InstrumentId;
use shinrai_market_data::MdRecord;

/// Connected client session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    /// Creates a session id.
    #[must_use]
    pub const fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Normalized event to fan out to subscribers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketEvent {
    /// Applied trade / BBO / snapshot tick.
    Tick(MdRecord),
    /// Feed or book is degraded; UI should hide stale book.
    Degraded {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// L2 book rebuilt; client may trust the book again.
    BookReady {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Book checksum.
        checksum: u64,
    },
}

impl MarketEvent {
    /// Instrument this event belongs to.
    #[must_use]
    pub const fn instrument_id(self) -> InstrumentId {
        match self {
            Self::Tick(record) => record.instrument_id(),
            Self::Degraded { instrument_id } | Self::BookReady { instrument_id, .. } => {
                instrument_id
            }
        }
    }
}

/// Outbound client frame (after JSON encode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMessage {
    /// Market-data payload.
    Market(MarketEvent),
    /// Session heartbeat (includes drop count).
    Heartbeat {
        /// Logical time.
        ts_logical: u64,
        /// Cumulative dropped outbound frames (queue overflow).
        dropped: u64,
    },
    /// Subscribe accepted.
    Subscribed {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// Error without secrets.
    Error {
        /// Stable code.
        code: &'static str,
    },
}
