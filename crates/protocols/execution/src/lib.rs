//! Execution venue contract shared by the paper simulator and broker adapters.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`]. A real FIX/REST adapter
//! should emit the same shapes so the OMS path stays unchanged.

#![forbid(unsafe_code)]

mod error;
mod licensed;
mod report;
mod rest;
mod sandbox;
mod session;
mod venue;

pub use error::ExecutionError;
pub use licensed::{
    LicensedSandboxConfig, LicensedSandboxVenue, OutboundMsg, SessionPhase,
};
pub use report::{stream_fingerprint, ExecType, ExecutionReport, SessionId};
pub use rest::{
    HttpMethod, HttpRequest, HttpResponse, HttpTransport, LocalPaperHttp, RemotePaperHttp,
    RestPaperVenue,
};
pub use sandbox::{SandboxBroker, SandboxConfig};
pub use session::VenueSessionState;
pub use venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};
