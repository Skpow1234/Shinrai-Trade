//! Coinbase Exchange public market-data adapter.

use serde_json::{json, Value};
use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks, QuantityLots};
use shinrai_market_data::{
    BookChange, BookDelta, BookEvent, BookLevel, BookSide, BookSnapshot, MdKind, MdRecord,
};

use crate::error::ProviderError;
use crate::vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};

/// Public Coinbase Exchange sandbox feed (no API key for ticker/heartbeat).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoinbaseExchange;

impl CoinbaseExchange {
    /// Public WebSocket feed.
    pub const WS_URL: &'static str = "wss://ws-feed.exchange.coinbase.com";
    /// REST origin for snapshots.
    pub const REST_ORIGIN: &'static str = "https://api.exchange.coinbase.com";

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
        price: &str,
    ) -> Result<PriceTicks, ProviderError> {
        let instrument = master.get(instrument_id)?;
        Ok(instrument.price_to_ticks(price)?)
    }
}

impl MarketDataVendor for CoinbaseExchange {
    fn id(&self) -> VendorId {
        VendorId::COINBASE_EXCHANGE
    }

    fn websocket_url(&self) -> &'static str {
        Self::WS_URL
    }

    fn subscribe_message(&self, product_ids: &[String]) -> Vec<u8> {
        json!({
            "type": "subscribe",
            "product_ids": product_ids,
            "channels": ["ticker", "heartbeat", "matches", "level2"],
        })
        .to_string()
        .into_bytes()
    }

    fn snapshot_spec(&self, product_id: &str) -> SnapshotSpec {
        SnapshotSpec::get(format!(
            "{}/products/{product_id}/book?level=2",
            Self::REST_ORIGIN
        ))
    }

    fn decode_stream(
        &self,
        raw: &[u8],
        ts_logical: u64,
        master: &InstrumentMaster,
    ) -> Result<DecodedFrame, ProviderError> {
        let value: Value = serde_json::from_slice(raw).map_err(|_| ProviderError::InvalidJson)?;
        let msg_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField("type"))?;

        match msg_type {
            "heartbeat" => {
                let product_id = json_str(&value, "product_id")?;
                let instrument_id = Self::resolve_product(master, product_id)?;
                Ok(DecodedFrame::Heartbeat { instrument_id })
            }
            "ticker" | "match" | "last_match" => {
                let product_id = json_str(&value, "product_id")?;
                let instrument_id = Self::resolve_product(master, product_id)?;
                let seq = json_u64(&value, "sequence")?;
                let price = json_str(&value, "price")?;
                let ticks = Self::price_ticks(master, instrument_id, price)?;
                let qty = optional_qty(master, instrument_id, &value)?;
                let kind = if msg_type == "ticker" {
                    MdKind::Bbo
                } else {
                    MdKind::Trade
                };
                Ok(DecodedFrame::Record(
                    MdRecord::new(instrument_id, seq, ts_logical, kind, ticks).with_qty(qty),
                ))
            }
            "snapshot" => {
                let product_id = json_str(&value, "product_id")?;
                Ok(DecodedFrame::Book(decode_book_value(
                    &value, ts_logical, product_id, master,
                )?))
            }
            "l2update" => {
                let product_id = json_str(&value, "product_id")?;
                let instrument_id = Self::resolve_product(master, product_id)?;
                let seq = value.get("sequence").and_then(json_u64_opt);
                let changes = decode_changes(master, instrument_id, &value)?;
                Ok(DecodedFrame::Book(BookEvent::Delta(BookDelta::new(
                    instrument_id,
                    seq,
                    ts_logical,
                    changes,
                ))))
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
        let seq = json_u64(&value, "sequence")?;
        let price = snapshot_price(&value)?;
        let ticks = Self::price_ticks(master, instrument_id, price)?;
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
        decode_book_value(&value, ts_logical, product_id, master)
    }
}

fn optional_qty(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    value: &Value,
) -> Result<QuantityLots, ProviderError> {
    let size = value
        .get("size")
        .and_then(Value::as_str)
        .or_else(|| value.get("last_size").and_then(Value::as_str));
    match size {
        None => Ok(QuantityLots::from_lots(0)),
        Some(decimal) => {
            let instrument = master.get(instrument_id)?;
            Ok(instrument.qty_to_lots(decimal)?)
        }
    }
}

fn json_str<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, ProviderError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(ProviderError::MissingField(field))
}

fn json_u64(value: &Value, field: &'static str) -> Result<u64, ProviderError> {
    let cell = value.get(field).ok_or(ProviderError::MissingField(field))?;
    json_u64_opt(cell).ok_or(ProviderError::MissingField(field))
}

fn json_u64_opt(cell: &Value) -> Option<u64> {
    match cell {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn decode_book_value(
    value: &Value,
    ts_logical: u64,
    product_id: &str,
    master: &InstrumentMaster,
) -> Result<BookEvent, ProviderError> {
    let instrument_id = CoinbaseExchange::resolve_product(master, product_id)?;
    let seq = value.get("sequence").and_then(json_u64_opt).unwrap_or(1);
    if seq == 0 {
        return Err(ProviderError::MissingField("sequence"));
    }
    let bids = decode_levels(master, instrument_id, value, "bids")?;
    let asks = decode_levels(master, instrument_id, value, "asks")?;
    Ok(BookEvent::Snapshot(BookSnapshot::new(
        instrument_id,
        seq,
        ts_logical,
        bids,
        asks,
    )))
}

fn decode_levels(
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
        let pair = row.as_array().ok_or(ProviderError::MissingField(side))?;
        let price = pair
            .first()
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField(side))?;
        let qty_text = pair
            .get(1)
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField(side))?;
        let ticks = instrument.price_to_ticks(price)?;
        let qty = instrument.size_to_lots(qty_text)?;
        if qty.lots() > 0 {
            levels.push(BookLevel::new(ticks, qty));
        }
    }
    Ok(levels)
}

