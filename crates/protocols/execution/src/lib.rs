//! Execution venue contract shared by the paper simulator and broker adapters.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`]. Phase 4 live path is Alpaca
//! paper REST (`AlpacaPaperVenue`); FIX remains optional for a later track.

#![forbid(unsafe_code)]

mod alpaca;
mod error;
mod licensed;
mod report;
mod rest;
mod sandbox;
mod session;
mod venue;

pub use alpaca::{
    AlpacaAccountSnap, AlpacaBrokerStatement, AlpacaConfig, AlpacaFillSnap, AlpacaPaperVenue,
    AlpacaPositionSnap, LocalAlpacaHttp, RemoteAlpacaHttp, ALPACA_PAPER_BASE_URL,
};
pub use error::ExecutionError;
pub use licensed::{LicensedSandboxConfig, LicensedSandboxVenue, OutboundMsg, SessionPhase};
pub use report::{stream_fingerprint, ExecType, ExecutionReport, SessionId};
pub use rest::{
    HttpMethod, HttpRequest, HttpResponse, HttpTransport, LocalPaperHttp, RemotePaperHttp,
    RestPaperVenue,
};
pub use sandbox::{SandboxBroker, SandboxConfig};
pub use session::VenueSessionState;
pub use venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};
