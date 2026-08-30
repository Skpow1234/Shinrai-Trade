//! Logical clock for delayed reports.

/// Monotonic simulator clock (integer ticks, not wall time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VirtualClock {
    now: u64,
}

impl VirtualClock {
    /// Clock at time zero.
    #[must_use]
    pub const fn new() -> Self {
        Self { now: 0 }
    }

    /// Current tick.
    #[must_use]
    pub const fn now(self) -> u64 {
        self.now
    }

    /// Advances the clock by `ticks` (saturating).
    pub fn advance(&mut self, ticks: u64) {
        self.now = self.now.saturating_add(ticks);
    }
}
