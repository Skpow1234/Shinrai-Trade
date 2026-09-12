//! Deterministic venue simulator that emits execution reports.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`] so the real adapter can
//! eventually speak the same types. Late fills after cancel are **emitted**
//! when configured, but the OMS rejects them.

#![forbid(unsafe_code)]

mod clock;
mod error;
mod exchange;
mod faults;
mod md;

pub use clock::VirtualClock;
pub use error::SimError;
pub use exchange::SimExchange;
pub use faults::{FaultConfig, FillPolicy};
pub use md::MdTick;
pub use shinrai_execution::{
    stream_fingerprint, ExecType, ExecutionError, ExecutionReport, ExecutionVenue, NewVenueOrder,
    SessionId, VenueOrderSnapshot,
};

/// Historical name for [`NewVenueOrder`].
pub type NewSimOrder = NewVenueOrder;
