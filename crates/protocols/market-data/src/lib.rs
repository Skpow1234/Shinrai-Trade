//! Vendor market-data protocol: decode, raw recording, connection supervision.
//!
//! Sandbox vendors:
//! - **Coinbase Exchange** public WS/REST — crypto (`BTC-USD`)
//! - **Alpaca Markets** authenticated stock stream — equities (`AAPL`, …)
//! - **Databento GLBX** — CME Globex futures (`ESZ5` / `ES.FUT`); trades + mbp-10 L2; paper or licensed live
//!
//! Domain types stay vendor-agnostic. This crate maps vendor JSON onto
//! [`shinrai_market_data::MdRecord`], keeps the raw frame for replay, and
//! never applies ticks across a sequence gap without a snapshot.

#![forbid(unsafe_code)]

mod alpaca;
mod coinbase;
mod databento;
mod error;
mod journal;
mod supervisor;
mod vendor;

pub use alpaca::AlpacaMarkets;
pub use coinbase::CoinbaseExchange;
pub use databento::DatabentoGlbx;
pub use error::ProviderError;
pub use journal::{RawFrame, RecordingJournal};
pub use supervisor::{FeedCommand, FeedSupervisor, IngestOutcome, SupervisorEvent};
pub use vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};
