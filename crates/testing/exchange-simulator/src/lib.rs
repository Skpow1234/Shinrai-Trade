//! Deterministic venue simulator that emits execution reports.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`] so the real adapter can
//! eventually speak the same types. Late fills after cancel are **emitted**
//! when configured, but the OMS rejects them (defined Phase 1 policy).

#![forbid(unsafe_code)]

mod clock;
mod error;
mod exchange;
mod faults;
mod md;
mod report;

pub use clock::VirtualClock;
pub use error::SimError;
pub use exchange::{NewSimOrder, SimExchange};
pub use faults::{FaultConfig, FillPolicy};
pub use md::MdTick;
pub use report::{stream_fingerprint, ExecType, ExecutionReport, SessionId};
