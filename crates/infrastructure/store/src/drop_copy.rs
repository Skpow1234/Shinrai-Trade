//! Durable venue Trade drop-copy fills.

use sqlx::{PgPool, Postgres, Transaction};

use shinrai_orders::OrderId;

use crate::error::StoreError;

/// One drop-copy fill row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropCopyFillSnapshot {
    /// Internal order id.
    pub order_id: OrderId,
    /// Venue execution id.
    pub exec_id: String,
    /// Fill qty in lots.
    pub qty: i64,
    /// Fill price in ticks.
    pub price: i64,
    /// Venue session number.
    pub session_n: u32,
    /// Report sequence within the session.
    pub seq: u64,
}

/// Inserts a fill if missing (idempotent on `(order_id, exec_id)`).
///
/// # Errors
///
/// Returns sqlx errors.
pub(crate) async fn upsert_drop_copy_fill_tx(
    tx: &mut Transaction<'_, Postgres>,
    snap: &DropCopyFillSnapshot,
) -> Result<(), StoreError> {
    sqlx::query(
        r"
        INSERT INTO drop_copy_fills (order_id, exec_id, qty, price, session_n, seq)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (order_id, exec_id) DO NOTHING
        ",
    )
    .bind(i64::try_from(snap.order_id.get()).unwrap_or(i64::MAX))
    .bind(&snap.exec_id)
    .bind(snap.qty)
    .bind(snap.price)
    .bind(i32::try_from(snap.session_n).unwrap_or(i32::MAX))
    .bind(i64::try_from(snap.seq).unwrap_or(i64::MAX))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Lists all durable drop-copy fills.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn list_drop_copy_fills(pool: &PgPool) -> Result<Vec<DropCopyFillSnapshot>, StoreError> {
    let rows = sqlx::query_as::<_, DropCopyRow>(
        r"
        SELECT order_id, exec_id, qty, price, session_n, seq
        FROM drop_copy_fills
        ORDER BY session_n, seq, order_id
        ",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(DropCopyRow::into_snap).collect())
}

#[derive(Debug, sqlx::FromRow)]
struct DropCopyRow {
    order_id: i64,
    exec_id: String,
    qty: i64,
    price: i64,
    session_n: i32,
    seq: i64,
}

impl DropCopyRow {
    fn into_snap(self) -> DropCopyFillSnapshot {
        DropCopyFillSnapshot {
            order_id: OrderId::from_u64(u64::try_from(self.order_id).unwrap_or(0)),
            exec_id: self.exec_id,
            qty: self.qty,
            price: self.price,
            session_n: u32::try_from(self.session_n).unwrap_or(0),
            seq: u64::try_from(self.seq).unwrap_or(0),
        }
    }
}
