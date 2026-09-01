//! Auth HTTP handlers: issue, refresh, revoke.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_md_fanout::{FanoutError, IssuedTokens};

use crate::app::{unix_logical_now, AppState};

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    grant_type: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    token: String,
}

/// `POST /v1/auth/token` — `client_credentials` or `refresh_token`.
pub async fn post_token(State(state): State<AppState>, Json(body): Json<TokenRequest>) -> Response {
    let now = unix_logical_now();
    let result = match body.grant_type.as_str() {
        "client_credentials" => {
            let Some(id) = body.client_id.as_deref().filter(|s| !s.is_empty()) else {
                return auth_error(StatusCode::BAD_REQUEST, "invalid_request");
            };
            let Some(secret) = body.client_secret.as_deref() else {
                return auth_error(StatusCode::BAD_REQUEST, "invalid_request");
            };
            state.auth.issue_client_credentials(id, secret, now)
        }
        "refresh_token" => {
            let Some(rt) = body.refresh_token.as_deref().filter(|s| !s.is_empty()) else {
                return auth_error(StatusCode::BAD_REQUEST, "invalid_request");
            };
            state.auth.refresh(rt, now)
        }
        _ => return auth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    };
    match result {
        Ok(issued) => (StatusCode::OK, Json(token_json(&issued, now))).into_response(),
        Err(FanoutError::InvalidCredentials) => auth_error(
            StatusCode::UNAUTHORIZED,
            FanoutError::InvalidCredentials.code(),
        ),
        Err(FanoutError::ExpiredToken) => {
            auth_error(StatusCode::UNAUTHORIZED, FanoutError::ExpiredToken.code())
        }
        Err(FanoutError::RevokedToken) => {
            auth_error(StatusCode::UNAUTHORIZED, FanoutError::RevokedToken.code())
        }
        Err(FanoutError::InvalidToken) => {
            auth_error(StatusCode::UNAUTHORIZED, FanoutError::InvalidToken.code())
        }
        Err(other) => auth_error(StatusCode::BAD_REQUEST, other.code()),
    }
}

/// `POST /v1/auth/revoke` — revoke access or refresh (family).
pub async fn post_revoke(
    State(state): State<AppState>,
    Json(body): Json<RevokeRequest>,
) -> Response {
    let token = body.token.trim();
    if token.is_empty() {
        return auth_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    state.auth.revoke(token);
    StatusCode::OK.into_response()
}

fn token_json(issued: &IssuedTokens, now: u64) -> Value {
    let expires_in = issued.access_expires_at().saturating_sub(now);
    json!({
        "token_type": "Bearer",
        "access_token": issued.access_token(),
        "expires_in": expires_in,
        "expires_at": issued.access_expires_at(),
        "refresh_token": issued.refresh_token(),
        "refresh_expires_at": issued.refresh_expires_at(),
    })
}

fn auth_error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({ "type": "error", "code": code }))).into_response()
}
