//! Dual-write integration: order gateway → `shinrai-store`.
//!
//! Skips when `SHINRAI_DATABASE_URL` / `DATABASE_URL` is unset.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_ledger::AccountId;
use shinrai_order_gateway::{router, AppState};
use shinrai_orders::ClientOrderId;
use shinrai_store::{
    connect_from_env, load_audit_after, load_order_by_client, migrate, StoreError,
};
use tower::ServiceExt;

fn database_configured() -> bool {
    std::env::var("SHINRAI_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .is_some_and(|u| !u.trim().is_empty())
}

#[tokio::test]
async fn submit_persists_order_and_audit() {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run dual-write tests");
        return;
    }

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let state = AppState::for_test_with_store("dw-tok", "trader", 1, 10_000, pool.clone()).await;
    let app = router(state);

    let clid = format!(
        "dw-ord-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

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

    let account = AccountId::from_u64(1);
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

    let pool = match connect_from_env().await {
        Ok(p) => p,
        Err(StoreError::MissingDatabaseUrl) => return,
        Err(err) => panic!("connect: {err}"),
    };
    migrate(&pool).await.expect("migrate");

    let clid = format!(
        "hy-ord-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    {
        let state =
            AppState::for_test_with_store("hy-tok", "trader", 1, 10_000, pool.clone()).await;
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
    }

    // Simulate process restart: empty deposits, load from Postgres.
    let state = AppState::for_test_hydrate("hy-tok", "trader", 1, pool.clone()).await;
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
