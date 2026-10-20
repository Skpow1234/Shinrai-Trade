//! Active-passive OMS leader lease (Postgres).

use std::time::Duration;

use sqlx::PgPool;

use crate::error::StoreError;

/// Snapshot of the singleton OMS leader lease row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderLease {
    /// Instance id holding the lease (empty when never claimed).
    pub holder: String,
    /// Monotonic epoch; increments when the holder changes.
    pub epoch: i64,
    /// True when `expires_at >= NOW()`.
    pub active: bool,
}

/// Tries to acquire or renew the OMS leader lease for `instance_id`.
///
/// Returns `true` when this instance holds a valid lease after the call.
///
/// # Errors
///
/// Returns sqlx / pool errors.
pub async fn heartbeat_oms_leader(
    pool: &PgPool,
    instance_id: &str,
    ttl: Duration,
) -> Result<bool, StoreError> {
    if instance_id.trim().is_empty() {
        return Err(StoreError::InvalidArgument(
            "oms leader instance_id must be non-empty".into(),
        ));
    }
    let ttl_secs = i64::try_from(ttl.as_secs().max(1)).unwrap_or(1);
    let row: Option<(String,)> = sqlx::query_as(
        r"
        UPDATE oms_leader_lease
        SET
            holder = $1,
            expires_at = NOW() + ($2 * INTERVAL '1 second'),
            epoch = CASE
                WHEN holder IS DISTINCT FROM $1 THEN epoch + 1
                ELSE epoch
            END
        WHERE id = 1
          AND (
                holder = $1
             OR holder = ''
             OR expires_at < NOW()
          )
        RETURNING holder
        ",
    )
    .bind(instance_id)
    .bind(ttl_secs)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some_and(|(holder,)| holder == instance_id))
}

/// Reads the current lease row (for ops / health).
///
/// # Errors
///
/// Returns sqlx errors when the row is missing or the query fails.
pub async fn load_oms_leader_lease(pool: &PgPool) -> Result<LeaderLease, StoreError> {
    let (holder, epoch, active): (String, i64, bool) = sqlx::query_as(
        r"
        SELECT holder, epoch, (expires_at >= NOW()) AS active
        FROM oms_leader_lease
        WHERE id = 1
        ",
    )
    .fetch_one(pool)
    .await?;
    Ok(LeaderLease {
        holder,
        epoch,
        active,
    })
}

/// Forces the OMS leader lease to expire (failover drills / integration tests).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn expire_oms_leader_lease(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE oms_leader_lease SET expires_at = NOW() - INTERVAL '1 second' WHERE id = 1",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns whether `instance_id` currently holds an unexpired lease.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn is_oms_leader(pool: &PgPool, instance_id: &str) -> Result<bool, StoreError> {
    let (ok,): (bool,) = sqlx::query_as(
        r"
        SELECT EXISTS (
            SELECT 1
            FROM oms_leader_lease
            WHERE id = 1
              AND holder = $1
              AND expires_at >= NOW()
        )
        ",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await?;
    Ok(ok)
}
