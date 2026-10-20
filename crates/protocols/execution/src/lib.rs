//! Execution venue contract shared by the paper simulator and broker adapters.
//!
//! Reports map onto [`shinrai_orders::OrderEvent`]. Phase 4 live path is Alpaca
//! paper REST (`AlpacaPaperVenue`); FIX 4.2 subset is [`FixPaperVenue`]
//! (in-process or TCP initiator / loopback acceptor).

#![forbid(unsafe_code)]

mod alpaca;
mod error;
mod fix;
mod fix_tcp;
mod licensed;
mod report;
mod rest;
mod runtime;
mod sandbox;
mod session;
mod venue;

pub use alpaca::{
    AlpacaAccountSnap, AlpacaBrokerStatement, AlpacaConfig, AlpacaFillSnap, AlpacaPaperVenue,
    AlpacaPositionSnap, LocalAlpacaHttp, RemoteAlpacaHttp, ALPACA_PAPER_BASE_URL,
};
pub use error::ExecutionError;
pub use fix::{decode_fix, encode_fix, FixConfig, FixPaperVenue, FixSessionKind};
pub use fix_tcp::{
    connect_fix, read_fix_message, spawn_paper_acceptor, write_fix_message, FixTcpAcceptorGuard,
};
pub use licensed::{LicensedSandboxConfig, LicensedSandboxVenue, OutboundMsg, SessionPhase};
pub use report::{stream_fingerprint, ExecType, ExecutionReport, SessionId};
pub use rest::{
    HttpMethod, HttpRequest, HttpResponse, HttpTransport, LocalPaperHttp, RemotePaperHttp,
    RestPaperVenue,
};
pub use sandbox::{SandboxBroker, SandboxConfig};
pub use session::VenueSessionState;
pub use venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};
