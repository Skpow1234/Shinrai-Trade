//! Transactional outbox.

use sqlx::{PgPool, Postgres, Transaction};

use crate::error::StoreError;

/// One outbox row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEvent {
    /// Row id.
    pub id: i64,
    /// Topic / routing key.
    pub topic: String,
    /// JSON payload.
    pub payload: serde_json::Value,
}

/// Inserts an unpublished outbox event.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_outbox_event(
    pool: &PgPool,
    topic: &str,
    payload: &serde_json::Value,
) -> Result<i64, StoreError> {
    let mut tx = pool.begin().await?;
    let id = insert_outbox_tx(&mut tx, topic, payload).await?;
    tx.commit().await?;
    Ok(id)
}

pub(crate) async fn insert_outbox_tx(
    tx: &mut Transaction<'_, Postgres>,
    topic: &str,
    payload: &serde_json::Value,
) -> Result<i64, StoreError> {
    let (id,): (i64,) = sqlx::query_as(
        r"
        INSERT INTO outbox_events (topic, payload)
        VALUES ($1, $2)
        RETURNING id
        ",
    )
    .bind(topic)
    .bind(payload)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Claims unpublished events (oldest first).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn claim_unpublished(pool: &PgPool, limit: i64) -> Result<Vec<OutboxEvent>, StoreError> {
    let rows: Vec<(i64, String, serde_json::Value)> = sqlx::query_as(
        r"
        SELECT id, topic, payload
        FROM outbox_events
        WHERE published_at IS NULL
        ORDER BY id ASC
        LIMIT $1
        ",
    )
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, topic, payload)| OutboxEvent { id, topic, payload })
        .collect())
}

/// Marks an outbox row as published.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn mark_published(pool: &PgPool, id: i64) -> Result<(), StoreError> {
    sqlx::query("UPDATE outbox_events SET published_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
