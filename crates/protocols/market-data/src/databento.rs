//! Databento-shaped CME Globex (`GLBX.MDP3`) futures market-data adapter.
//!
//! JSON wire covers trades, mbp-1 (BBO), and mbp-10 L2 (snapshot + level delta).
//! Live I/O talks to a licensed Databento-compatible endpoint when keys/URLs are
//! set; paper mode emits the same JSON through [`FeedSupervisor`].
//!
//! Raw CME MDP3 multicast (binary UDP) is **not** implemented here — use a
//! licensed Databento (or equivalent) gateway that publishes this JSON shape.

use serde_json::{json, Value};
use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks, QuantityLots};
use shinrai_market_data::{
    BookChange, BookDelta, BookEvent, BookLevel, BookSide, BookSnapshot, MdKind, MdRecord,
};

use crate::error::ProviderError;
use crate::vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};

/// CME Globex futures via Databento-shaped JSON (dataset default `GLBX.MDP3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabentoGlbx {
    api_key: String,
    dataset: String,
    /// Subscribe schema (`trades`, `mbp-1`, `mbp-10`, …).
    schema: String,
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
    /// Default subscribe schema (trades); set `mbp-10` for L2 depth.
    pub const SCHEMA_TRADES: &'static str = "trades";
    /// Top-of-book schema.
    pub const SCHEMA_MBP1: &'static str = "mbp-1";
    /// Ten-level book schema (Databento MBP-10).
    pub const SCHEMA_MBP10: &'static str = "mbp-10";

    /// Licensed live vendor (API key required for remote I/O).
    #[must_use]
    pub fn live(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            dataset: Self::DATASET_GLBX.into(),
            schema: Self::SCHEMA_TRADES.into(),
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
            schema: Self::SCHEMA_MBP10.into(),
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

    /// Overrides subscribe schema (`trades`, `mbp-1`, `mbp-10`).
    #[must_use]
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = schema.into();
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

    /// Subscribe schema.
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
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

    /// Paper trade-mark snapshot body for supervisor recovery (no HTTP).
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

    /// Paper mbp-10 L2 snapshot around a mid price (0.25 tick grid).
    #[must_use]
    pub fn paper_book_body(product_id: &str, sequence: u64, mid_decimal: &str) -> Vec<u8> {
        let mid: f64 = mid_decimal.parse().unwrap_or(5_200.0);
        let tick = 0.25_f64;
        let mut bids = Vec::new();
        let mut asks = Vec::new();
        for i in 1_u8..=10 {
            let depth = f64::from(i);
            let bid = ((mid - tick * depth) / tick).round() * tick;
            let ask = ((mid + tick * depth) / tick).round() * tick;
            let sz = i64::from(i);
            bids.push(json!([format!("{bid:.2}"), sz]));
            asks.push(json!([format!("{ask:.2}"), sz]));
        }
        json!({
            "schema": "mbp-10",
            "symbol": product_id,
            "sequence": sequence,
            "bids": bids,
            "asks": asks,
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
            "schema": self.schema,
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
        let schema = if self.schema == Self::SCHEMA_MBP10 || self.schema == Self::SCHEMA_MBP1 {
            self.schema.as_str()
        } else {
            Self::SCHEMA_TRADES
        };
        SnapshotSpec::get_with_headers(
            format!(
                "{}/v0/timeseries.get_range?dataset={}&symbols={}&schema={schema}&limit=1",
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
            "mbp-10" | "mbp" | "book" | "l2" => decode_book_frame(&value, ts_logical, master),
            "mbp-10-delta" | "l2update" | "book_delta" => {
                decode_book_delta_frame(&value, ts_logical, master)
            }
            "snapshot" => {
                // Price-only recovery snapshot → record; book snapshots use mbp-10.
                if value.get("bids").is_some() || value.get("levels").is_some() {
                    decode_book_frame(&value, ts_logical, master)
                } else {
                    Ok(DecodedFrame::Control)
                }
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
        let price = if let Ok(p) = Self::price_from_value(&value) {
            p
        } else if let Some(p) = first_book_mid(&value) {
            p
        } else {
            return Err(ProviderError::MissingField("price"));
        };
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
        let value: Value = serde_json::from_slice(raw).map_err(|_| ProviderError::InvalidJson)?;
        let mut value = value;
        if value.get("symbol").is_none() {
            value
                .as_object_mut()
                .map(|o| o.insert("symbol".into(), json!(product_id)));
        }
        match decode_book_frame(&value, ts_logical, master)? {
            DecodedFrame::Book(event) => Ok(event),
            _ => Err(ProviderError::MissingField("bids|asks|levels")),
        }
    }
}

fn decode_book_frame(
    value: &Value,
    ts_logical: u64,
    master: &InstrumentMaster,
) -> Result<DecodedFrame, ProviderError> {
    let symbol = json_str(value, "symbol")?;
    let instrument_id = DatabentoGlbx::resolve_product(master, symbol)?;
    let seq = value
        .get("sequence")
        .and_then(json_u64_opt)
        .unwrap_or(1)
        .max(1);
    // Single-level delta style on an mbp-10 channel.
    if value.get("side").is_some() && value.get("price").is_some() && value.get("bids").is_none() {
        return decode_book_delta_frame(value, ts_logical, master);
    }
    let (bids, asks) = decode_book_levels(master, instrument_id, value)?;
    if bids.is_empty() && asks.is_empty() {
        return Err(ProviderError::MissingField("bids|asks|levels"));
    }
    Ok(DecodedFrame::Book(BookEvent::Snapshot(BookSnapshot::new(
        instrument_id,
        seq,
        ts_logical,
        bids,
        asks,
    ))))
}

fn decode_book_delta_frame(
    value: &Value,
    ts_logical: u64,
    master: &InstrumentMaster,
) -> Result<DecodedFrame, ProviderError> {
    let symbol = json_str(value, "symbol")?;
    let instrument_id = DatabentoGlbx::resolve_product(master, symbol)?;
    let seq = value.get("sequence").and_then(json_u64_opt);
    let changes = if let Some(rows) = value.get("changes").and_then(Value::as_array) {
        decode_changes_array(master, instrument_id, rows)?
    } else {
        let side = match value
            .get("side")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase()
            .as_str()
        {
            "B" | "BID" | "BUY" | "1" => BookSide::Bid,
            "A" | "ASK" | "SELL" | "2" => BookSide::Ask,
            _ => return Err(ProviderError::MissingField("side")),
        };
        let price = value
            .get("price")
            .and_then(Value::as_str)
            .or_else(|| value.get("px").and_then(Value::as_str))
            .ok_or(ProviderError::MissingField("price"))?;
        let instrument = master.get(instrument_id)?;
        let ticks = instrument.price_to_ticks(price)?;
        let qty = level_qty(instrument, value.get("size").or_else(|| value.get("sz")))?;
        vec![BookChange::new(side, ticks, qty)]
    };
    Ok(DecodedFrame::Book(BookEvent::Delta(BookDelta::new(
        instrument_id,
        seq,
        ts_logical,
        changes,
    ))))
}

fn decode_book_levels(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    value: &Value,
) -> Result<(Vec<BookLevel>, Vec<BookLevel>), ProviderError> {
    if value.get("levels").and_then(Value::as_array).is_some() {
        return decode_databento_levels(master, instrument_id, value);
    }
    let bids = decode_side_levels(master, instrument_id, value, "bids")?;
    let asks = decode_side_levels(master, instrument_id, value, "asks")?;
    Ok((bids, asks))
}

fn decode_databento_levels(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    value: &Value,
) -> Result<(Vec<BookLevel>, Vec<BookLevel>), ProviderError> {
    let rows = value
        .get("levels")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MissingField("levels"))?;
    let instrument = master.get(instrument_id)?;
    let mut bids = Vec::new();
    let mut asks = Vec::new();
    for row in rows {
        if let Some(bid_px) = row
            .get("bid_px")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let qty = level_qty(
                instrument,
                row.get("bid_sz").or_else(|| row.get("bid_size")),
            )?;
            if qty.lots() > 0 {
                bids.push(BookLevel::new(instrument.price_to_ticks(bid_px)?, qty));
            }
        }
        if let Some(ask_px) = row
            .get("ask_px")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let qty = level_qty(
                instrument,
                row.get("ask_sz").or_else(|| row.get("ask_size")),
            )?;
            if qty.lots() > 0 {
                asks.push(BookLevel::new(instrument.price_to_ticks(ask_px)?, qty));
            }
        }
    }
    Ok((bids, asks))
}

fn decode_side_levels(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    value: &Value,
    side: &'static str,
) -> Result<Vec<BookLevel>, ProviderError> {
    let Some(rows) = value.get(side).and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let instrument = master.get(instrument_id)?;
    let mut levels = Vec::new();
    for row in rows {
        let (price, qty_cell) = match row {
            Value::Array(pair) => {
                let price = pair
                    .first()
                    .and_then(Value::as_str)
                    .ok_or(ProviderError::MissingField(side))?;
                (price, pair.get(1))
            }
            Value::Object(map) => {
                let price = map
                    .get("price")
                    .or_else(|| map.get("px"))
                    .and_then(Value::as_str)
                    .ok_or(ProviderError::MissingField(side))?;
                (price, map.get("size").or_else(|| map.get("sz")))
            }
            _ => return Err(ProviderError::MissingField(side)),
        };
        let qty = level_qty(instrument, qty_cell)?;
        if qty.lots() > 0 {
            levels.push(BookLevel::new(instrument.price_to_ticks(price)?, qty));
        }
    }
    Ok(levels)
}

fn decode_changes_array(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    rows: &[Value],
) -> Result<Vec<BookChange>, ProviderError> {
    let instrument = master.get(instrument_id)?;
    let mut changes = Vec::new();
    for row in rows {
        let pair = row
            .as_array()
            .ok_or(ProviderError::MissingField("changes"))?;
        let side = match pair
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "b" | "bid" | "buy" => BookSide::Bid,
            "a" | "ask" | "sell" => BookSide::Ask,
            _ => return Err(ProviderError::MissingField("side")),
        };
        let price = pair
            .get(1)
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField("price"))?;
        let qty = level_qty(instrument, pair.get(2))?;
        changes.push(BookChange::new(
            side,
            instrument.price_to_ticks(price)?,
            qty,
        ));
    }
    Ok(changes)
}

