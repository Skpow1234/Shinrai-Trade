//! Consumer-side venue session cursor (gap-fill position).

use sqlx::{PgPool, Postgres, Transaction};

use crate::error::StoreError;

/// Applied venue session cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueSessionCursorSnapshot {
    /// Last applied session number (`None` = none yet).
    pub applied_session_n: Option<u32>,
    /// Next expected report sequence within that session.
    pub next_expected_seq: u64,
}

impl Default for VenueSessionCursorSnapshot {
    fn default() -> Self {
        Self {
            applied_session_n: None,
            next_expected_seq: 1,
        }
    }
}

/// Upserts the single-row consumer cursor.
///
/// # Errors
///
/// Returns sqlx errors.
pub(crate) async fn upsert_venue_session_cursor_tx(
    tx: &mut Transaction<'_, Postgres>,
    snap: &VenueSessionCursorSnapshot,
) -> Result<(), StoreError> {
    let session_n = snap
        .applied_session_n
        .map(|n| i32::try_from(n).unwrap_or(0));
    sqlx::query(
        r"
        INSERT INTO venue_session_cursor (id, applied_session_n, next_expected_seq, updated_at)
        VALUES (1, $1, $2, NOW())
        ON CONFLICT (id) DO UPDATE SET
            applied_session_n = EXCLUDED.applied_session_n,
            next_expected_seq = EXCLUDED.next_expected_seq,
            updated_at = NOW()
        ",
    )
    .bind(session_n)
    .bind(i64::try_from(snap.next_expected_seq).unwrap_or(1))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Loads the consumer venue session cursor.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn load_venue_session_cursor(
    pool: &PgPool,
) -> Result<VenueSessionCursorSnapshot, StoreError> {
    let row = sqlx::query_as::<_, CursorRow>(
        r"
        SELECT applied_session_n, next_expected_seq
        FROM venue_session_cursor
        WHERE id = 1
        ",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map_or_else(VenueSessionCursorSnapshot::default, CursorRow::into_snap))
}

#[derive(Debug, sqlx::FromRow)]
struct CursorRow {
    applied_session_n: Option<i32>,
    next_expected_seq: i64,
}

impl CursorRow {
    fn into_snap(self) -> VenueSessionCursorSnapshot {
        VenueSessionCursorSnapshot {
            applied_session_n: self
                .applied_session_n
                .map(|n| u32::try_from(n).unwrap_or(0)),
            next_expected_seq: u64::try_from(self.next_expected_seq).unwrap_or(1).max(1),
        }
    }
}
