//! Fault injection and fill policies.

/// How the simulator fills accepted orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillPolicy {
    /// Acknowledge then fill remaining quantity in one trade.
    Full,
    /// Acknowledge, fill `first_lots`, then the remainder on the next clock tick.
    Split {
        /// First fill size in lots.
        first_lots: i64,
    },
    /// Acknowledge only; no automatic fills.
    Rest,
}

impl Default for FillPolicy {
    fn default() -> Self {
        Self::Full
    }
}

/// Controllable simulator faults (deterministic; no RNG).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct FaultConfig {
    /// Reject every new order.
    pub reject_all: bool,
    /// After a trade, enqueue a second report with the same exec id.
    pub duplicate_exec: bool,
    /// Clock ticks to wait after ack before the first fill.
    pub delay_ticks: u64,
    /// Fill behaviour for accepted orders.
    pub fill_policy: FillPolicy,
    /// After a cancel is confirmed, still emit a trade (OMS must reject it).
    pub late_fill_after_cancel: bool,
    /// When emitting market data, skip a sequence number (gap).
    pub md_skip_seq: bool,
}

impl Default for FaultConfig {
    fn default() -> Self {
        Self {
            reject_all: false,
            duplicate_exec: false,
            delay_ticks: 0,
            fill_policy: FillPolicy::Full,
            late_fill_after_cancel: false,
            md_skip_seq: false,
        }
    }
}

impl FaultConfig {
    /// Happy-path defaults: immediate full fill, no faults.
    #[must_use]
    pub fn happy_path() -> Self {
        Self::default()
    }
}
