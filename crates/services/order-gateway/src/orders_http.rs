//! Order HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_instruments::{ExternalId, InstrumentId, PriceTicks, QuantityLots};
use shinrai_md_fanout::Authenticator;
use shinrai_orders::{ClientOrderId, Order, OrderId, Side, SubmitOutcome};
use shinrai_paper::{PaperError, SubmitRequest};
use tracing::{info_span, Instrument};

use crate::app::{
    extract_bearer, lock_engine, record_fill_mark, resolve_account, unauthorized, AppState,
    AuthQuery,
};

#[derive(Debug, Deserialize)]
pub struct SubmitOrderBody {
    client_order_id: String,
    symbol: String,
    side: String,
    qty: i64,
    price: i64,
}

#[derive(Debug, Deserialize)]
pub struct OrderAuthQuery {
    token: Option<String>,
}

/// `GET /v1/orders` — list orders for the authenticated account.
pub async fn list_orders(
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(&headers, &auth);
    let now = crate::app::unix_logical_now();
    let claims = match state.auth.authenticate(token.as_deref(), now) {
        Ok(c) => c,
        Err(err) => return unauthorized(err),
    };
    let account = match resolve_account(&state, claims.subject().as_str()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };
    let engine = lock_engine(&state);
    let orders: Vec<Value> = engine
        .orders()
        .orders()
        .filter(|o| o.account_id() == account)
        .map(|o| order_json(&state, o))
        .collect();
    (StatusCode::OK, Json(json!({ "orders": orders }))).into_response()
}

/// `POST /v1/orders` — submit a paper limit buy.
#[allow(clippy::too_many_lines)]
pub async fn post_order(
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<SubmitOrderBody>,
) -> Response {
    let token = extract_bearer(&headers, &auth);
    let now = crate::app::unix_logical_now();
    let claims = match state.auth.authenticate(token.as_deref(), now) {
        Ok(c) => c,
        Err(err) => return unauthorized(err),
    };
    let account = match resolve_account(&state, claims.subject().as_str()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };

    let span = info_span!(
        "order.submit",
        account_id = account.get(),
        client_order_id = %body.client_order_id.trim(),
        symbol = %body.symbol.trim(),
        side = %body.side.trim(),
        qty = body.qty,
        price = body.price,
        order_id = tracing::field::Empty,
        outcome = tracing::field::Empty,
    );

    async move {
        state.metrics.record_submit();

        let Ok(client_order_id) = ClientOrderId::new(body.client_order_id.trim()) else {
            tracing::Span::current().record("outcome", "invalid_client_order_id");
            return bad_request("invalid_client_order_id");
        };
        let side = match parse_side(&body.side) {
            Ok(s) => s,
            Err(code) => {
                tracing::Span::current().record("outcome", code);
                return bad_request(code);
            }
        };
        if body.qty <= 0 || body.price <= 0 {
            tracing::Span::current().record("outcome", "invalid_qty_or_price");
            return bad_request("invalid_qty_or_price");
        }
        let instrument_id = match resolve_symbol(&state.master, body.symbol.trim()) {
            Ok(id) => id,
            Err(code) => {
                tracing::Span::current().record("outcome", code);
                return bad_request(code);
            }
        };

        let req = SubmitRequest {
            account_id: account,
            client_order_id,
            instrument_id,
            side,
            qty: QuantityLots::from_lots(body.qty),
            price: PriceTicks::from_scaled(body.price),
        };

        let outcome = {
            let mut engine = lock_engine(&state);
            engine.set_logical_now(now);
            engine.submit(&req)
        };

        match outcome {
            Ok(SubmitOutcome::Created(order)) => {
                let span = tracing::Span::current();
                span.record("order_id", order.id().get());
                span.record("outcome", "created");
                state.metrics.record_accepted();
                record_fill_mark(&state, &order);
                state.maybe_persist(Some(&order)).await;
                (StatusCode::OK, Json(order_json(&state, &order))).into_response()
            }
            Ok(SubmitOutcome::Duplicate(order)) => {
                let span = tracing::Span::current();
                span.record("order_id", order.id().get());
                span.record("outcome", "duplicate");
                state.metrics.record_accepted();
                record_fill_mark(&state, &order);
                state.maybe_persist(Some(&order)).await;
                (StatusCode::OK, Json(order_json(&state, &order))).into_response()
            }
            Err(PaperError::Risk(reason)) => {
                tracing::Span::current().record("outcome", reason.code());
                state.metrics.record_risk_rejected_code(reason.code());
                state.maybe_persist(None).await;
                risk_rejected(reason.code())
            }
            Err(PaperError::Instrument(_) | PaperError::Order(_)) => {
                tracing::Span::current().record("outcome", "invalid_order");
                bad_request("invalid_order")
            }
            Err(PaperError::Ledger(_)) => {
                tracing::Span::current().record("outcome", "ledger_error");
                (
                    StatusCode::CONFLICT,
                    Json(json!({ "type": "error", "code": "ledger_error" })),
                )
                    .into_response()
            }
            Err(other) => {
                tracing::Span::current().record("outcome", "internal");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        json!({ "type": "error", "code": "internal", "detail": other.to_string() }),
                    ),
                )
                    .into_response()
            }
        }
    }
    .instrument(span)
    .await
}

