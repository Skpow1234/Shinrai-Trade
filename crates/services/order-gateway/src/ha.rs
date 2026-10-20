//! Active-passive OMS leader election (Postgres lease).

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use shinrai_store::{heartbeat_oms_leader, StorePool};

use crate::app::AppState;

/// Rejects `/v1/*` (and other non-health routes) when HA is on and this
/// instance does not hold the OMS leader lease.
pub async fn require_oms_leader(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if !state.ha_enabled() || req.uri().path() == "/health" {
        return next.run(req).await;
    }
    if state.is_oms_leader() {
        return next.run(req).await;
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "2")],
        Json(json!({
            "type": "error",
            "code": "not_oms_leader",
            "instance_id": state.instance_id(),
            "message": "this order-gateway instance is not the OMS leader",
        })),
    )
        .into_response()
}

/// Renews / acquires the OMS leader lease until the process exits.
pub async fn run_leader_heartbeat(
    pool: StorePool,
    state: AppState,
    ttl: Duration,
    interval: Duration,
) {
    let instance = state.instance_id().to_owned();
    loop {
        match heartbeat_oms_leader(&pool, &instance, ttl).await {
            Ok(is_leader) => {
                let was = state.is_oms_leader_flag();
                state.set_oms_leader(is_leader);
                if is_leader && !was {
                    eprintln!(
                        "shinrai-order-gateway: acquired OMS leader lease (instance={instance})"
                    );
                } else if !is_leader && was {
                    eprintln!("shinrai-order-gateway: lost OMS leader lease (instance={instance})");
                }
            }
            Err(err) => {
                state.set_oms_leader(false);
                eprintln!("shinrai-order-gateway: OMS leader heartbeat error: {err}");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

impl AppState {
    pub(crate) fn is_oms_leader_flag(&self) -> bool {
        self.oms_leader.load(Ordering::Relaxed)
    }
}
