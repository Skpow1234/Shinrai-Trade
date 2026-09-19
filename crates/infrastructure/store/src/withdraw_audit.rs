//! Durable paper withdraw audit trail.

use sqlx::PgPool;

use shinrai_ledger::AccountId;

use crate::error::StoreError;

/// One withdraw audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawAuditSnapshot {
    /// Surrogate id after insert.
    pub id: u64,
    /// Account.
    pub account_id: AccountId,
    /// Amount in minor units.
    pub amount_minor: i64,
    /// Currency code.
    pub currency: String,
    /// Idempotency key (`withdraw:…`).
    pub idempotency_key: String,
    /// Auth subject when known.
    pub actor_subject: Option<String>,
    /// Whether `X-Admin-Override` was used.
    pub override_used: bool,
}

/// Inserts a withdraw audit row (idempotent on `idempotency_key`).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_withdraw_audit(
    pool: &PgPool,
    snap: &WithdrawAuditSnapshot,
) -> Result<u64, StoreError> {
    let (id,): (i64,) = sqlx::query_as(
        r"
        INSERT INTO withdraw_audit
            (account_id, amount_minor, currency, idempotency_key, actor_subject, override_used)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (idempotency_key) DO UPDATE SET
            idempotency_key = EXCLUDED.idempotency_key
        RETURNING id
        ",
    )
    .bind(i64::try_from(snap.account_id.get()).unwrap_or(i64::MAX))
    .bind(snap.amount_minor)
    .bind(&snap.currency)
    .bind(&snap.idempotency_key)
    .bind(&snap.actor_subject)
    .bind(snap.override_used)
    .fetch_one(pool)
    .await?;
    Ok(u64::try_from(id).unwrap_or(0))
}

/// Lists recent withdraw audit rows (newest first).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn list_withdraw_audit(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<WithdrawAuditSnapshot>, StoreError> {
    let rows = sqlx::query_as::<_, WithdrawRow>(
        r"
        SELECT id, account_id, amount_minor, currency, idempotency_key, actor_subject, override_used
        FROM withdraw_audit
        ORDER BY id DESC
        LIMIT $1
        ",
    )
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(WithdrawRow::into_snap).collect())
}

#[derive(Debug, sqlx::FromRow)]
struct WithdrawRow {
    id: i64,
    account_id: i64,
    amount_minor: i64,
    currency: String,
    idempotency_key: String,
    actor_subject: Option<String>,
    override_used: bool,
}

impl WithdrawRow {
    fn into_snap(self) -> WithdrawAuditSnapshot {
        WithdrawAuditSnapshot {
            id: u64::try_from(self.id).unwrap_or(0),
            account_id: AccountId::from_u64(u64::try_from(self.account_id).unwrap_or(0)),
            amount_minor: self.amount_minor,
            currency: self.currency,
            idempotency_key: self.idempotency_key,
            actor_subject: self.actor_subject,
            override_used: self.override_used,
        }
    }
}
