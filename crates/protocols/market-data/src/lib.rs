//! Vendor market-data protocol: decode, raw recording, connection supervision.
//!
//! Phase 2.1 sandbox vendor is **Coinbase Exchange** public WebSocket/REST
//! (`wss://ws-feed.exchange.coinbase.com`). Equities/futures remain on the
//! instrument master but are not streamed until a later licensed feed.
//!
//! Domain types stay vendor-agnostic. This crate maps vendor JSON onto
//! [`shinrai_market_data::MdRecord`], keeps the raw frame for replay, and
//! never applies ticks across a sequence gap without a snapshot.

#![forbid(unsafe_code)]

mod coinbase;
mod error;
mod journal;
mod supervisor;
mod vendor;

pub use coinbase::CoinbaseExchange;
pub use error::ProviderError;
pub use journal::{RawFrame, RecordingJournal};
pub use supervisor::{FeedCommand, FeedSupervisor, IngestOutcome, SupervisorEvent};
pub use vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};
