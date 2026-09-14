//! Consumer inbox deduplication for outbox events.

use sqlx::PgPool;

use crate::error::StoreError;

/// Records that `consumer` processed `event_id`.
///
/// Returns `true` when this is the first successful claim (caller should handle
/// the event). Returns `false` when the pair was already processed (skip).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn try_claim_inbox(
    pool: &PgPool,
    consumer: &str,
    event_id: i64,
) -> Result<bool, StoreError> {
    let inserted: Option<(i64,)> = sqlx::query_as(
        r"
        INSERT INTO inbox_dedup (consumer, event_id)
        VALUES ($1, $2)
        ON CONFLICT (consumer, event_id) DO NOTHING
        RETURNING event_id
        ",
    )
    .bind(consumer)
    .bind(event_id)
    .fetch_optional(pool)
    .await?;
    Ok(inserted.is_some())
}
