//! Venue session cursor (paper / adapter session durability).

use crate::report::SessionId;

/// Snapshot of an execution venue's session (heartbeats / reconnect / gap-fill).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueSessionState {
    /// Current session id (bumps on reconnect).
    pub session: SessionId,
    /// Next sequence number the venue will assign.
    pub next_seq: u64,
    /// Whether the venue accepts new commands.
    pub connected: bool,
}

impl VenueSessionState {
    /// Connected session starting at `session` with `next_seq`.
    #[must_use]
    pub const fn connected(session: SessionId, next_seq: u64) -> Self {
        Self {
            session,
            next_seq,
            connected: true,
        }
    }

    /// Disconnected view of the same cursor.
    #[must_use]
    pub const fn disconnected(session: SessionId, next_seq: u64) -> Self {
        Self {
            session,
            next_seq,
            connected: false,
        }
    }
}
