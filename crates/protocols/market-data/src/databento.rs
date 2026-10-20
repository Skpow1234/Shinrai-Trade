//! Databento-shaped CME Globex (`GLBX.MDP3`) futures market-data adapter.
//!
//! Phase A uses a JSON wire (trades / mbp-1 / snapshot) that matches the
//! gateway's Alpaca-style supervisor path. Live I/O talks to a licensed
//! Databento-compatible endpoint when keys/URLs are set; paper mode emits the
//! same JSON through [`FeedSupervisor`] so `ESZ5` is no longer raw synth-only.

use serde_json::{json, Value};
use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks, QuantityLots};
use shinrai_market_data::{BookEvent, BookSnapshot, MdKind, MdRecord};

use crate::error::ProviderError;
use crate::vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};

/// CME Globex futures via Databento-shaped JSON (dataset default `GLBX.MDP3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabentoGlbx {
    api_key: String,
    dataset: String,
    ws_url: String,
    rest_url: String,
    paper: bool,
}

impl DatabentoGlbx {
    /// Default live WebSocket origin (override with `SHINRAI_MD_FUTURES_WS`).
    pub const WS_DEFAULT: &'static str = "wss://live.databento.com";
    /// Default REST origin for snapshot recovery.
    pub const REST_DEFAULT: &'static str = "https://hist.databento.com";
    /// CME Globex MDP3 dataset id.
    pub const DATASET_GLBX: &'static str = "GLBX.MDP3";

    /// Licensed live vendor (API key required for remote I/O).
    #[must_use]
    pub fn live(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            dataset: Self::DATASET_GLBX.into(),
            ws_url: Self::WS_DEFAULT.into(),
            rest_url: Self::REST_DEFAULT.into(),
            paper: false,
        }
    }

    /// In-process paper vendor (same decode path; no remote keys).
    #[must_use]
    pub fn paper() -> Self {
        Self {
            api_key: String::new(),
            dataset: Self::DATASET_GLBX.into(),
            ws_url: "paper://futures".into(),
            rest_url: "paper://futures".into(),
            paper: true,
        }
    }

    /// Overrides WebSocket / REST origins (tests or private gateways).
    #[must_use]
    pub fn with_endpoints(
        mut self,
        ws_url: impl Into<String>,
        rest_url: impl Into<String>,
    ) -> Self {
        self.ws_url = ws_url.into();
        self.rest_url = rest_url.into();
        self
    }

    /// Overrides dataset id (default [`Self::DATASET_GLBX`]).
    #[must_use]
    pub fn with_dataset(mut self, dataset: impl Into<String>) -> Self {
        self.dataset = dataset.into();
        self
    }

    /// Whether this instance is paper (no remote sockets).
    #[must_use]
    pub const fn is_paper(&self) -> bool {
        self.paper
    }

    /// Dataset id.
    #[must_use]
    pub fn dataset(&self) -> &str {
        &self.dataset
    }

    /// Full WebSocket URL.
    #[must_use]
    pub fn stream_url(&self) -> &str {
        &self.ws_url
    }

    /// Auth JSON for a live stream (send before subscribe).
    #[must_use]
    pub fn auth_message(&self) -> Vec<u8> {
        json!({
            "action": "auth",
            "key": self.api_key,
            "dataset": self.dataset,
        })
        .to_string()
        .into_bytes()
    }

    /// Paper snapshot body for supervisor recovery (no HTTP).
    #[must_use]
    pub fn paper_snapshot_body(product_id: &str, sequence: u64, price_decimal: &str) -> Vec<u8> {
        json!({
            "schema": "snapshot",
            "symbol": product_id,
            "sequence": sequence,
            "price": price_decimal,
        })
        .to_string()
        .into_bytes()
    }

    /// Paper trade frame for the supervised paper publisher.
    #[must_use]
    pub fn paper_trade_body(
        product_id: &str,
        sequence: u64,
        price_decimal: &str,
        size: i64,
    ) -> Vec<u8> {
        json!({
            "schema": "trades",
            "symbol": product_id,
            "sequence": sequence,
            "price": price_decimal,
            "size": size,
        })
        .to_string()
        .into_bytes()
    }

    fn resolve_product(
        master: &InstrumentMaster,
        product_id: &str,
    ) -> Result<InstrumentId, ProviderError> {
        // Continuous root used by some licensed feeds → front-month fixture.
        let ticker =
            if product_id.eq_ignore_ascii_case("ES.FUT") || product_id.eq_ignore_ascii_case("ES") {
                "ESZ5"
            } else {
                product_id
            };
        let alias = ExternalId::ticker(ticker)?;
        master
            .resolve_alias(&alias)
            .or_else(|_| {
                let at = ExternalId::ticker_at(ticker, "XCME")?;
                master.resolve_alias(&at)
            })
            .map_err(|_| ProviderError::UnknownInstrument {
                product_id: product_id.to_owned(),
            })
    }

    fn price_ticks(
        master: &InstrumentMaster,
        instrument_id: InstrumentId,
        price: &str,
    ) -> Result<PriceTicks, ProviderError> {
        let instrument = master.get(instrument_id)?;
        Ok(instrument.price_to_ticks(price.trim())?)
    }

    fn price_from_value(value: &Value) -> Result<String, ProviderError> {
        if let Some(s) = value.get("price").and_then(Value::as_str) {
            if !s.is_empty() {
                return Ok(s.to_owned());
            }
        }
        if let Some(s) = value.get("close").and_then(Value::as_str) {
            if !s.is_empty() {
                return Ok(s.to_owned());
            }
        }
        // Databento fixed-point integer (1e-9) → decimal string for ES-scale prices.
        if let Some(n) = value.get("price").and_then(Value::as_i64) {
            if n > 0 {
                let whole = n / 1_000_000_000;
                let frac = (n % 1_000_000_000).unsigned_abs();
                return Ok(format!("{whole}.{frac:09}"));
            }
        }
        Err(ProviderError::MissingField("price"))
    }
}

