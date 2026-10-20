//! Pre-trade risk engine: fail-closed checks before the OMS.
//!
//! In-memory state is snapshotted to Postgres (`risk_state`) by the order
//! gateway and restored on hydrate.

#![forbid(unsafe_code)]

mod engine;
mod limits;
mod reason;

pub use engine::{RiskContext, RiskDecision, RiskEngine, RiskOrderIntent, RiskStateSnapshot};
pub use limits::RiskLimits;
pub use reason::RiskRejectReason;
