//! Dual-write integration: order gateway → `shinrai-store`.
//!
//! Skips when `SHINRAI_DATABASE_URL` / `DATABASE_URL` is unset.
//!
//! These tests share one Postgres; they serialize on an async mutex so order-id
//! `1` upserts and bootstrap ledger keys do not race across tokio tests.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_ledger::AccountId;
use shinrai_order_gateway::{router, AppState};
use shinrai_orders::ClientOrderId;
use shinrai_store::{
    connect_from_env, load_audit_after, load_order_by_client, migrate, StoreError,
};
use tokio::sync::Mutex;
use tower::ServiceExt;

/// Serializes dual-write tests that share the CI Postgres service.
static DB_LOCK: Mutex<()> = Mutex::const_new(());

fn database_configured() -> bool {
    std::env::var("SHINRAI_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .is_some_and(|u| !u.trim().is_empty())
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[tokio::test]
async fn submit_persists_order_and_audit() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run dual-write tests");
        return;
    }
    let _guard = DB_LOCK.lock().await;

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    // Distinct account avoids clobbering hydrate-test rows when suites overlap.
    let account_raw = 101_u64;
    let state =
        AppState::for_test_with_store("dw-tok", "trader", account_raw, 10_000, pool.clone()).await;
    let app = router(state);

    let clid = format!("dw-ord-{}", unique_suffix());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=dw-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": clid,
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 5,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let account = AccountId::from_u64(account_raw);
    let client = ClientOrderId::new(&clid).expect("clid");
    let stored = load_order_by_client(&pool, account, &client)
        .await
        .expect("load")
        .expect("order dual-written");
    assert_eq!(stored.cum_qty.lots(), 5);

    let audit = load_audit_after(&pool, 0, 200).await.expect("audit");
    assert!(
        !audit.is_empty(),
        "expected audit rows after bootstrap + submit"
    );
}

#[tokio::test]
async fn restart_hydrates_order_into_memory() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run dual-write tests");
        return;
    }
    let _guard = DB_LOCK.lock().await;

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let account_raw = 202_u64;
    let clid = format!("hy-ord-{}", unique_suffix());

    {
        let state =
            AppState::for_test_with_store("hy-tok", "trader", account_raw, 10_000, pool.clone())
                .await;
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/orders?token=hy-tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "client_order_id": clid,
                            "symbol": "AAPL",
                            "side": "Buy",
                            "qty": 3,
                            "price": 10000
                        })
                        .to_string(),
                    ))
                    .expect("req"),
            )
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);

        let account = AccountId::from_u64(account_raw);
        let client = ClientOrderId::new(&clid).expect("clid");
        let stored = load_order_by_client(&pool, account, &client)
            .await
            .expect("load")
            .expect("order must be dual-written before hydrate");
        assert_eq!(stored.cum_qty.lots(), 3);
    }

    // Simulate process restart: empty deposits, load from Postgres.
    let state = AppState::for_test_hydrate("hy-tok", "trader", account_raw, pool.clone()).await;
    let app = router(state);
    let list = app
        .oneshot(
            Request::builder()
                .uri("/v1/orders?token=hy-tok")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("list");
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let orders = json["orders"].as_array().expect("orders");
    let found = orders
        .iter()
        .any(|o| o["client_order_id"] == clid && o["status"] == "Filled" && o["cum_qty"] == 3);
    assert!(found, "hydrated engine should expose filled order {clid}");
}

#[tokio::test]
async fn restart_hydrates_working_order_and_cancel() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run dual-write tests");
        return;
    }
    let _guard = DB_LOCK.lock().await;

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let account_raw = 303_u64;
    let clid = format!("rest-ord-{}", unique_suffix());
    let order_id;

    {
        let state = AppState::for_test_with_store_resting(
            "rest-tok",
            "trader",
            account_raw,
            10_000,
            pool.clone(),
        )
        .await;
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/orders?token=rest-tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "client_order_id": clid,
                            "symbol": "AAPL",
                            "side": "Buy",
                            "qty": 4,
                            "price": 10000
                        })
                        .to_string(),
                    ))
                    .expect("req"),
            )
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("bytes");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["status"], "New");
        order_id = json["id"].as_u64().expect("id");

        let account = AccountId::from_u64(account_raw);
        let client = ClientOrderId::new(&clid).expect("clid");
        let stored = load_order_by_client(&pool, account, &client)
            .await
            .expect("load")
            .expect("working order persisted");
        assert_eq!(stored.status.as_str(), "New");
        assert_eq!(stored.leaves_qty.lots(), 4);
    }

    let state = AppState::for_test_hydrate("rest-tok", "trader", account_raw, pool.clone()).await;
    let app = router(state);
    let recon_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/reconciliation?token=rest-tok")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("recon");
    assert_eq!(recon_resp.status(), StatusCode::OK);
    let recon_body = axum::body::to_bytes(recon_resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let recon_json: serde_json::Value = serde_json::from_slice(&recon_body).expect("json");
    assert_eq!(
        recon_json["ok"], true,
        "hydrate must reinflate venue for working orders: {recon_json}"
    );

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orders/{order_id}/cancel?token=rest-tok"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("cancel");
    let cancel_status = cancel.status();
    let cbody = axum::body::to_bytes(cancel.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let cjson: serde_json::Value = serde_json::from_slice(&cbody).expect("json");
    assert_eq!(cancel_status, StatusCode::OK, "cancel failed: {cjson}");
    assert_eq!(cjson["status"], "Canceled");
}

#[tokio::test]
async fn outbox_publisher_marks_ledger_events() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run dual-write tests");
        return;
    }
    let _guard = DB_LOCK.lock().await;

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let account_raw = 404_u64;
    let state =
        AppState::for_test_with_store("ob-tok", "trader", account_raw, 10_000, pool.clone()).await;
    let metrics = state.outbox_metrics();
    let app = router(state);

    let clid = format!("ob-ord-{}", unique_suffix());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=ob-tok")
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
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let published = shinrai_order_gateway::publish_outbox_once(&pool, &metrics)
        .await
        .expect("publish");
    assert!(published > 0, "expected outbox events after submit");

    let remaining = shinrai_store::claim_unpublished(&pool, 100)
        .await
        .expect("claim");
    assert!(
        remaining.is_empty(),
        "all outbox rows should be marked published"
    );
}
