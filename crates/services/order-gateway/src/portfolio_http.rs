//! Portfolio, audit, reconciliation, and metrics HTTP handlers.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_audit::{AuditKind, AuditRecord};
use shinrai_instruments::{ExternalId, InstrumentId, PriceTicks};
use shinrai_ledger::AccountId;
use shinrai_md_fanout::Authenticator;
use shinrai_money::MoneyError;
use shinrai_portfolio::{build_snapshot, PortfolioSnapshot};

use crate::app::{extract_bearer, lock_engine, resolve_account, unauthorized, AppState, AuthQuery};

#[derive(Debug, Deserialize)]
pub struct AccountQuery {
    token: Option<String>,
    /// Comma-separated `SYMBOL:price_scaled` marks for unrealized P&L.
    marks: Option<String>,
    /// Pagination: return records with `seq` greater than this value.
    after_seq: Option<u64>,
    limit: Option<usize>,
}

/// `GET /v1/portfolio`
pub async fn get_portfolio(
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    State(state): State<AppState>,
) -> Response {
    let account = match authenticate(&state, &headers, query.token.as_ref()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };
    let marks = match parse_marks(&state, query.marks.as_deref()) {
        Ok(m) => m,
        Err(code) => return bad_request(code),
    };
    let engine = lock_engine(&state);
    let snap = match build_snapshot(
        account,
        engine.book(),
        engine.orders(),
        &state.master,
        &marks,
    ) {
        Ok(s) => s,
        Err(MoneyError::Overflow) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "type": "error", "code": "overflow" })),
            )
                .into_response();
        }
        Err(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "type": "error",
                    "code": "money_error",
                    "detail": other.to_string()
                })),
            )
                .into_response();
        }
    };
    (StatusCode::OK, Json(portfolio_json(&state, &snap))).into_response()
}

/// `GET /v1/audit`
pub async fn get_audit(
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    State(state): State<AppState>,
) -> Response {
    let account = match authenticate(&state, &headers, query.token.as_ref()) {
        Ok(a) => a,
        Err(err) => return unauthorized(err),
    };
    let limit = query.limit.unwrap_or(100).min(500);
    let after = query.after_seq.unwrap_or(0);
    let engine = lock_engine(&state);
    let rows: Vec<Value> = engine
        .audit()
        .for_account(account)
        .filter(|r| r.seq() > after)
        .take(limit)
        .map(audit_row_json)
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "account_id": account.get(),
            "records": rows,
            "next_after_seq": rows.last().and_then(|v| v["seq"].as_u64()),
        })),
    )
        .into_response()
}

/// `GET /v1/reconciliation`
pub async fn get_reconciliation(
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(err) = authenticate(&state, &headers, query.token.as_ref()) {
        return unauthorized(err);
    }
    let engine = lock_engine(&state);
    let report = engine.reconcile();
    (
        StatusCode::OK,
        Json(json!({
            "ok": report.ok,
            "mismatches": report.mismatches.iter().map(|m| json!({
                "kind": m.kind.code(),
                "order_id": m.order_id.get(),
                "detail": m.detail,
            })).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

/// `GET /v1/metrics` — coarse counters (no auth; local ops only).
pub async fn get_metrics(State(state): State<AppState>) -> Json<Value> {
    Json(state.metrics.snapshot())
}

fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    token: Option<&String>,
) -> Result<AccountId, shinrai_md_fanout::FanoutError> {
    let token = extract_bearer(
        headers,
        &AuthQuery {
            token: token.cloned(),
        },
    );
    let now = crate::app::unix_logical_now();
    let claims = state.auth.authenticate(token.as_deref(), now)?;
    resolve_account(state, claims.subject().as_str())
}

fn parse_marks(
    state: &AppState,
    raw: Option<&str>,
) -> Result<HashMap<InstrumentId, PriceTicks>, &'static str> {
    let mut out = HashMap::new();
    let Some(raw) = raw else {
        return Ok(out);
    };
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((symbol, scaled)) = entry.split_once(':') else {
            return Err("invalid_marks");
        };
        let scaled: i64 = scaled.trim().parse().map_err(|_| "invalid_marks")?;
        if scaled <= 0 {
            return Err("invalid_marks");
        }
        let alias = ExternalId::ticker(symbol.trim()).map_err(|_| "invalid_marks")?;
        let id = state
            .master
            .resolve_alias(&alias)
            .map_err(|_| "unknown_symbol")?;
        out.insert(id, PriceTicks::from_scaled(scaled));
    }
    Ok(out)
}

fn portfolio_json(state: &AppState, snap: &PortfolioSnapshot) -> Value {
    json!({
        "account_id": snap.account_id.get(),
        "cash": snap.cash.iter().map(|c| json!({
            "currency": c.available.currency().code().as_str(),
            "available_minor": c.available.minor_units(),
            "reserved_minor": c.reserved.minor_units(),
        })).collect::<Vec<_>>(),
        "positions": snap.positions.iter().map(|p| {
            let symbol = state.master.get(p.instrument_id)
                .map_or_else(|_| "?".into(), |i| i.symbol_display().to_owned());
            json!({
                "instrument_id": p.instrument_id.get(),
                "symbol": symbol,
                "lots": p.lots,
                "avg_cost_scaled": p.avg_cost_scaled,
                "mark_scaled": p.mark_scaled,
                "cost_basis_minor": p.cost_basis_minor,
                "market_value_minor": p.market_value_minor,
                "unrealized_pnl_minor": p.unrealized_pnl_minor,
            })
        }).collect::<Vec<_>>(),
        "total_unrealized_pnl_minor": snap.total_unrealized_pnl_minor,
    })
}

fn audit_row_json(record: &AuditRecord) -> Value {
    let mut row = json!({
        "seq": record.seq(),
        "at": record.at(),
        "kind": record.kind().name(),
        "account_id": record.account_id().map(shinrai_ledger::AccountId::get),
        "order_id": record.order_id().map(shinrai_orders::OrderId::get),
    });
    if let AuditKind::RiskRejected { code } = record.kind() {
        row["code"] = json!(code);
    }
    if let AuditKind::OrderEventApplied { status } = record.kind() {
        row["status"] = json!(status);
    }
    if let AuditKind::VenueReport { exec_type } = record.kind() {
        row["exec_type"] = json!(exec_type);
    }
    row
}

fn bad_request(code: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "type": "error", "code": code })),
    )
        .into_response()
}
