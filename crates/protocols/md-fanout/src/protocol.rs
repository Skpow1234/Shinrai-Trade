//! JSON client protocol. Invalid frames do not echo payloads (tokens).

use serde_json::{json, Value};
use shinrai_market_data::MdRecord;

use crate::error::FanoutError;
use crate::session::{ClientMessage, MarketEvent};

/// Inbound command after connect (token is not accepted here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCommand {
    /// Subscribe by ticker / alias.
    Subscribe {
        /// External symbol (resolved via the instrument master).
        symbol: String,
    },
    /// Unsubscribe by ticker / alias.
    Unsubscribe {
        /// External symbol.
        symbol: String,
    },
    /// Client liveness.
    Heartbeat,
}

/// Decodes a client text frame.
///
/// # Errors
///
/// Returns [`FanoutError::InvalidCommand`] without including the raw payload.
pub fn decode_command(raw: &[u8]) -> Result<ClientCommand, FanoutError> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| FanoutError::InvalidCommand)?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(FanoutError::InvalidCommand)?;
    match kind {
        "subscribe" => {
            let symbol = required_symbol(&value)?;
            Ok(ClientCommand::Subscribe { symbol })
        }
        "unsubscribe" => {
            let symbol = required_symbol(&value)?;
            Ok(ClientCommand::Unsubscribe { symbol })
        }
        "heartbeat" | "pong" => Ok(ClientCommand::Heartbeat),
        _ => Err(FanoutError::InvalidCommand),
    }
}

fn required_symbol(value: &Value) -> Result<String, FanoutError> {
    let symbol = value
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or(FanoutError::InvalidCommand)?;
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(FanoutError::InvalidCommand);
    }
    Ok(symbol.to_owned())
}

/// Encodes an outbound message.
#[must_use]
pub fn encode_message(msg: &ClientMessage) -> Vec<u8> {
    let value = match msg {
        ClientMessage::Market(MarketEvent::Tick(record)) => tick_json(record),
        ClientMessage::Market(MarketEvent::Degraded { instrument_id }) => json!({
            "type": "degraded",
            "instrument_id": instrument_id.get(),
        }),
        ClientMessage::Market(MarketEvent::BookReady {
            instrument_id,
            checksum,
        }) => json!({
            "type": "book_ready",
            "instrument_id": instrument_id.get(),
            "checksum": checksum,
        }),
        ClientMessage::Heartbeat {
            ts_logical,
            dropped,
        } => json!({
            "type": "heartbeat",
            "ts": ts_logical,
            "dropped": dropped,
        }),
        ClientMessage::Subscribed { instrument_id } => json!({
            "type": "subscribed",
            "instrument_id": instrument_id.get(),
        }),
        ClientMessage::Error { code } => json!({
            "type": "error",
            "code": code,
        }),
    };
    value.to_string().into_bytes()
}

fn tick_json(record: &MdRecord) -> Value {
    json!({
        "type": "tick",
        "instrument_id": record.instrument_id().get(),
        "seq": record.seq(),
        "ts": record.ts_logical(),
        "kind": kind_name(record),
        "price_scaled": record.price().scaled(),
        "qty_lots": record.qty().lots(),
    })
}

fn kind_name(record: &MdRecord) -> &'static str {
    use shinrai_market_data::MdKind;
    match record.kind() {
        MdKind::Trade => "trade",
        MdKind::Bbo => "bbo",
        MdKind::Book => "book",
        MdKind::Status => "status",
        MdKind::Snapshot => "snapshot",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;

    #[test]
    fn decode_rejects_garbage_without_payload() {
        let err = decode_command(br"not json").expect_err("bad");
        assert_eq!(err, FanoutError::InvalidCommand);
        assert!(!err.to_string().contains("not json"));
    }

    #[test]
    fn subscribe_round_trip_shape() {
        let cmd = decode_command(br#"{"type":"subscribe","symbol":"BTC-USD"}"#).expect("ok");
        assert_eq!(
            cmd,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into()
            }
        );
        let bytes = encode_message(&ClientMessage::Subscribed {
            instrument_id: InstrumentId::from_u64(3),
        });
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("subscribed"));
        assert!(text.contains('3'));
    }
}
