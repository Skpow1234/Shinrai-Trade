//! Vendor-agnostic decode and snapshot contracts.

use shinrai_instruments::{InstrumentId, InstrumentMaster};
use shinrai_market_data::{BookEvent, MdRecord};

use crate::error::ProviderError;

/// Stable vendor identifier (not a display name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VendorId(&'static str);

impl VendorId {
    /// Coinbase Exchange public market data.
    pub const COINBASE_EXCHANGE: Self = Self("coinbase-exchange");

    /// Raw id string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl core::fmt::Display for VendorId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// How to fetch a recovery snapshot (HTTP GET in Phase 2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSpec {
    url: String,
}

impl SnapshotSpec {
    /// Builds a GET snapshot spec.
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    /// Snapshot URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Result of decoding one vendor stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedFrame {
    /// Liveness only; does not advance the sequenced consumer.
    Heartbeat {
        /// Instrument if the heartbeat is product-scoped.
        instrument_id: InstrumentId,
    },
    /// Sequenced normalized record.
    Record(MdRecord),
    /// L2 snapshot or delta.
    Book(BookEvent),
    /// Subscribe ack, vendor error, or other non-market payload.
    Control,
}

/// One market-data venue.
pub trait MarketDataVendor {
    /// Vendor identity.
    fn id(&self) -> VendorId;

    /// Public WebSocket endpoint.
    fn websocket_url(&self) -> &'static str;

    /// Subscribe payload for the given vendor product ids.
    fn subscribe_message(&self, product_ids: &[String]) -> Vec<u8>;

    /// Snapshot fetch description for a product.
    fn snapshot_spec(&self, product_id: &str) -> SnapshotSpec;

    /// Decodes a stream frame (WebSocket text).
    ///
    /// # Errors
    ///
    /// Returns decode / mapping errors. Unknown products are errors so the
    /// supervisor can skip without applying.
    fn decode_stream(
        &self,
        raw: &[u8],
        ts_logical: u64,
        master: &InstrumentMaster,
    ) -> Result<DecodedFrame, ProviderError>;

    /// Decodes a snapshot HTTP body for a known product.
    ///
    /// # Errors
    ///
    /// Returns decode / mapping errors.
    fn decode_snapshot(
        &self,
        raw: &[u8],
        ts_logical: u64,
        product_id: &str,
        master: &InstrumentMaster,
    ) -> Result<MdRecord, ProviderError>;

    /// Decodes a REST/WS L2 snapshot body.
    ///
    /// # Errors
    ///
    /// Returns decode / mapping errors.
    fn decode_book_snapshot(
        &self,
        raw: &[u8],
        ts_logical: u64,
        product_id: &str,
        master: &InstrumentMaster,
    ) -> Result<BookEvent, ProviderError>;
}
