//! Alpaca Markets stock market-data adapter (trades + quotes + REST snapshot).
//!
//! Uses the authenticated IEX stream by default. Futures (e.g. ESZ5) are not on
//! this feed — pair with synth or a CME vendor for those products.

use serde_json::{json, Value};
use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks, QuantityLots};
use shinrai_market_data::{BookEvent, BookSnapshot, MdKind, MdRecord};

use crate::error::ProviderError;
use crate::vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};

/// Alpaca stock market-data vendor (keys required for live I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpacaMarkets {
    api_key: String,
    api_secret: String,
    /// `iex` (free) or `sip` (paid entitlement).
    feed: String,
}

impl AlpacaMarkets {
    /// IEX WebSocket base (path includes feed).
    pub const WS_ORIGIN: &'static str = "wss://stream.data.alpaca.markets/v2";
    /// REST data API origin.
    pub const REST_ORIGIN: &'static str = "https://data.alpaca.markets";

    /// Builds a vendor for the free IEX feed.
    #[must_use]
    pub fn iex(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            feed: "iex".into(),
        }
    }

    /// Builds a vendor for the SIP feed (requires Alpaca SIP entitlement).
    #[must_use]
    pub fn sip(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            feed: "sip".into(),
        }
    }

    /// Auth JSON for the stream (send before subscribe).
    #[must_use]
    pub fn auth_message(&self) -> Vec<u8> {
        json!({
            "action": "auth",
            "key": self.api_key,
            "secret": self.api_secret,
        })
        .to_string()
        .into_bytes()
    }

    /// Feed path segment (`iex` / `sip`).
    #[must_use]
    pub fn feed(&self) -> &str {
        &self.feed
    }

    /// Full WebSocket URL for this feed.
    #[must_use]
    pub fn stream_url(&self) -> String {
        format!("{}/{}", Self::WS_ORIGIN, self.feed)
    }

    fn resolve_product(
        master: &InstrumentMaster,
        product_id: &str,
    ) -> Result<InstrumentId, ProviderError> {
        let alias = ExternalId::ticker(product_id)?;
        master
            .resolve_alias(&alias)
            .map_err(|_| ProviderError::UnknownInstrument {
                product_id: product_id.to_owned(),
            })
    }

    fn price_ticks(
        master: &InstrumentMaster,
        instrument_id: InstrumentId,
        price: f64,
    ) -> Result<PriceTicks, ProviderError> {
        let instrument = master.get(instrument_id)?;
        // Format to the instrument price scale (not a fixed 6dp — equities are scale 2).
        let scale = usize::from(instrument.tick_table().price_scale());
        let decimal = format!("{price:.scale$}");
        Ok(instrument.price_to_ticks(&decimal)?)
    }
}

impl MarketDataVendor for AlpacaMarkets {
    fn id(&self) -> VendorId {
        VendorId::ALPACA_MARKETS
    }

    fn websocket_url(&self) -> &'static str {
        // Trait requires &'static str; I/O layer should prefer [`Self::stream_url`].
        "wss://stream.data.alpaca.markets/v2/iex"
    }

    fn subscribe_message(&self, product_ids: &[String]) -> Vec<u8> {
        json!({
            "action": "subscribe",
            "trades": product_ids,
            "quotes": product_ids,
        })
        .to_string()
        .into_bytes()
    }

    fn snapshot_spec(&self, product_id: &str) -> SnapshotSpec {
        SnapshotSpec::get_with_headers(
            format!("{}/v2/stocks/{product_id}/snapshot", Self::REST_ORIGIN),
            vec![
                ("APCA-API-KEY-ID".into(), self.api_key.clone()),
                ("APCA-API-SECRET-KEY".into(), self.api_secret.clone()),
            ],
        )
    }

    fn decode_stream(
        &self,
        raw: &[u8],
        ts_logical: u64,
        master: &InstrumentMaster,
    ) -> Result<DecodedFrame, ProviderError> {
        let value: Value = serde_json::from_slice(raw).map_err(|_| ProviderError::InvalidJson)?;
        // Gateway splits arrays; accept single objects here.
        let msg_type = value
            .get("T")
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField("T"))?;
        match msg_type {
            "t" => {
                let symbol = json_str(&value, "S")?;
                let instrument_id = Self::resolve_product(master, symbol)?;
                let seq = value
                    .get("i")
                    .and_then(json_u64_opt)
                    .filter(|n| *n > 0)
                    .unwrap_or(ts_logical.max(1));
                let price = json_f64(&value, "p")?;
                let ticks = Self::price_ticks(master, instrument_id, price)?;
                let qty = value
                    .get("s")
                    .and_then(json_u64_opt)
                    .map_or(QuantityLots::from_lots(0), |n| {
                        QuantityLots::from_lots(i64::try_from(n).unwrap_or(i64::MAX))
                    });
                Ok(DecodedFrame::Record(
                    MdRecord::new(instrument_id, seq, ts_logical, MdKind::Trade, ticks)
                        .with_qty(qty),
                ))
            }
            "q" => {
                let symbol = json_str(&value, "S")?;
                let instrument_id = Self::resolve_product(master, symbol)?;
                let bid = value.get("bp").and_then(Value::as_f64).unwrap_or(0.0);
                let ask = value.get("ap").and_then(Value::as_f64).unwrap_or(0.0);
                let mid = if bid > 0.0 && ask > 0.0 {
                    f64::midpoint(bid, ask)
                } else if ask > 0.0 {
                    ask
                } else {
                    bid
                };
                if mid <= 0.0 {
                    return Ok(DecodedFrame::Control);
                }
                let ticks = Self::price_ticks(master, instrument_id, mid)?;
                let seq = ts_logical.max(1);
                Ok(DecodedFrame::Record(MdRecord::new(
                    instrument_id,
                    seq,
                    ts_logical,
                    MdKind::Bbo,
                    ticks,
                )))
            }
            _ => Ok(DecodedFrame::Control),
        }
    }

    fn decode_snapshot(
        &self,
        raw: &[u8],
        ts_logical: u64,
        product_id: &str,
        master: &InstrumentMaster,
    ) -> Result<MdRecord, ProviderError> {
        let value: Value = serde_json::from_slice(raw).map_err(|_| ProviderError::InvalidJson)?;
        let instrument_id = Self::resolve_product(master, product_id)?;
        let (price, seq) = snapshot_mark(&value)?;
        let ticks = Self::price_ticks(master, instrument_id, price)?;
        Ok(MdRecord::new(
            instrument_id,
            seq.max(1),
            ts_logical,
            MdKind::Snapshot,
            ticks,
        ))
    }

    fn decode_book_snapshot(
        &self,
        raw: &[u8],
        ts_logical: u64,
        product_id: &str,
        master: &InstrumentMaster,
    ) -> Result<BookEvent, ProviderError> {
        // Stocks path: mark recovery uses decode_snapshot; L2 not required.
        let instrument_id = Self::resolve_product(master, product_id)?;
        let value: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
        let seq = value
            .get("latestTrade")
            .and_then(|t| t.get("i"))
            .and_then(json_u64_opt)
            .unwrap_or(1)
            .max(1);
        Ok(BookEvent::Snapshot(BookSnapshot::new(
            instrument_id,
            seq,
            ts_logical,
            Vec::new(),
            Vec::new(),
        )))
    }
}

