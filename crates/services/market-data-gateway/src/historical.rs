//! Historical REST handlers (`GET /v1/bars`, `GET /v1/trades`).

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_instruments::InstrumentMaster;
use shinrai_market_data::{BarHistoryQuery, BarInterval, PageParams, TradeHistoryQuery};

use crate::app::{extract_bearer, lock_archive, lock_hub, unauthorized, AppState, AuthQuery};

/// Query string for `GET /v1/bars`.
#[derive(Debug, Deserialize)]
pub struct BarsQuery {
    symbol: String,
    interval: String,
    start: Option<u64>,
    end: Option<u64>,
    limit: Option<usize>,
    cursor: Option<u64>,
    token: Option<String>,
}

/// Query string for `GET /v1/trades`.
#[derive(Debug, Deserialize)]
pub struct TradesQuery {
    symbol: String,
    start: Option<u64>,
    end: Option<u64>,
    limit: Option<usize>,
    cursor: Option<u64>,
    token: Option<String>,
}

pub async fn get_bars(
    Query(query): Query<BarsQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(
        &headers,
        &AuthQuery {
            token: query.token.clone(),
        },
    );
    if let Err(err) = lock_hub(&state).preview_auth(token.as_deref()) {
        return unauthorized(err);
    }
    let interval = match parse_interval(&query.interval) {
        Ok(i) => i,
        Err(msg) => return bad_request(msg),
    };
    let instrument_id = match resolve(&state.master, &query.symbol) {
        Ok(id) => id,
        Err(err) => return api_error(err),
    };
    let page = lock_archive(&state).query_bars(BarHistoryQuery::new(
        instrument_id,
        interval,
        query.start,
        query.end,
        PageParams::new(query.limit, query.cursor),
    ));
    let bars: Vec<Value> = page
        .bars()
        .iter()
        .map(|b| {
            json!({
                "open_ts": b.open_ts(),
                "open_scaled": b.open().scaled(),
                "high_scaled": b.high().scaled(),
                "low_scaled": b.low().scaled(),
                "close_scaled": b.close().scaled(),
                "volume_lots": b.volume().lots(),
                "trade_count": b.trade_count(),
            })
        })
        .collect();
    Json(json!({
        "type": "bars",
        "symbol": query.symbol,
        "instrument_id": instrument_id.get(),
        "interval_secs": interval.duration(),
        "bars": bars,
        "next_cursor": page.next_cursor(),
    }))
    .into_response()
}

pub async fn get_trades(
    Query(query): Query<TradesQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(
        &headers,
        &AuthQuery {
            token: query.token.clone(),
        },
    );
    if let Err(err) = lock_hub(&state).preview_auth(token.as_deref()) {
        return unauthorized(err);
    }
    let instrument_id = match resolve(&state.master, &query.symbol) {
        Ok(id) => id,
        Err(err) => return api_error(err),
    };
    let page = lock_archive(&state).query_trades(TradeHistoryQuery::new(
        instrument_id,
        query.start,
        query.end,
        PageParams::new(query.limit, query.cursor),
    ));
    let trades: Vec<Value> = page
        .trades()
        .iter()
        .map(|t| {
            json!({
                "seq": t.seq(),
                "ts": t.ts_logical(),
                "price_scaled": t.price().scaled(),
                "qty_lots": t.qty().lots(),
            })
        })
        .collect();
    Json(json!({
        "type": "trades",
        "symbol": query.symbol,
        "instrument_id": instrument_id.get(),
        "trades": trades,
        "next_cursor": page.next_cursor(),
    }))
    .into_response()
}

fn resolve(
    master: &InstrumentMaster,
    symbol: &str,
) -> Result<shinrai_instruments::InstrumentId, shinrai_md_fanout::FanoutError> {
    shinrai_md_fanout::resolve_symbol(master, symbol)
}

fn parse_interval(raw: &str) -> Result<BarInterval, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("interval required");
    }
    if let Ok(secs) = raw.parse::<u64>() {
        return BarInterval::try_new(secs).map_err(|_| "interval must be positive");
    }
    match raw.to_ascii_lowercase().as_str() {
        "1s" | "sec" | "second" => Ok(BarInterval::SECOND),
        "1m" | "min" | "minute" => Ok(BarInterval::MINUTE),
        "1h" | "hour" => Ok(BarInterval::HOUR),
        "1d" | "day" => Ok(BarInterval::DAY),
        _ => Err("interval: use seconds (60) or 1s/1m/1h/1d"),
    }
}

fn bad_request(msg: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "type": "error", "code": "bad_request", "message": msg })),
    )
        .into_response()
}

fn api_error(err: shinrai_md_fanout::FanoutError) -> Response {
    let status = match err {
        shinrai_md_fanout::FanoutError::UnknownInstrument => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({ "type": "error", "code": err.code() }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interval_aliases() {
        assert_eq!(parse_interval("60").unwrap().duration(), 60);
        assert_eq!(parse_interval("1m").unwrap(), BarInterval::MINUTE);
        assert!(parse_interval("").is_err());
    }
}
