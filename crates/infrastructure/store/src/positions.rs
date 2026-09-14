//! Paper position lots persistence.

use sqlx::{PgPool, Postgres, Transaction};

use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;

use crate::error::StoreError;

/// One paper position row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperPositionSnapshot {
    /// Account.
    pub account_id: AccountId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Signed lots (positive = long).
    pub lots: i64,
    /// Lots reserved for working sells.
    pub reserved_lots: i64,
}

/// Upserts a paper position row.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn upsert_paper_position(
    pool: &PgPool,
    snap: &PaperPositionSnapshot,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    upsert_paper_position_tx(&mut tx, snap).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn upsert_paper_position_tx(
    tx: &mut Transaction<'_, Postgres>,
    snap: &PaperPositionSnapshot,
) -> Result<(), StoreError> {
    sqlx::query(
        r"
        INSERT INTO paper_positions (account_id, instrument_id, lots, reserved_lots)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (account_id, instrument_id) DO UPDATE SET
            lots = EXCLUDED.lots,
            reserved_lots = EXCLUDED.reserved_lots
        ",
    )
    .bind(i64::try_from(snap.account_id.get()).unwrap_or(i64::MAX))
    .bind(i64::try_from(snap.instrument_id.get()).unwrap_or(i64::MAX))
    .bind(snap.lots)
    .bind(snap.reserved_lots)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Lists all paper positions.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn list_paper_positions(pool: &PgPool) -> Result<Vec<PaperPositionSnapshot>, StoreError> {
    let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        r"
        SELECT account_id, instrument_id, lots, reserved_lots
        FROM paper_positions
        ORDER BY account_id, instrument_id
        ",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(account_id, instrument_id, lots, reserved_lots)| PaperPositionSnapshot {
                account_id: AccountId::from_u64(u64::try_from(account_id).unwrap_or(0)),
                instrument_id: InstrumentId::from_u64(u64::try_from(instrument_id).unwrap_or(0)),
                lots,
                reserved_lots,
            },
        )
        .collect())
}