fn snapshot_mark(value: &Value) -> Result<(f64, u64), ProviderError> {
    if let Some(trade) = value.get("latestTrade") {
        if let Some(p) = trade.get("p").and_then(Value::as_f64) {
            if p > 0.0 {
                let seq = trade.get("i").and_then(json_u64_opt).unwrap_or(1);
                return Ok((p, seq));
            }
        }
    }
    if let Some(quote) = value.get("latestQuote") {
        let bid = quote.get("bp").and_then(Value::as_f64).unwrap_or(0.0);
        let ask = quote.get("ap").and_then(Value::as_f64).unwrap_or(0.0);
        let mid = if bid > 0.0 && ask > 0.0 {
            f64::midpoint(bid, ask)
        } else if ask > 0.0 {
            ask
        } else {
            bid
        };
        if mid > 0.0 {
            return Ok((mid, 1));
        }
    }
    Err(ProviderError::MissingField("latestTrade|latestQuote"))
}

fn json_str<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, ProviderError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(ProviderError::MissingField(field))
}

fn json_f64(value: &Value, field: &'static str) -> Result<f64, ProviderError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or(ProviderError::MissingField(field))
}

fn json_u64_opt(cell: &Value) -> Option<u64> {
    match cell {
        Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(i64::unsigned_abs)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::phase1_master;

    #[test]
    fn decode_trade_and_quote() {
        let vendor = AlpacaMarkets::iex("k", "s");
        let master = phase1_master();
        let trade = br#"{"T":"t","S":"AAPL","i":99,"p":190.25,"s":10}"#;
        let frame = vendor.decode_stream(trade, 5, &master).expect("trade");
        match frame {
            DecodedFrame::Record(r) => {
                assert_eq!(r.kind(), MdKind::Trade);
                assert_eq!(r.seq(), 99);
                assert_eq!(r.price().scaled(), 19_025);
            }
            other => panic!("expected record, got {other:?}"),
        }
        let quote = br#"{"T":"q","S":"AAPL","bp":190.0,"ap":190.5}"#;
        let frame = vendor.decode_stream(quote, 6, &master).expect("quote");
        assert!(matches!(frame, DecodedFrame::Record(r) if r.kind() == MdKind::Bbo));
    }

    #[test]
    fn decode_snapshot_latest_trade() {
        let vendor = AlpacaMarkets::iex("k", "s");
        let master = phase1_master();
        let body = br#"{"latestTrade":{"p":191.0,"i":7},"latestQuote":{"bp":190.9,"ap":191.1}}"#;
        let rec = vendor
            .decode_snapshot(body, 1, "AAPL", &master)
            .expect("snap");
        assert_eq!(rec.kind(), MdKind::Snapshot);
        assert_eq!(rec.price().scaled(), 19_100);
        assert_eq!(rec.seq(), 7);
    }

    #[test]
    fn auth_and_subscribe_payloads() {
        let vendor = AlpacaMarkets::iex("KEY", "SECRET");
        let auth = String::from_utf8(vendor.auth_message()).expect("utf8");
        assert!(auth.contains("\"action\":\"auth\""));
        assert!(auth.contains("KEY"));
        let sub = String::from_utf8(vendor.subscribe_message(&["AAPL".into()])).expect("utf8");
        assert!(sub.contains("subscribe"));
        assert!(sub.contains("AAPL"));
        let spec = vendor.snapshot_spec("AAPL");
        assert!(spec.url().contains("/v2/stocks/AAPL/snapshot"));
        assert_eq!(spec.headers().len(), 2);
    }
}
