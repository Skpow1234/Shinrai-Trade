//! Chaos / durability: persist failure, ambiguous venue, rate limits, ops auth.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState, RateLimiter};
use shinrai_store::{connect_from_env, migrate, StoreError};
use tokio::sync::Mutex;
use tower::ServiceExt;

static DB_LOCK: Mutex<()> = Mutex::const_new(());

fn database_configured() -> bool {
    std::env::var("SHINRAI_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .is_some_and(|u| !u.trim().is_empty())
}

#[tokio::test]
async fn persist_failure_returns_503_and_engages_kill() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run chaos persist test");
        return;
    }
    let _guard = DB_LOCK.lock().await;

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let mut state = AppState::for_test_with_store("chaos-tok", "trader", 901, 10_000, pool).await;
    state.close_store_for_test().await;
    let app = router(state);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=chaos-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "chaos-persist-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 1,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["code"], "persist_failed");

    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    let mbody = axum::body::to_bytes(metrics.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let mjson: serde_json::Value = serde_json::from_slice(&mbody).expect("json");
    assert_eq!(mjson["persist_degraded"], true);
    assert!(mjson["dual_write_failures"].as_u64().unwrap_or(0) >= 1);

    let risk = app
        .oneshot(
            Request::builder()
                .uri("/v1/ops/risk")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("risk");
    let rbody = axum::body::to_bytes(risk.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let rjson: serde_json::Value = serde_json::from_slice(&rbody).expect("json");
    assert_eq!(rjson["global_kill"], true);
}

#[tokio::test]
async fn venue_disconnect_leaves_pending_and_blocks_new() {
    let state = AppState::for_test("ambig-tok", "trader", 1, 10_000);
    state.disconnect_venue_for_test();
    let app = router(state);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=ambig-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "ambig-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 1,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let stuck = app
        .oneshot(
            Request::builder()
                .uri("/v1/ops/stuck-orders?max_age_secs=0")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("stuck");
    assert_eq!(stuck.status(), StatusCode::OK);
    let body = axum::body::to_bytes(stuck.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(
        json["stuck_count"].as_u64().unwrap_or(0) >= 1,
        "expected PendingNew after ambiguous disconnect: {json}"
    );
}

#[tokio::test]
async fn rate_limit_rejects_burst() {
    let state = AppState::for_test("rl-tok", "trader", 1, 10_000)
        .with_rate_limiter(RateLimiter::new(1, Duration::from_secs(60)));
    let app = router(state);

    let mk = |clid: &str| {
        Request::builder()
            .method("POST")
            .uri("/v1/orders?token=rl-tok")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "client_order_id": clid,
                    "symbol": "AAPL",
                    "side": "Buy",
                    "qty": 1,
                    "price": 10000
                })
                .to_string(),
            ))
            .expect("req")
    };

    let first = app.clone().oneshot(mk("rl-1")).await.expect("1");
    assert_eq!(first.status(), StatusCode::OK);
    let second = app.oneshot(mk("rl-2")).await.expect("2");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn ops_token_gates_metrics() {
    let app = router(AppState::for_test("x", "trader", 1, 100).with_ops_token("secret-ops"));
    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("denied");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let ok = app
        .oneshot(
            Request::builder()
                .uri("/v1/metrics?ops_token=secret-ops")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ok");
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn ops_risk_kill_switch() {
    let app = router(AppState::for_test("risk-tok", "trader", 1, 10_000));
    let patch = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ops/risk")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "global_kill": true }).to_string()))
                .expect("req"),
        )
        .await
        .expect("patch");
    assert_eq!(patch.status(), StatusCode::OK);

    let order = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=risk-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "killed-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 1,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");
    assert_eq!(order.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(order.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["code"], "kill_switch");
}
