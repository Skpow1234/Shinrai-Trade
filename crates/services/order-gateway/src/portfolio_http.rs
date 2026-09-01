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
    /// Comma-separated `SYMBOL:price_scaled` marks (override stored/live).
    marks: Option<String>,
    /// Include marks from fill prices / bootstrap (default true). Accepts `1`/`0` or `true`/`false`.
    use_stored_marks: Option<String>,
    /// Fetch last trade from MD gateway when `SHINRAI_OG_MD_URL` is set.
    use_live_marks: Option<String>,
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
    let manual = match parse_marks(&state, query.marks.as_deref()) {
        Ok(m) => m,
        Err(code) => return bad_request(code),
    };
    let position_symbols: Vec<String> = {
        let engine = lock_engine(&state);
        engine
            .book()
            .positions_for(account)
            .filter_map(|(id, lots)| {
                (lots != 0)
                    .then(|| {
                        state
                            .master
                            .get(id)
                            .ok()
                            .map(|i| i.symbol_display().to_owned())
                    })
                    .flatten()
            })
            .collect()
    };
    let use_stored = match parse_bool_query(query.use_stored_marks.as_deref(), true) {
        Ok(v) => v,
        Err(code) => return bad_request(code),
    };
    let use_live = match parse_bool_query(query.use_live_marks.as_deref(), false) {
        Ok(v) => v,
        Err(code) => return bad_request(code),
    };
    let marks = match resolve_marks(&state, use_stored, use_live, manual, &position_symbols).await {
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
        "total_cost_basis_minor": snap.total_cost_basis_minor,
        "realized_pnl_minor": snap.realized_pnl_minor,
        "total_equity_minor": snap.total_equity_minor,
    })
}

fn parse_bool_query(value: Option<&str>, default: bool) -> Result<bool, &'static str> {
    match value {
        None => Ok(default),
        Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" => Ok(default),
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" => Ok(false),
            _ => Err("invalid_bool"),
        },
    }
}

async fn resolve_marks(
    state: &AppState,
    use_stored: bool,
    use_live: bool,
    manual: HashMap<InstrumentId, PriceTicks>,
    position_symbols: &[String],
) -> Result<HashMap<InstrumentId, PriceTicks>, &'static str> {
    let mut marks = if use_stored {
        state
            .marks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .merged_with(&manual)
    } else {
        manual
    };
    if use_live {
        let Some(base) = state.md_base_url.as_deref() else {
            return Err("md_url_not_configured");
        };
        let token = state.md_token.as_deref();
        for symbol in position_symbols {
            if let Some(px) =
                crate::md_client::fetch_quote(base, token, &state.master, symbol).await
            {
                if let Some(id) = crate::md_client::instrument_for_symbol(&state.master, symbol) {
                    marks.insert(id, px);
                }
            }
        }
    }
    Ok(marks)
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
