//! Ops HTTP: stuck orders, risk control plane, and simple HTML dashboard.

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_instruments::ExternalId;
use shinrai_ledger::AccountId;
use shinrai_risk::RiskLimits;

use crate::app::{lock_engine, require_ops_auth, unix_logical_now, AppState};
use crate::ops::{find_stuck_orders, DEFAULT_STUCK_AGE_SECS};

#[derive(Debug, Deserialize)]
pub struct StuckQuery {
    /// Override stuck age threshold (logical seconds).
    max_age_secs: Option<u64>,
    /// Ops bearer when `SHINRAI_OG_OPS_TOKEN` is configured.
    ops_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpsAuthQuery {
    /// Ops bearer when `SHINRAI_OG_OPS_TOKEN` is configured.
    ops_token: Option<String>,
}

/// `GET /v1/ops/stuck-orders` — pending OMS rows older than threshold.
pub async fn get_stuck_orders(
    headers: HeaderMap,
    Query(query): Query<StuckQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let now = unix_logical_now();
    let max_age = query.max_age_secs.unwrap_or(state.stuck_age_secs);
    let engine = lock_engine(&state);
    let stuck = find_stuck_orders(&engine, now, max_age);
    Json(json!({
        "now": now,
        "max_age_secs": max_age,
        "stuck_count": stuck.len(),
        "stuck_orders": stuck.iter().map(|s| json!({
            "order_id": s.order_id.get(),
            "status": s.status.to_string(),
            "age_secs": s.age_secs,
            "last_at": s.last_at,
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// `GET /v1/ops/risk` — current limits + kill switches.
pub async fn get_risk(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let engine = lock_engine(&state);
    let limits = engine.risk().limits();
    Json(json!({
        "global_kill": engine.risk().global_kill(),
        "limits": limits_json(limits),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct RiskPatchBody {
    /// Enable/disable global kill switch.
    global_kill: Option<bool>,
    /// Replace static limits (partial fields allowed).
    limits: Option<RiskLimitsPatch>,
    /// Restrict trading for a symbol.
    restrict_symbol: Option<String>,
    /// Clear restriction for a symbol.
    allow_symbol: Option<String>,
    /// Per-account kill switch.
    account_kill: Option<AccountKillPatch>,
}

#[derive(Debug, Deserialize)]
pub struct AccountKillPatch {
    account_id: u64,
    on: bool,
}

#[derive(Debug, Deserialize)]
pub struct RiskLimitsPatch {
    max_order_qty_lots: Option<i64>,
    max_order_notional_minor: Option<i64>,
    max_position_lots: Option<i64>,
    collar_bps: Option<i64>,
    market_session_utc: Option<(u32, u32)>,
    clear_market_session: Option<bool>,
    max_daily_loss_minor: Option<i64>,
    allow_short: Option<bool>,
    max_short_lots: Option<i64>,
    max_asset_class_notional_minor: Option<i64>,
}

/// `POST /v1/ops/risk` — mutate kill switches / limits (ops control plane).
pub async fn post_risk(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<RiskPatchBody>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let mut engine = lock_engine(&state);
    if let Some(on) = body.global_kill {
        engine.risk_mut().set_global_kill(on);
    }
    if let Some(patch) = body.limits {
        let mut limits = engine.risk().limits();
        apply_limits_patch(&mut limits, &patch);
        engine.risk_mut().set_limits(limits);
    }
    if let Some(symbol) = body.restrict_symbol.as_deref() {
        match resolve_symbol_id(&state, symbol) {
            Ok(id) => engine.risk_mut().restrict_instrument(id),
            Err(code) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "type": "error", "code": code })),
                )
                    .into_response();
            }
        }
    }
    if let Some(symbol) = body.allow_symbol.as_deref() {
        match resolve_symbol_id(&state, symbol) {
            Ok(id) => engine.risk_mut().allow_instrument(id),
            Err(code) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "type": "error", "code": code })),
                )
                    .into_response();
            }
        }
    }
    if let Some(ak) = body.account_kill {
        engine
            .risk_mut()
            .set_account_kill(AccountId::from_u64(ak.account_id), ak.on);
    }
    let limits = engine.risk().limits();
    Json(json!({
        "ok": true,
        "global_kill": engine.risk().global_kill(),
        "limits": limits_json(limits),
    }))
    .into_response()
}

fn apply_limits_patch(limits: &mut RiskLimits, patch: &RiskLimitsPatch) {
    if let Some(v) = patch.max_order_qty_lots {
        limits.max_order_qty_lots = v;
    }
    if let Some(v) = patch.max_order_notional_minor {
        limits.max_order_notional_minor = i128::from(v);
    }
    if let Some(v) = patch.max_position_lots {
        limits.max_position_lots = v;
    }
    if let Some(v) = patch.collar_bps {
        limits.collar_bps = v;
    }
    if patch.clear_market_session == Some(true) {
        limits.market_session_utc = None;
    } else if let Some(session) = patch.market_session_utc {
        limits.market_session_utc = Some(session);
    }
    if let Some(v) = patch.max_daily_loss_minor {
        limits.max_daily_loss_minor = i128::from(v);
    }
    if let Some(v) = patch.allow_short {
        limits.allow_short = v;
    }
    if let Some(v) = patch.max_short_lots {
        limits.max_short_lots = v;
    }
    if let Some(v) = patch.max_asset_class_notional_minor {
        limits.max_asset_class_notional_minor = i128::from(v);
    }
}

fn limits_json(limits: RiskLimits) -> Value {
    json!({
        "max_order_qty_lots": limits.max_order_qty_lots,
        "max_order_notional_minor": i64::try_from(limits.max_order_notional_minor).unwrap_or(i64::MAX),
        "max_position_lots": limits.max_position_lots,
        "collar_bps": limits.collar_bps,
        "market_session_utc": limits.market_session_utc,
        "max_daily_loss_minor": i64::try_from(limits.max_daily_loss_minor).unwrap_or(i64::MAX),
        "allow_short": limits.allow_short,
        "max_short_lots": limits.max_short_lots,
        "max_asset_class_notional_minor": i64::try_from(limits.max_asset_class_notional_minor).unwrap_or(i64::MAX),
    })
}

fn resolve_symbol_id(
    state: &AppState,
    symbol: &str,
) -> Result<shinrai_instruments::InstrumentId, &'static str> {
    let alias = ExternalId::ticker(symbol.trim()).map_err(|_| "invalid_symbol")?;
    state
        .master
        .resolve_alias(&alias)
        .map_err(|_| "unknown_symbol")
}

/// `GET /v1/ops` — minimal local HTML dashboard (polls `/v1/metrics`).
#[allow(clippy::too_many_lines)]
pub async fn get_ops_dashboard(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Shinrai order-gateway ops</title>
<style>
  :root {{
    --bg: #0f1419;
    --panel: #1a222c;
    --text: #e7ecf1;
    --muted: #8b9aab;
    --ok: #3d9a6a;
    --warn: #c9852c;
    --bad: #c44b4b;
    --accent: #5b8def;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; padding: 1.5rem;
    font-family: "IBM Plex Mono", "Consolas", monospace;
    background: radial-gradient(1200px 600px at 10% -10%, #1c2a3a, var(--bg));
    color: var(--text);
  }}
  h1 {{ font-size: 1.1rem; font-weight: 600; letter-spacing: 0.04em; margin: 0 0 0.25rem; }}
  .sub {{ color: var(--muted); font-size: 0.8rem; margin-bottom: 1.25rem; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 0.75rem; }}
  .card {{
    background: var(--panel); border: 1px solid #2a3542; border-radius: 6px;
    padding: 0.85rem 1rem;
  }}
  .card .label {{ color: var(--muted); font-size: 0.7rem; text-transform: uppercase; }}
  .card .value {{ font-size: 1.4rem; margin-top: 0.25rem; }}
  .ok {{ color: var(--ok); }} .warn {{ color: var(--warn); }} .bad {{ color: var(--bad); }}
  table {{ width: 100%; border-collapse: collapse; margin-top: 1rem; font-size: 0.85rem; }}
  th, td {{ text-align: left; padding: 0.4rem 0.5rem; border-bottom: 1px solid #2a3542; }}
  th {{ color: var(--muted); font-weight: 500; }}
  a {{ color: var(--accent); }}
  #err {{ color: var(--bad); margin-top: 1rem; white-space: pre-wrap; }}
</style>
</head>
<body>
  <h1>Shinrai · order-gateway</h1>
  <p class="sub">Local ops view · polls <code>/v1/metrics</code> every 2s · default stuck age {DEFAULT_STUCK_AGE_SECS}s</p>
  <div class="grid" id="cards"></div>
  <h2 style="font-size:0.9rem;margin:1.5rem 0 0.5rem;color:var(--muted)">Stuck pending</h2>
  <table>
    <thead><tr><th>order_id</th><th>status</th><th>age_secs</th><th>last_at</th></tr></thead>
    <tbody id="stuck"></tbody>
  </table>
  <p class="sub" style="margin-top:1rem"><a href="/v1/metrics">/v1/metrics</a> · <a href="/v1/ops/stuck-orders">/v1/ops/stuck-orders</a> · <a href="/v1/ops/risk">/v1/ops/risk</a></p>
  <div id="err"></div>
<script>
async function refresh() {{
  const err = document.getElementById('err');
  try {{
    const r = await fetch('/v1/metrics');
    const m = await r.json();
    err.textContent = '';
    const cards = [
      ['submitted', m.orders_submitted, ''],
      ['accepted', m.orders_accepted, 'ok'],
      ['risk reject', m.orders_risk_rejected, m.orders_risk_rejected > 0 ? 'warn' : ''],
      ['canceled', m.orders_canceled, ''],
      ['dual-write fail', m.dual_write_failures, m.dual_write_failures > 0 ? 'bad' : ''],
      ['OMS total', m.orders_total, ''],
      ['pending', m.orders_pending, m.orders_pending > 0 ? 'warn' : ''],
      ['stuck', m.stuck_count, m.stuck_count > 0 ? 'bad' : 'ok'],
      ['recon', m.reconciliation_ok ? 'ok' : 'BREAK', m.reconciliation_ok ? 'ok' : 'bad'],
      ['trial bal', m.ledger_trial_balance_ok ? 'ok' : 'FAIL', m.ledger_trial_balance_ok ? 'ok' : 'bad'],
      ['store', m.store_enabled ? 'on' : 'off', ''],
    ];
    document.getElementById('cards').innerHTML = cards.map(([l,v,c]) =>
      `<div class="card"><div class="label">${{l}}</div><div class="value ${{c}}">${{v}}</div></div>`
    ).join('');
    const rows = (m.stuck_orders || []).map(s =>
      `<tr><td>${{s.order_id}}</td><td>${{s.status}}</td><td>${{s.age_secs}}</td><td>${{s.last_at}}</td></tr>`
    ).join('') || '<tr><td colspan="4" style="color:var(--muted)">none</td></tr>';
    document.getElementById('stuck').innerHTML = rows;
  }} catch (e) {{
    err.textContent = String(e);
  }}
}}
refresh();
setInterval(refresh, 2000);
</script>
</body>
</html>"#
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(html),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct EodCashRow {
    account_id: u64,
    currency: String,
    minor_units: i128,
}

#[derive(Debug, Deserialize)]
pub struct EodPositionRow {
    account_id: u64,
    symbol: String,
    lots: i64,
}

#[derive(Debug, Deserialize)]
pub struct EodFillRow {
    order_id: u64,
    exec_id: String,
    qty: i64,
    price_ticks: i64,
}

#[derive(Debug, Deserialize)]
pub struct EodBody {
    cash: Option<Vec<EodCashRow>>,
    positions: Option<Vec<EodPositionRow>>,
    fills: Option<Vec<EodFillRow>>,
}

/// `POST /v1/ops/reconciliation/eod` — compare internal state to a broker EOD snapshot.
pub async fn post_eod_reconciliation(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<EodBody>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }

    let mut snapshot = shinrai_paper::BrokerEodSnapshot::default();
    for row in body.cash.unwrap_or_default() {
        let currency = match row.currency.trim().to_ascii_uppercase().as_str() {
            "USD" => shinrai_money::Currency::usd(),
            "EUR" => shinrai_money::Currency::eur(),
            "JPY" => shinrai_money::Currency::jpy(),
            "GBP" => shinrai_money::Currency::gbp(),
            other => {
                let Ok(code) = shinrai_money::CurrencyCode::new(other) else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "type": "error", "code": "invalid_currency" })),
                    )
                        .into_response();
                };
                match shinrai_money::Currency::from_code(code, 2) {
                    Ok(c) => c,
                    Err(_) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "type": "error", "code": "invalid_currency" })),
                        )
                            .into_response();
                    }
                }
            }
        };
        snapshot.cash.push(shinrai_paper::BrokerEodCash {
            account_id: AccountId::from_u64(row.account_id),
            currency,
            minor_units: row.minor_units,
        });
    }
    for row in body.positions.unwrap_or_default() {
        let Ok(alias) = ExternalId::ticker(row.symbol.trim()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "type": "error", "code": "invalid_symbol" })),
            )
                .into_response();
        };
        let Ok(instrument_id) = state.master.resolve_alias(&alias) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "type": "error", "code": "unknown_symbol" })),
            )
                .into_response();
        };
        snapshot.positions.push(shinrai_paper::BrokerEodPosition {
            account_id: AccountId::from_u64(row.account_id),
            instrument_id,
            lots: row.lots,
        });
    }
    for row in body.fills.unwrap_or_default() {
        let Ok(exec_id) = shinrai_orders::ExecId::new(row.exec_id.trim()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "type": "error", "code": "invalid_exec_id" })),
            )
                .into_response();
        };
        snapshot.fills.push(shinrai_paper::BrokerEodFill {
            order_id: shinrai_orders::OrderId::from_u64(row.order_id),
            exec_id,
            qty: row.qty,
            price_ticks: row.price_ticks,
        });
    }

    {
        let mut guard = state
            .eod_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(snapshot.clone());
    }

    let engine = lock_engine(&state);
    let report = engine.reconcile_eod(&snapshot);
    Json(json!({
        "ok": report.ok,
        "mismatches": report.mismatches.iter().map(|m| json!({
            "kind": m.kind.code(),
            "order_id": m.order_id.get(),
            "detail": m.detail,
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ApprovalBody {
    account_id: u64,
    symbol: String,
    /// Ops actor identity for maker/checker (header `X-Ops-Actor` also accepted).
    #[serde(default)]
    actor: Option<String>,
    /// When set, completes second control for this request id.
    #[serde(default)]
    approve_id: Option<u64>,
}

/// `POST /v1/ops/approvals` — dual-control restricted-instrument override.
///
/// Without `approve_id`: creates a pending request (maker).
/// With `approve_id`: checker approves (must differ from maker), then grants risk override.
pub async fn post_approvals(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<ApprovalBody>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let actor = body
        .actor
        .clone()
        .or_else(|| {
            headers
                .get("x-ops-actor")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "ops".into());

    if let Some(id) = body.approve_id {
        let approved = {
            let mut store = state
                .approvals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match store.approve(id, &actor) {
                Ok(r) => r,
                Err(code) => {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({ "type": "error", "code": code })),
                    )
                        .into_response();
                }
            }
        };
        {
            let mut engine = lock_engine(&state);
            engine
                .risk_mut()
                .grant_override(approved.account_id, approved.instrument_id);
        }
        return Json(json!({
            "ok": true,
            "status": "approved",
            "id": approved.id,
            "account_id": approved.account_id.get(),
            "symbol": approved.symbol,
            "requested_by": approved.requested_by,
            "approved_by": approved.approved_by,
        }))
        .into_response();
    }

    let Ok(alias) = ExternalId::ticker(body.symbol.trim()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "type": "error", "code": "invalid_symbol" })),
        )
            .into_response();
    };
    let Ok(instrument_id) = state.master.resolve_alias(&alias) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "type": "error", "code": "unknown_symbol" })),
        )
            .into_response();
    };
    let account = AccountId::from_u64(body.account_id);
    let req = {
        let mut store = state
            .approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        store.request(account, instrument_id, body.symbol.trim(), actor)
    };
    Json(json!({
        "ok": true,
        "status": "pending",
        "id": req.id,
        "account_id": req.account_id.get(),
        "symbol": req.symbol,
        "requested_by": req.requested_by,
    }))
    .into_response()
}