impl MarketDataVendor for DatabentoGlbx {
    fn id(&self) -> VendorId {
        VendorId::DATABENTO_GLBX
    }

    fn websocket_url(&self) -> &'static str {
        Self::WS_DEFAULT
    }

    fn subscribe_message(&self, product_ids: &[String]) -> Vec<u8> {
        json!({
            "action": "subscribe",
            "dataset": self.dataset,
            "schema": "trades",
            "symbols": product_ids,
        })
        .to_string()
        .into_bytes()
    }

    fn snapshot_spec(&self, product_id: &str) -> SnapshotSpec {
        if self.paper {
            return SnapshotSpec::get(format!("paper://futures/{product_id}"));
        }
        let mut headers = Vec::new();
        if !self.api_key.is_empty() {
            headers.push(("Authorization".into(), format!("Bearer {}", self.api_key)));
        }
        SnapshotSpec::get_with_headers(
            format!(
                "{}/v0/timeseries.get_range?dataset={}&symbols={}&schema=trades&limit=1",
                self.rest_url, self.dataset, product_id
            ),
            headers,
        )
    }

    fn decode_stream(
        &self,
        raw: &[u8],
        ts_logical: u64,
        master: &InstrumentMaster,
    ) -> Result<DecodedFrame, ProviderError> {
        let value: Value = serde_json::from_slice(raw).map_err(|_| ProviderError::InvalidJson)?;
        let schema = value
            .get("schema")
            .and_then(Value::as_str)
            .or_else(|| value.get("type").and_then(Value::as_str))
            .unwrap_or("");
        match schema {
            "heartbeat" => {
                let symbol = value
                    .get("symbol")
                    .and_then(Value::as_str)
                    .unwrap_or("ESZ5");
                let instrument_id = Self::resolve_product(master, symbol)?;
                Ok(DecodedFrame::Heartbeat { instrument_id })
            }
            "trades" | "trade" => {
                let symbol = json_str(&value, "symbol")?;
                let instrument_id = Self::resolve_product(master, symbol)?;
                let seq = value
                    .get("sequence")
                    .and_then(json_u64_opt)
                    .filter(|n| *n > 0)
                    .unwrap_or(ts_logical.max(1));
                let price = Self::price_from_value(&value)?;
                let ticks = Self::price_ticks(master, instrument_id, &price)?;
                let qty = value
                    .get("size")
                    .and_then(json_i64_opt)
                    .map_or(QuantityLots::from_lots(1), QuantityLots::from_lots);
                Ok(DecodedFrame::Record(
                    MdRecord::new(instrument_id, seq, ts_logical, MdKind::Trade, ticks)
                        .with_qty(qty),
                ))
            }
            "mbp-1" | "bbo" | "quote" => {
                let symbol = json_str(&value, "symbol")?;
                let instrument_id = Self::resolve_product(master, symbol)?;
                let bid = value
                    .get("bid_px")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let ask = value
                    .get("ask_px")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mid =
                    mid_decimal(bid, ask).ok_or(ProviderError::MissingField("bid_px|ask_px"))?;
                let ticks = Self::price_ticks(master, instrument_id, &mid)?;
                let seq = value
                    .get("sequence")
                    .and_then(json_u64_opt)
                    .unwrap_or(ts_logical.max(1));
                Ok(DecodedFrame::Record(MdRecord::new(
                    instrument_id,
                    seq,
                    ts_logical,
                    MdKind::Bbo,
                    ticks,
                )))
            }
            "snapshot" => {
                // Streamed snapshot treated as control; REST path uses decode_snapshot.
                Ok(DecodedFrame::Control)
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
        let price = Self::price_from_value(&value)?;
        let ticks = Self::price_ticks(master, instrument_id, &price)?;
        let seq = value
            .get("sequence")
            .and_then(json_u64_opt)
            .unwrap_or(1)
            .max(1);
        Ok(MdRecord::new(
            instrument_id,
            seq,
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
        let instrument_id = Self::resolve_product(master, product_id)?;
        let value: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
        let seq = value
            .get("sequence")
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

fn mid_decimal(bid: &str, ask: &str) -> Option<String> {
    let bid_f: f64 = bid.parse().ok().filter(|n| *n > 0.0)?;
    let ask_f: f64 = ask.parse().ok().filter(|n| *n > 0.0)?;
    let mid = f64::midpoint(bid_f, ask_f);
    Some(format!("{mid:.2}"))
}

fn json_str<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, ProviderError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(ProviderError::MissingField(field))
}

fn json_u64_opt(cell: &Value) -> Option<u64> {
    match cell {
        Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(i64::unsigned_abs)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn json_i64_opt(cell: &Value) -> Option<i64> {
    match cell {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok())),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::phase1_master;

    #[test]
    fn decode_trade_and_bbo() {
        let vendor = DatabentoGlbx::paper();
        let master = phase1_master();
        let trade =
            br#"{"schema":"trades","symbol":"ESZ5","sequence":99,"price":"5200.00","size":2}"#;
        let frame = vendor.decode_stream(trade, 5, &master).expect("trade");
        match frame {
            DecodedFrame::Record(r) => {
                assert_eq!(r.kind(), MdKind::Trade);
                assert_eq!(r.seq(), 99);
                assert_eq!(r.price().scaled(), 520_000);
                assert_eq!(r.qty().lots(), 2);
            }
            other => panic!("expected record, got {other:?}"),
        }
        let bbo = br#"{"schema":"mbp-1","symbol":"ESZ5","sequence":100,"bid_px":"5199.75","ask_px":"5200.25"}"#;
        let frame = vendor.decode_stream(bbo, 6, &master).expect("bbo");
        assert!(matches!(frame, DecodedFrame::Record(r) if r.kind() == MdKind::Bbo));
    }

    #[test]
    fn es_fut_alias_resolves_esz5() {
        let vendor = DatabentoGlbx::paper();
        let master = phase1_master();
        let trade =
            br#"{"schema":"trades","symbol":"ES.FUT","sequence":1,"price":"5200.50","size":1}"#;
        let frame = vendor.decode_stream(trade, 1, &master).expect("trade");
        match frame {
            DecodedFrame::Record(r) => assert_eq!(r.price().scaled(), 520_050),
            other => panic!("expected record, got {other:?}"),
        }
    }

    #[test]
    fn decode_snapshot_and_subscribe() {
        let vendor = DatabentoGlbx::live("KEY").with_dataset("GLBX.MDP3");
        let master = phase1_master();
        let body = br#"{"schema":"snapshot","symbol":"ESZ5","sequence":7,"price":"5210.00"}"#;
        let rec = vendor
            .decode_snapshot(body, 1, "ESZ5", &master)
            .expect("snap");
        assert_eq!(rec.kind(), MdKind::Snapshot);
        assert_eq!(rec.price().scaled(), 521_000);
        assert_eq!(rec.seq(), 7);
        let sub = String::from_utf8(vendor.subscribe_message(&["ESZ5".into()])).expect("utf8");
        assert!(sub.contains("subscribe"));
        assert!(sub.contains("ESZ5"));
        assert!(sub.contains("GLBX.MDP3"));
        let auth = String::from_utf8(vendor.auth_message()).expect("utf8");
        assert!(auth.contains("KEY"));
        let spec = vendor.snapshot_spec("ESZ5");
        assert!(spec.url().contains("GLBX.MDP3"));
        assert!(!spec.headers().is_empty());
    }

    #[test]
    fn paper_snapshot_spec_is_local() {
        let vendor = DatabentoGlbx::paper();
        let spec = vendor.snapshot_spec("ESZ5");
        assert!(spec.url().starts_with("paper://"));
    }
}
