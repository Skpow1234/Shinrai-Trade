//! Market-data errors.

use core::fmt;

use shinrai_instruments::InstrumentId;

/// Errors from journal or replay operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdError {
    /// Sequence number was zero.
    InvalidSequence,
    /// Instrument id was invalid for this operation.
    InvalidInstrument,
    /// Snapshot sequence is not ahead of the degraded gap.
    InvalidSnapshot {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Expected next sequence after gap.
        expected: u64,
        /// Snapshot sequence offered.
        snapshot_seq: u64,
    },
}

impl fmt::Display for MdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSequence => f.write_str("sequence must be positive"),
            Self::InvalidInstrument => f.write_str("invalid instrument"),
            Self::InvalidSnapshot {
                instrument_id,
                expected,
                snapshot_seq,
            } => write!(
                f,
                "snapshot seq {snapshot_seq} invalid for {instrument_id} (expected >= {expected})"
            ),
        }
    }
}

impl std::error::Error for MdError {}
