//! Integration tests against local Postgres (`compose.yaml`).
//!
//! Skips when `SHINRAI_DATABASE_URL` / `DATABASE_URL` is unset so host CI stays green.

use shinrai_audit::{AuditKind, AuditRecord};
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_ledger::{AccountId, Direction, EntryBuilder, LedgerAccount};
use shinrai_money::{Currency, Money};
use shinrai_orders::{ClientOrderId, ExecId, Order, OrderId, OrderStatus, Side, VenueOrderId};
use shinrai_store::{
    claim_unpublished, connect_from_env, insert_audit_record, insert_ledger_entry,
    load_audit_after, load_ledger_entry_by_key, load_order_by_client, load_order_by_id,
    mark_published, migrate, upsert_order, LedgerEntrySnapshot, OrderSnapshot, StoreError,
    StoredStatus,
};
use sqlx::Row;

fn database_configured() -> bool {
    std::env::var("SHINRAI_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .is_some_and(|u| !u.trim().is_empty())
}

async fn pool_or_skip() -> Option<sqlx::PgPool> {
    if !database_configured() {
        eprintln!("skipping: set SHINRAI_DATABASE_URL to run store integration tests");
        return None;
    }
    let pool = connect_from_env().await.expect("connect");
    migrate(&pool).await.expect("migrate");
    Some(pool)
}

#[tokio::test]
async fn migrate_and_ping() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let row = sqlx::query("SELECT 1 AS n")
        .fetch_one(&pool)
        .await
        .expect("ping");
    let n: i32 = row.get("n");
    assert_eq!(n, 1);
}

#[tokio::test]
async fn order_round_trip_and_client_lookup() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };

    let oid = OrderId::from_u64(900_000 + uuid_like() % 100_000);
    let order = Order::new_pending(
        oid,
        AccountId::from_u64(42),
        ClientOrderId::new(format!("clid-{}", uuid_like())).expect("clid"),
        InstrumentId::from_u64(1),
        Side::Buy,
        QuantityLots::from_lots(10),
        PriceTicks::from_scaled(10_000),
    )
    .expect("order");
    let mut snap = OrderSnapshot::from_order(&order);
    snap.status = StoredStatus::from(OrderStatus::Filled);
    snap.cum_qty = QuantityLots::from_lots(10);
    snap.leaves_qty = QuantityLots::from_lots(0);
    snap.avg_px = Some(PriceTicks::from_scaled(10_000));
    snap.venue_order_id = Some(VenueOrderId::new("V-1").expect("v"));
    snap.seen_execs = vec![ExecId::new("E-1").expect("e")];

    upsert_order(&pool, &snap).await.expect("upsert");
    let loaded = load_order_by_id(&pool, snap.id)
        .await
        .expect("load")
        .expect("present");
    assert_eq!(loaded.client_order_id, snap.client_order_id);
    assert_eq!(loaded.cum_qty.lots(), 10);
    assert_eq!(loaded.seen_execs.len(), 1);

    let by_client = load_order_by_client(&pool, snap.account_id, &snap.client_order_id)
        .await
        .expect("by client")
        .expect("present");
    assert_eq!(by_client.id, snap.id);
}

#[tokio::test]
async fn ledger_idempotent_insert_with_outbox() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };

    let acc = AccountId::from_u64(7);
    let amount = Money::from_major(100, Currency::usd()).expect("m");
    let key = format!("store-dep-{}", uuid_like());
    let entry = EntryBuilder::new(&key)
        .expect("key")
        .debit(
            LedgerAccount::CustomerCash {
                account: acc,
                currency: Currency::usd(),
            },
            amount,
        )
        .credit(
            LedgerAccount::PaperFunding {
                currency: Currency::usd(),
            },
            amount,
        )
        .build()
        .expect("balanced");

    let snap = LedgerEntrySnapshot::from_balanced(&entry);
    let payload = serde_json::json!({ "idempotency_key": key, "kind": "ledger_posted" });
    let id1 = insert_ledger_entry(&pool, &snap, Some("ledger.posted"), Some(payload.clone()))
        .await
        .expect("insert");
    let id2 = insert_ledger_entry(&pool, &snap, Some("ledger.posted"), Some(payload))
        .await
        .expect("dup");
    assert_eq!(id1, id2);

    let loaded = load_ledger_entry_by_key(&pool, &key)
        .await
        .expect("load")
        .expect("present");
    assert_eq!(loaded.postings.len(), 2);
    assert_eq!(loaded.postings[0].direction, Direction::Debit);

    let pending = claim_unpublished(&pool, 50).await.expect("claim");
    assert!(pending.iter().any(|e| e.topic == "ledger.posted"));
    if let Some(ev) = pending.iter().find(|e| e.topic == "ledger.posted") {
        mark_published(&pool, ev.id).await.expect("pub");
    }
}

#[tokio::test]
async fn audit_append_and_page() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };

    let seq = unique_seq(&pool).await;
    let kind = AuditKind::RiskRejected {
        code: "insufficient_buying_power".into(),
    };
    let correlation = Some("ci-audit-1".to_owned());
    let prev_hash = shinrai_audit::GENESIS_HASH.to_owned();
    let content_hash = shinrai_audit::compute_content_hash(
        seq,
        1_700_000_000,
        Some(AccountId::from_u64(1)),
        Some(OrderId::from_u64(9)),
        &kind,
        correlation.as_deref(),
        &prev_hash,
    );
    let record = AuditRecord::from_parts(
        seq,
        1_700_000_000,
        Some(AccountId::from_u64(1)),
        Some(OrderId::from_u64(9)),
        kind,
        correlation,
        prev_hash,
        content_hash,
    );
    insert_audit_record(&pool, &record).await.expect("insert");
    let page = load_audit_after(&pool, seq.saturating_sub(1), 10)
        .await
        .expect("page");
    let loaded = page.iter().find(|r| r.seq() == seq).expect("row");
    assert_eq!(loaded.correlation_id(), Some("ci-audit-1"));
    assert!(!loaded.content_hash().is_empty());
}

#[tokio::test]
async fn missing_url_is_explicit_error() {
    // Ensure the error type exists for callers even when URL is unset in subprocesses.
    // This test does not clear the parent env; it only documents the API.
    let _ = StoreError::MissingDatabaseUrl;
}

async fn unique_seq(pool: &sqlx::PgPool) -> u64 {
    let (max,): (Option<i64>,) = sqlx::query_as("SELECT MAX(seq) FROM audit_records")
        .fetch_one(pool)
        .await
        .unwrap_or((None,));
    u64::try_from(max.unwrap_or(0)).unwrap_or(0) + 1
}

fn uuid_like() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos() % u128::from(u64::MAX)).unwrap_or(1))
        .unwrap_or(1)
}
