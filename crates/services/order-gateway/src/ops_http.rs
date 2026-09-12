//! Ops HTTP: stuck orders JSON + simple HTML dashboard.

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{lock_engine, unix_logical_now, AppState};
use crate::ops::{find_stuck_orders, DEFAULT_STUCK_AGE_SECS};

#[derive(Debug, Deserialize)]
pub struct StuckQuery {
    /// Override stuck age threshold (logical seconds).
    max_age_secs: Option<u64>,
}

/// `GET /v1/ops/stuck-orders` — pending OMS rows older than threshold.
pub async fn get_stuck_orders(
    Query(query): Query<StuckQuery>,
    State(state): State<AppState>,
) -> Json<Value> {
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
}

/// `GET /v1/ops` — minimal local HTML dashboard (polls `/v1/metrics`).
pub async fn get_ops_dashboard() -> Response {
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
  <p class="sub" style="margin-top:1rem"><a href="/v1/metrics">/v1/metrics</a> · <a href="/v1/ops/stuck-orders">/v1/ops/stuck-orders</a></p>
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
