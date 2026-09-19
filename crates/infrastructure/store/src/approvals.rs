//! Durable dual-control approval requests.

use sqlx::PgPool;

use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;

use crate::error::StoreError;

/// One dual-control approval row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestSnapshot {
    /// Surrogate id (`0` before insert).
    pub id: u64,
    /// Account.
    pub account_id: AccountId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Symbol display.
    pub symbol: String,
    /// Maker actor.
    pub requested_by: String,
    /// Checker actor when approved.
    pub approved_by: Option<String>,
}

/// Inserts a pending approval request; returns assigned id.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_approval_request(
    pool: &PgPool,
    snap: &ApprovalRequestSnapshot,
) -> Result<u64, StoreError> {
    let (id,): (i64,) = sqlx::query_as(
        r"
        INSERT INTO approval_requests
            (account_id, instrument_id, symbol, requested_by)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        ",
    )
    .bind(i64::try_from(snap.account_id.get()).unwrap_or(i64::MAX))
    .bind(i64::try_from(snap.instrument_id.get()).unwrap_or(i64::MAX))
    .bind(&snap.symbol)
    .bind(&snap.requested_by)
    .fetch_one(pool)
    .await?;
    Ok(u64::try_from(id).unwrap_or(0))
}

/// Marks an approval request approved (idempotent if already set to same actor).
///
/// # Errors
///
/// Returns sqlx errors or `InvalidStored` when missing.
pub async fn approve_approval_request(
    pool: &PgPool,
    id: u64,
    approved_by: &str,
) -> Result<ApprovalRequestSnapshot, StoreError> {
    let updated = sqlx::query(
        r"
        UPDATE approval_requests
        SET approved_by = $2, approved_at = NOW()
        WHERE id = $1 AND approved_by IS NULL
        ",
    )
    .bind(i64::try_from(id).unwrap_or(i64::MAX))
    .bind(approved_by)
    .execute(pool)
    .await?
    .rows_affected();

    if updated == 0 {
        // Either missing or already approved — load and validate.
        let Some(existing) = load_approval_request(pool, id).await? else {
            return Err(StoreError::InvalidStored {
                field: "approval_requests",
                value: "not_found".into(),
            });
        };
        if existing.approved_by.as_deref() == Some(approved_by) {
            return Ok(existing);
        }
        if existing.approved_by.is_some() {
            return Err(StoreError::InvalidStored {
                field: "approval_requests",
                value: "already_approved".into(),
            });
        }
        return Err(StoreError::InvalidStored {
            field: "approval_requests",
            value: "not_found".into(),
        });
    }
    load_approval_request(pool, id)
        .await?
        .ok_or(StoreError::InvalidStored {
            field: "approval_requests",
            value: "not_found".into(),
        })
}

/// Loads one approval by id.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn load_approval_request(
    pool: &PgPool,
    id: u64,
) -> Result<Option<ApprovalRequestSnapshot>, StoreError> {
    let row = sqlx::query_as::<_, ApprovalRow>(
        r"
        SELECT id, account_id, instrument_id, symbol, requested_by, approved_by
        FROM approval_requests
        WHERE id = $1
        ",
    )
    .bind(i64::try_from(id).unwrap_or(i64::MAX))
    .fetch_optional(pool)
    .await?;
    Ok(row.map(ApprovalRow::into_snap))
}

/// Lists all approval requests (pending first, then recent).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn list_approval_requests(
    pool: &PgPool,
) -> Result<Vec<ApprovalRequestSnapshot>, StoreError> {
    let rows = sqlx::query_as::<_, ApprovalRow>(
        r"
        SELECT id, account_id, instrument_id, symbol, requested_by, approved_by
        FROM approval_requests
        ORDER BY (approved_by IS NOT NULL), id DESC
        ",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ApprovalRow::into_snap).collect())
}

#[derive(Debug, sqlx::FromRow)]
struct ApprovalRow {
    id: i64,
    account_id: i64,
    instrument_id: i64,
    symbol: String,
    requested_by: String,
    approved_by: Option<String>,
}

impl ApprovalRow {
    fn into_snap(self) -> ApprovalRequestSnapshot {
        ApprovalRequestSnapshot {
            id: u64::try_from(self.id).unwrap_or(0),
            account_id: AccountId::from_u64(u64::try_from(self.account_id).unwrap_or(0)),
            instrument_id: InstrumentId::from_u64(u64::try_from(self.instrument_id).unwrap_or(0)),
            symbol: self.symbol,
            requested_by: self.requested_by,
            approved_by: self.approved_by,
        }
    }
}
