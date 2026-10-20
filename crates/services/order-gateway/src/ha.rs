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
///
/// On follower → leader promote, rehydrates from Postgres before serving traffic.
/// Followers warm-rehydrate on each tick so failover stays fast.
pub async fn run_leader_heartbeat(
    pool: StorePool,
    state: AppState,
    ttl: Duration,
    interval: Duration,
) {
    let instance = state.instance_id().to_owned();
    loop {
        match heartbeat_oms_leader(&pool, &instance, ttl).await {
            Ok(true) => {
                let was = state.is_oms_leader_flag();
                if was {
                    state.set_oms_leader(true);
                } else if promote_oms_leader(&pool, &state, ttl).await {
                    eprintln!(
                        "shinrai-order-gateway: acquired OMS leader lease after hydrate (instance={instance})"
                    );
                } else {
                    eprintln!(
                        "shinrai-order-gateway: promote incomplete; staying follower (instance={instance})"
                    );
                }
            }
            Ok(false) => {
                let was = state.is_oms_leader_flag();
                state.set_oms_leader(false);
                state.set_oms_promoting(false);
                if was {
                    eprintln!(
                        "shinrai-order-gateway: lost OMS leader lease (instance={instance})"
                    );
                }
                // Warm standby: keep in-memory OMS aligned with shared Postgres.
                if let Err(err) = state.rehydrate_from_store().await {
                    eprintln!(
                        "shinrai-order-gateway: follower warm hydrate failed (instance={instance}): {err}"
                    );
                }
            }
            Err(err) => {
                state.set_oms_leader(false);
                state.set_oms_promoting(false);
                eprintln!("shinrai-order-gateway: OMS leader heartbeat error: {err}");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Rehydrates from the store, reconfirms the lease, then flips the leader flag.
///
/// Returns `true` when this instance may serve OMS traffic.
pub async fn promote_oms_leader(
    pool: &StorePool,
    state: &AppState,
    ttl: Duration,
) -> bool {
    let instance = state.instance_id();
    state.set_oms_promoting(true);
    state.set_oms_leader(false);

    let outcome = async {
        state.rehydrate_from_store().await?;
        // Hydrate may outlast a lease tick; renew before serving.
        heartbeat_oms_leader(pool, instance, ttl).await
    }
    .await;

    state.set_oms_promoting(false);
    match outcome {
        Ok(true) => {
            state.set_oms_leader(true);
            true
        }
        Ok(false) => {
            state.set_oms_leader(false);
            eprintln!(
                "shinrai-order-gateway: lost OMS lease during promote hydrate (instance={instance})"
            );
            false
        }
        Err(err) => {
            state.set_oms_leader(false);
            eprintln!(
                "shinrai-order-gateway: promote hydrate failed (instance={instance}): {err}"
            );
            false
        }
    }
}

impl AppState {
    pub(crate) fn is_oms_leader_flag(&self) -> bool {
        self.oms_leader.load(Ordering::Relaxed)
    }
}
