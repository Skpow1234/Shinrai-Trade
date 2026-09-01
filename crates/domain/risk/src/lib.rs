//! Pre-trade risk engine: fail-closed checks before the OMS.
//!
//! State is in-memory and intended to be rebuildable from durable events
//! in later phases.

#![forbid(unsafe_code)]

mod engine;
mod limits;
mod reason;

pub use engine::{RiskContext, RiskDecision, RiskEngine, RiskOrderIntent};
pub use limits::RiskLimits;
pub use reason::RiskRejectReason;
