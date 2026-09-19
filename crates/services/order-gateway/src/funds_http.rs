//! Phase 5 paper fund movements (deposit / withdraw / balances).
//!
//! These endpoints move **paper ledger balances only**. They do not custody
//! real customer money, bank rails, or broker funding. Production custody
//! requires regulatory classification and licensed partners.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use shinrai_md_fanout::Authenticator;
use shinrai_money::{Currency, Money};
use shinrai_paper::PaperError;

use crate::app::{extract_bearer, lock_engine, resolve_account, unauthorized, AppState, AuthQuery};

#[derive(Debug, Deserialize)]
pub struct FundsBody {
    /// Idempotency key (required).
    idempotency_key: String,
    /// Amount in currency minor units (e.g. USD cents).
    amount_minor: i128,
    /// ISO-like currency (default USD).
    #[serde(default)]
    currency: Option<String>,
}

/// `GET /v1/accounts/balances` — available / reserved cash for the subject account.
pub async fn get_balances(
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
    let ccy = Currency::usd();
    let available = engine.book().available(account, ccy);
    let reserved = engine.book().reserved(account, ccy);
    Json(json!({
        "mode": "paper",
        "account_id": account.get(),
        "balances": [{
            "currency": ccy.code().as_str(),
            "available_minor": available.minor_units(),
            "reserved_minor": reserved.minor_units(),
            "total_minor": available.minor_units().saturating_add(reserved.minor_units()),
        }],
    }))
    .into_response()
}

/// `POST /v1/accounts/deposit` — paper cash deposit (double-entry, idempotent).
pub async fn post_deposit(
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<FundsBody>,
) -> Response {
    if !state.funds_enabled {
        return funds_disabled();
    }
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
    let kyc = state.kyc.status(claims.subject().as_str());
    if !kyc.allows_trading() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "type": "error",
                "code": "kyc_required",
                "kyc_status": kyc.code(),
            })),
        )
            .into_response();
    }
    let amount = match parse_amount(&body) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let key = body.idempotency_key.trim();
    if key.is_empty() {
        return bad_request("missing_idempotency_key");
    }
    let key = format!("deposit:{key}");

    let snapshot = state.snapshot_engine();
    {
        let mut engine = lock_engine(&state);
        engine.set_logical_now(now);
        engine.set_correlation_id(Some(key.clone()));
        if let Err(err) = engine.deposit(account, amount, key.clone()) {
            return funds_error(err);
        }
    }
    if let Err(err) = state.must_persist(None).await {
        state.rollback_engine(snapshot);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "type": "error",
                "code": "persist_failed",
                "message": err.to_string(),
            })),
        )
            .into_response();
    }

    let engine = lock_engine(&state);
    let available = engine.book().available(account, amount.currency());
    Json(json!({
        "mode": "paper",
        "type": "deposit",
        "status": "posted",
        "account_id": account.get(),
        "currency": amount.currency().code().as_str(),
        "amount_minor": amount.minor_units(),
        "idempotency_key": body.idempotency_key.trim(),
        "available_minor": available.minor_units(),
        "note": "Paper ledger only — not a bank or broker funding rail.",
    }))
    .into_response()
}

/// `POST /v1/accounts/withdraw` — paper cash withdrawal (available only, idempotent).
pub async fn post_withdraw(
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    State(state): State<AppState>,
    Json(body): Json<FundsBody>,
) -> Response {
    if !state.funds_enabled {
        return funds_disabled();
    }
    if !state.withdrawals_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "type": "error",
                "code": "withdrawals_disabled",
                "mode": "paper",
            })),
        )
            .into_response();
    }
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
    let kyc = state.kyc.status(claims.subject().as_str());
    if !kyc.allows_trading() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "type": "error",
                "code": "kyc_required",
                "kyc_status": kyc.code(),
            })),
        )
            .into_response();
    }
    let amount = match parse_amount(&body) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let key = body.idempotency_key.trim();
    if key.is_empty() {
        return bad_request("missing_idempotency_key");
    }
    let key = format!("withdraw:{key}");

    // Optional step-up for large paper withdrawals (admin override bearer).
    if let Some(threshold) = state.withdraw_approval_threshold_minor {
        if amount.minor_units() >= threshold {
            let override_ok = state
                .admin_override_token
                .as_deref()
                .is_some_and(|expected| {
                    headers
                        .get("x-admin-override")
                        .and_then(|v| v.to_str().ok())
                        == Some(expected)
                });
            if !override_ok {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({
                        "type": "error",
                        "code": "withdraw_approval_required",
                        "threshold_minor": threshold,
                        "mode": "paper",
                        "hint": "Send X-Admin-Override for paper withdrawals at/above the threshold.",
                    })),
                )
                    .into_response();
            }
        }
    }

    let snapshot = state.snapshot_engine();
    {
        let mut engine = lock_engine(&state);
        engine.set_logical_now(now);
        engine.set_correlation_id(Some(key.clone()));
        if let Err(err) = engine.withdraw(account, amount, key.clone()) {
            return funds_error(err);
        }
    }
    if let Err(err) = state.must_persist(None).await {
        state.rollback_engine(snapshot);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "type": "error",
                "code": "persist_failed",
                "message": err.to_string(),
            })),
        )
            .into_response();
    }

    let engine = lock_engine(&state);
    let available = engine.book().available(account, amount.currency());
    Json(json!({
        "mode": "paper",
        "type": "withdraw",
        "status": "posted",
        "account_id": account.get(),
        "currency": amount.currency().code().as_str(),
        "amount_minor": amount.minor_units(),
        "idempotency_key": body.idempotency_key.trim(),
        "available_minor": available.minor_units(),
        "note": "Paper ledger only — not a bank payout.",
    }))
    .into_response()
}

fn parse_amount(body: &FundsBody) -> Result<Money, Response> {
    if body.amount_minor <= 0 {
        return Err(bad_request("invalid_amount"));
    }
    let code = body
        .currency
        .as_deref()
        .unwrap_or("USD")
        .trim()
        .to_ascii_uppercase();
    let ccy = match code.as_str() {
        "USD" => Currency::usd(),
        "EUR" => Currency::eur(),
        "JPY" => Currency::jpy(),
        "GBP" => Currency::gbp(),
        other => {
            let Ok(code) = shinrai_money::CurrencyCode::new(other) else {
                return Err(bad_request("invalid_currency"));
            };
            Currency::from_code(code, 2).map_err(|_| bad_request("invalid_currency"))?
        }
    };
    Ok(Money::from_minor(body.amount_minor, ccy))
}

fn funds_error(err: PaperError) -> Response {
    let code = match &err {
        PaperError::Ledger(shinrai_ledger::LedgerError::InsufficientFunds) => "insufficient_funds",
        PaperError::Ledger(shinrai_ledger::LedgerError::ZeroAmount) => "invalid_amount",
        _ => "funds_failed",
    };
    let status = if code == "insufficient_funds" {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(json!({
            "type": "error",
            "code": code,
            "mode": "paper",
            "message": err.to_string(),
        })),
    )
        .into_response()
}

fn funds_disabled() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "type": "error",
            "code": "funds_disabled",
            "mode": "paper",
            "hint": "Set SHINRAI_OG_FUNDS_ENABLED=1 to enable paper deposits/withdrawals.",
        })),
    )
        .into_response()
}

fn bad_request(code: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "type": "error", "code": code, "mode": "paper" })),
    )
        .into_response()
}