fn decode_changes(
    master: &InstrumentMaster,
    instrument_id: InstrumentId,
    value: &Value,
) -> Result<Vec<BookChange>, ProviderError> {
    let rows = value
        .get("changes")
        .and_then(Value::as_array)
        .ok_or(ProviderError::MissingField("changes"))?;
    let instrument = master.get(instrument_id)?;
    let mut changes = Vec::new();
    for row in rows {
        let pair = row
            .as_array()
            .ok_or(ProviderError::MissingField("changes"))?;
        let side = match pair.first().and_then(Value::as_str) {
            Some("buy") => BookSide::Bid,
            Some("sell") => BookSide::Ask,
            _ => return Err(ProviderError::MissingField("side")),
        };
        let price = pair
            .get(1)
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField("price"))?;
        let qty_text = pair
            .get(2)
            .and_then(Value::as_str)
            .ok_or(ProviderError::MissingField("size"))?;
        changes.push(BookChange::new(
            side,
            instrument.price_to_ticks(price)?,
            instrument.size_to_lots(qty_text)?,
        ));
    }
    Ok(changes)
}

fn snapshot_price(value: &Value) -> Result<&str, ProviderError> {
    if let Some(price) = first_level_price(value, "bids") {
        return Ok(price);
    }
    if let Some(price) = first_level_price(value, "asks") {
        return Ok(price);
    }
    Err(ProviderError::MissingField("bids"))
}

fn first_level_price<'a>(value: &'a Value, side: &str) -> Option<&'a str> {
    value
        .get(side)?
        .as_array()?
        .first()?
        .as_array()?
        .first()?
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::{btc_usd, phase1_master};

    #[test]
    fn ticker_normalizes_without_floats() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let raw =
            br#"{"type":"ticker","sequence":50001,"product_id":"BTC-USD","price":"65000.12"}"#;
        let frame = vendor.decode_stream(raw, 7, &master).expect("decode");
        match frame {
            DecodedFrame::Record(record) => {
                assert_eq!(record.instrument_id(), btc_usd().id());
                assert_eq!(record.seq(), 50_001);
                assert_eq!(record.price().scaled(), 6_500_012);
                assert_eq!(record.kind(), MdKind::Bbo);
                assert_eq!(record.ts_logical(), 7);
            }
            other => panic!("expected record, got {other:?}"),
        }
    }

    #[test]
    fn unknown_product_is_error() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let raw = br#"{"type":"ticker","sequence":1,"product_id":"ETH-USD","price":"1.00"}"#;
        let err = vendor.decode_stream(raw, 0, &master).expect_err("unknown");
        assert!(matches!(err, ProviderError::UnknownInstrument { .. }));
    }

    #[test]
    fn snapshot_uses_best_bid() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let raw =
            br#"{"sequence":"50010","bids":[["65000.10","1.0",1]],"asks":[["65000.12","2.0",1]]}"#;
        let record = vendor
            .decode_snapshot(raw, 3, "BTC-USD", &master)
            .expect("snap");
        assert_eq!(record.kind(), MdKind::Snapshot);
        assert_eq!(record.seq(), 50_010);
        assert_eq!(record.price().scaled(), 6_500_010);
    }

    #[test]
    fn subscribe_lists_heartbeat_and_ticker() {
        let msg = CoinbaseExchange.subscribe_message(&["BTC-USD".into()]);
        let text = String::from_utf8(msg).expect("utf8");
        assert!(text.contains("heartbeat"));
        assert!(text.contains("ticker"));
        assert!(text.contains("matches"));
        assert!(text.contains("level2"));
        assert!(text.contains("BTC-USD"));
    }

    #[test]
    fn book_snapshot_and_l2update() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let raw =
            br#"{"sequence":10,"bids":[["65000.10","1.0",1]],"asks":[["65000.12","0.01",1]]}"#;
        let BookEvent::Snapshot(snap) = vendor
            .decode_book_snapshot(raw, 0, "BTC-USD", &master)
            .expect("snap")
        else {
            panic!("snapshot");
        };
        assert_eq!(snap.seq(), 10);
        assert_eq!(snap.bids()[0].qty().lots(), 100_000_000);
        assert_eq!(snap.asks()[0].qty().lots(), 1_000_000);

        let upd = br#"{"type":"l2update","product_id":"BTC-USD","changes":[["buy","65000.10","0.00000000"],["sell","65000.12","0.02"]]}"#;
        match vendor.decode_stream(upd, 1, &master).expect("upd") {
            DecodedFrame::Book(BookEvent::Delta(delta)) => {
                assert_eq!(delta.changes().len(), 2);
                assert_eq!(delta.changes()[0].qty().lots(), 0);
                assert_eq!(delta.changes()[1].qty().lots(), 2_000_000);
            }
            other => panic!("expected delta, got {other:?}"),
        }
    }

    #[test]
    fn match_carries_lot_quantity() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let raw = br#"{"type":"match","sequence":9,"product_id":"BTC-USD","price":"65000.12","size":"0.01"}"#;
        let frame = vendor.decode_stream(raw, 1, &master).expect("decode");
        match frame {
            DecodedFrame::Record(record) => {
                assert_eq!(record.kind(), MdKind::Trade);
                assert_eq!(record.qty().lots(), 1_000_000);
            }
            other => panic!("expected record, got {other:?}"),
        }
    }
}