fn level_qty(
    instrument: &shinrai_instruments::Instrument,
    cell: Option<&Value>,
) -> Result<QuantityLots, ProviderError> {
    let Some(cell) = cell else {
        return Ok(QuantityLots::from_lots(0));
    };
    if let Some(n) = json_i64_opt(cell) {
        return Ok(QuantityLots::from_lots(n.max(0)));
    }
    if let Some(s) = cell.as_str() {
        return Ok(instrument.size_to_lots(s)?);
    }
    Ok(QuantityLots::from_lots(0))
}

fn first_book_mid(value: &Value) -> Option<String> {
    let bid = value
        .get("bids")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|row| {
            row.as_array()
                .and_then(|p| p.first())
                .and_then(Value::as_str)
                .or_else(|| row.get("price").and_then(Value::as_str))
        })?;
    let ask = value
        .get("asks")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|row| {
            row.as_array()
                .and_then(|p| p.first())
                .and_then(Value::as_str)
                .or_else(|| row.get("price").and_then(Value::as_str))
        })?;
    mid_decimal(bid, ask)
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
    use shinrai_market_data::BookEvent;

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
    fn decode_mbp10_book_and_delta() {
        let vendor = DatabentoGlbx::paper();
        let master = phase1_master();
        let book = br#"{
            "schema":"mbp-10","symbol":"ESZ5","sequence":42,
            "bids":[["5199.75",5],["5199.50",10]],
            "asks":[["5200.00",3],["5200.25",8]]
        }"#;
        let frame = vendor.decode_stream(book, 1, &master).expect("book");
        match frame {
            DecodedFrame::Book(BookEvent::Snapshot(snap)) => {
                assert_eq!(snap.seq(), 42);
                assert_eq!(snap.bids().len(), 2);
                assert_eq!(snap.asks().len(), 2);
                assert_eq!(snap.bids()[0].qty().lots(), 5);
                assert_eq!(snap.asks()[0].price().scaled(), 520_000);
            }
            other => panic!("expected book snapshot, got {other:?}"),
        }
        let delta = br#"{
            "schema":"mbp-10-delta","symbol":"ESZ5","sequence":43,
            "side":"B","price":"5199.75","size":0
        }"#;
        let frame = vendor.decode_stream(delta, 2, &master).expect("delta");
        match frame {
            DecodedFrame::Book(BookEvent::Delta(d)) => {
                assert_eq!(d.seq(), Some(43));
                assert_eq!(d.changes().len(), 1);
                assert_eq!(d.changes()[0].qty().lots(), 0);
            }
            other => panic!("expected book delta, got {other:?}"),
        }
        let levels = br#"{
            "schema":"mbp-10","symbol":"ESZ5","sequence":44,
            "levels":[
              {"bid_px":"5199.75","bid_sz":2,"ask_px":"5200.00","ask_sz":4},
              {"bid_px":"5199.50","bid_sz":1,"ask_px":"5200.25","ask_sz":1}
            ]
        }"#;
        let BookEvent::Snapshot(snap) = vendor
            .decode_book_snapshot(levels, 3, "ESZ5", &master)
            .expect("levels")
        else {
            panic!("snapshot");
        };
        assert_eq!(snap.bids().len(), 2);
        assert_eq!(snap.asks().len(), 2);
    }

    #[test]
    fn paper_book_body_has_ten_levels() {
        let body = DatabentoGlbx::paper_book_body("ESZ5", 7, "5200.00");
        let vendor = DatabentoGlbx::paper();
        let master = phase1_master();
        let BookEvent::Snapshot(snap) = vendor
            .decode_book_snapshot(&body, 1, "ESZ5", &master)
            .expect("paper book")
        else {
            panic!("snapshot");
        };
        assert_eq!(snap.seq(), 7);
        assert_eq!(snap.bids().len(), 10);
        assert_eq!(snap.asks().len(), 10);
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
        let vendor = DatabentoGlbx::live("KEY")
            .with_dataset("GLBX.MDP3")
            .with_schema("mbp-10");
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
        assert!(sub.contains("mbp-10"));
        let auth = String::from_utf8(vendor.auth_message()).expect("utf8");
        assert!(auth.contains("KEY"));
        let spec = vendor.snapshot_spec("ESZ5");
        assert!(spec.url().contains("GLBX.MDP3"));
        assert!(spec.url().contains("mbp-10"));
        assert!(!spec.headers().is_empty());
    }

    #[test]
    fn paper_snapshot_spec_is_local() {
        let vendor = DatabentoGlbx::paper();
        let spec = vendor.snapshot_spec("ESZ5");
        assert!(spec.url().starts_with("paper://"));
        assert_eq!(vendor.schema(), "mbp-10");
    }
}