/// `GET /v1/ops/approvals` — list pending dual-control requests.
pub async fn get_approvals(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let pending = state
        .approvals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pending();
    Json(json!({
        "pending": pending.iter().map(|r| json!({
            "id": r.id,
            "account_id": r.account_id.get(),
            "symbol": r.symbol,
            "instrument_id": r.instrument_id.get(),
            "requested_by": r.requested_by,
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// `GET /v1/ops/audit/export` — JSON audit export for compliance.
pub async fn get_audit_export(
    headers: HeaderMap,
    Query(query): Query<OpsAuthQuery>,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = require_ops_auth(&state, &headers, query.ops_token.as_deref(), None) {
        return resp;
    }
    let engine = lock_engine(&state);
    let records: Vec<_> = engine
        .audit()
        .records()
        .map(|r| {
            json!({
                "seq": r.seq(),
                "at": r.at(),
                "kind": format!("{:?}", r.kind()),
                "account_id": r.account_id().map(AccountId::get),
                "order_id": r.order_id().map(|o| o.get()),
                "correlation_id": r.correlation_id(),
                "content_hash": r.content_hash(),
                "previous_hash": r.prev_hash(),
            })
        })
        .collect();
    let durable: Vec<_> = engine
        .durable_trade_execs()
        .iter()
        .map(|t| {
            json!({
                "order_id": t.order_id.get(),
                "exec_id": t.exec_id.as_str(),
                "qty": t.qty,
                "price": t.price,
                "session": t.session.n,
                "seq": t.seq,
            })
        })
        .collect();
    Json(json!({
        "chain_ok": engine.audit_chain_ok(),
        "audit": records,
        "durable_drop_copy": durable,
    }))
    .into_response()
}