/// `GET /v1/orders/:id`
pub async fn get_order(
    Path(id): Path<u64>,
    headers: HeaderMap,
    Query(auth): Query<OrderAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(
        &headers,
        &AuthQuery {
            token: auth.token.clone(),
        },
    );
    let now = crate::app::unix_logical_now();
    let claims = match state.auth.authenticate(token.as_deref(), now) {
        Ok(c) => c,
        Err(err) => return unauthorized(err),
    };
    let account = match resolve_account(&state, claims.subject().as_str()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };

    let order_id = OrderId::from_u64(id);
    let engine = lock_engine(&state);
    let order = match engine.orders().get(order_id) {
        Ok(o) => o.clone(),
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "type": "error", "code": "not_found" })),
            )
                .into_response();
        }
    };
    if order.account_id() != account {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "type": "error", "code": "not_found" })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(order_json(&state, &order))).into_response()
}

/// `POST /v1/orders/:id/cancel`
pub async fn post_cancel(
    Path(id): Path<u64>,
    headers: HeaderMap,
    Query(auth): Query<OrderAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(
        &headers,
        &AuthQuery {
            token: auth.token.clone(),
        },
    );
    let now = crate::app::unix_logical_now();
    let claims = match state.auth.authenticate(token.as_deref(), now) {
        Ok(c) => c,
        Err(err) => return unauthorized(err),
    };
    let account = match resolve_account(&state, claims.subject().as_str()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };

    let span = info_span!(
        "order.cancel",
        account_id = account.get(),
        order_id = id,
        outcome = tracing::field::Empty,
    );

    async move {
        let order_id = OrderId::from_u64(id);
        let canceled = {
            let mut engine = lock_engine(&state);
            let existing = match engine.orders().get(order_id) {
                Ok(o) if o.account_id() == account => o.clone(),
                Ok(_) | Err(_) => {
                    tracing::Span::current().record("outcome", "not_found");
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "type": "error", "code": "not_found" })),
                    )
                        .into_response();
                }
            };
            match engine.cancel(order_id) {
                Ok(o) => o,
                Err(PaperError::Order(_)) => existing,
                Err(other) => {
                    tracing::Span::current().record("outcome", "cancel_failed");
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({ "type": "error", "code": "cancel_failed", "detail": other.to_string() })),
                    )
                        .into_response();
                }
            }
        };
        tracing::Span::current().record("outcome", "canceled");
        state.metrics.record_canceled();
        state.maybe_persist(Some(&canceled)).await;
        (StatusCode::OK, Json(order_json(&state, &canceled))).into_response()
    }
    .instrument(span)
    .await
}

fn order_json(state: &AppState, order: &Order) -> Value {
    let symbol = state
        .master
        .get(order.instrument_id())
        .map_or_else(|_| "?".into(), |i| i.symbol_display().to_owned());
    json!({
        "id": order.id().get(),
        "client_order_id": order.client_order_id().as_str(),
        "account_id": order.account_id().get(),
        "instrument_id": order.instrument_id().get(),
        "symbol": symbol,
        "side": side_str(order.side()),
        "status": order.status().to_string(),
        "order_qty": order.order_qty().lots(),
        "price": order.price().scaled(),
        "cum_qty": order.cum_qty().lots(),
        "leaves_qty": order.leaves_qty().lots(),
        "reject_reason": order.reject_reason(),
    })
}

fn parse_side(raw: &str) -> Result<Side, &'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        _ => Err("invalid_side"),
    }
}

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "Buy",
        Side::Sell => "Sell",
    }
}

fn resolve_symbol(
    master: &shinrai_instruments::InstrumentMaster,
    symbol: &str,
) -> Result<InstrumentId, &'static str> {
    if symbol.is_empty() {
        return Err("invalid_symbol");
    }
    let alias = ExternalId::ticker(symbol).map_err(|_| "invalid_symbol")?;
    master.resolve_alias(&alias).map_err(|_| "unknown_symbol")
}

fn bad_request(code: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "type": "error", "code": code })),
    )
        .into_response()
}

fn risk_rejected(code: &'static str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "type": "error", "code": code })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_parse() {
        assert_eq!(parse_side("Buy"), Ok(Side::Buy));
        assert!(parse_side("short").is_err());
    }
}
