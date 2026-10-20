//! Durable risk control-plane state (kills, limits, restrictions).

use serde_json::{json, Value};
use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;
use shinrai_risk::{RiskLimits, RiskStateSnapshot};
use sqlx::PgPool;

use crate::error::StoreError;

/// Upserts the singleton risk control-plane row.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn upsert_risk_state(pool: &PgPool, snap: &RiskStateSnapshot) -> Result<(), StoreError> {
    let account_kills: Vec<i64> = snap
        .account_kills
        .iter()
        .map(|a| i64::try_from(a.get()).unwrap_or(i64::MAX))
        .collect();
    let restricted: Vec<i64> = snap
        .restricted
        .iter()
        .map(|i| i64::try_from(i.get()).unwrap_or(i64::MAX))
        .collect();
    sqlx::query(
        r"
        INSERT INTO risk_state (id, global_kill, limits_json, account_kills, restricted_instruments, updated_at)
        VALUES (1, $1, $2, $3, $4, NOW())
        ON CONFLICT (id) DO UPDATE SET
            global_kill = EXCLUDED.global_kill,
            limits_json = EXCLUDED.limits_json,
            account_kills = EXCLUDED.account_kills,
            restricted_instruments = EXCLUDED.restricted_instruments,
            updated_at = NOW()
        ",
    )
    .bind(snap.global_kill)
    .bind(limits_to_json(snap.limits))
    .bind(&account_kills)
    .bind(&restricted)
    .execute(pool)
    .await?;
    Ok(())
}

/// Loads the risk control-plane row when present.
///
/// # Errors
///
/// Returns sqlx / decode errors.
pub async fn load_risk_state(pool: &PgPool) -> Result<Option<RiskStateSnapshot>, StoreError> {
    let row: Option<(bool, Value, Vec<i64>, Vec<i64>)> = sqlx::query_as(
        r"
        SELECT global_kill, limits_json, account_kills, restricted_instruments
        FROM risk_state
        WHERE id = 1
        ",
    )
    .fetch_optional(pool)
    .await?;
    let Some((global_kill, limits_json, account_kills, restricted)) = row else {
        return Ok(None);
    };
    Ok(Some(RiskStateSnapshot {
        limits: limits_from_json(&limits_json)?,
        global_kill,
        account_kills: account_kills
            .into_iter()
            .map(|id| AccountId::from_u64(u64::try_from(id).unwrap_or(0)))
            .collect(),
        restricted: restricted
            .into_iter()
            .map(|id| InstrumentId::from_u64(u64::try_from(id).unwrap_or(0)))
            .collect(),
    }))
}

fn limits_to_json(limits: RiskLimits) -> Value {
    json!({
        "max_order_qty_lots": limits.max_order_qty_lots,
        "max_order_notional_minor": limits.max_order_notional_minor.to_string(),
        "max_position_lots": limits.max_position_lots,
        "collar_bps": limits.collar_bps,
        "market_session_utc": limits.market_session_utc,
        "max_daily_loss_minor": limits.max_daily_loss_minor.to_string(),
        "allow_short": limits.allow_short,
        "max_short_lots": limits.max_short_lots,
        "max_asset_class_notional_minor": limits.max_asset_class_notional_minor.to_string(),
    })
}

fn limits_from_json(v: &Value) -> Result<RiskLimits, StoreError> {
    let max_order_qty_lots = v
        .get("max_order_qty_lots")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("max_order_qty_lots"))?;
    let max_order_notional_minor = parse_i128(v, "max_order_notional_minor")?;
    let max_position_lots = v
        .get("max_position_lots")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("max_position_lots"))?;
    let collar_bps = v
        .get("collar_bps")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("collar_bps"))?;
    let market_session_utc = match v.get("market_session_utc") {
        None | Some(Value::Null) => None,
        Some(Value::Array(arr)) if arr.len() == 2 => {
            let open = arr[0]
                .as_u64()
                .ok_or_else(|| invalid("market_session_utc"))?;
            let close = arr[1]
                .as_u64()
                .ok_or_else(|| invalid("market_session_utc"))?;
            Some((
                u32::try_from(open).unwrap_or(u32::MAX),
                u32::try_from(close).unwrap_or(u32::MAX),
            ))
        }
        _ => return Err(invalid("market_session_utc")),
    };
    let max_daily_loss_minor = parse_i128(v, "max_daily_loss_minor")?;
    let allow_short = v
        .get("allow_short")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("allow_short"))?;
    let max_short_lots = v
        .get("max_short_lots")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("max_short_lots"))?;
    let max_asset_class_notional_minor = parse_i128(v, "max_asset_class_notional_minor")?;
    Ok(RiskLimits {
        max_order_qty_lots,
        max_order_notional_minor,
        max_position_lots,
        collar_bps,
        market_session_utc,
        max_daily_loss_minor,
        allow_short,
        max_short_lots,
        max_asset_class_notional_minor,
    })
}

fn parse_i128(v: &Value, field: &'static str) -> Result<i128, StoreError> {
    match v.get(field) {
        Some(Value::Number(n)) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .ok_or_else(|| invalid(field)),
        Some(Value::String(s)) => s.parse().map_err(|_| StoreError::InvalidInteger {
            field,
            value: s.clone(),
        }),
        _ => Err(invalid(field)),
    }
}

fn invalid(field: &'static str) -> StoreError {
    StoreError::InvalidStored {
        field,
        value: "risk_state.limits_json".into(),
    }
}
