//! Execution venue contract shared by the paper simulator and broker adapters.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`]. A real FIX/REST adapter
//! should emit the same shapes so the OMS path stays unchanged.

#![forbid(unsafe_code)]

mod error;
mod report;
mod sandbox;
mod venue;

pub use error::ExecutionError;
pub use report::{stream_fingerprint, ExecType, ExecutionReport, SessionId};
pub use sandbox::{SandboxBroker, SandboxConfig};
pub use venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot};
