//! Audit record persistence.

use sqlx::{PgPool, Postgres, Transaction};

use shinrai_audit::{AuditKind, AuditRecord};
use shinrai_ledger::AccountId;
use shinrai_orders::OrderId;

use crate::error::StoreError;

/// Inserts one audit row (append-only; duplicate `seq` is ignored).
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_audit_record(pool: &PgPool, record: &AuditRecord) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    insert_audit_record_tx(&mut tx, record).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn insert_audit_record_tx(
    tx: &mut Transaction<'_, Postgres>,
    record: &AuditRecord,
) -> Result<(), StoreError> {
    let (kind, detail) = encode_kind(record.kind());
    sqlx::query(
        r"
        INSERT INTO audit_records (
            seq, at_unix, account_id, order_id, kind, detail,
            correlation_id, prev_hash, content_hash
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (seq) DO NOTHING
        ",
    )
    .bind(i64::try_from(record.seq()).unwrap_or(i64::MAX))
    .bind(i64::try_from(record.at()).unwrap_or(i64::MAX))
    .bind(
        record
            .account_id()
            .map(|a| i64::try_from(a.get()).unwrap_or(i64::MAX)),
    )
    .bind(
        record
            .order_id()
            .map(|o| i64::try_from(o.get()).unwrap_or(i64::MAX)),
    )
    .bind(kind)
    .bind(detail)
    .bind(record.correlation_id())
    .bind(record.prev_hash())
    .bind(record.content_hash())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Loads audit records with `seq > after_seq`, ascending, limited.
///
/// # Errors
///
/// Returns sqlx / decode errors.
pub async fn load_audit_after(
    pool: &PgPool,
    after_seq: u64,
    limit: i64,
) -> Result<Vec<AuditRecord>, StoreError> {
    let rows: Vec<AuditRow> = sqlx::query_as(
        r"
        SELECT seq, at_unix, account_id, order_id, kind, detail,
               correlation_id, prev_hash, content_hash
        FROM audit_records
        WHERE seq > $1
        ORDER BY seq ASC
        LIMIT $2
        ",
    )
    .bind(i64::try_from(after_seq).unwrap_or(i64::MAX))
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(decode_row(row)?);
    }
    Ok(out)
}

fn encode_kind(kind: &AuditKind) -> (&'static str, Option<String>) {
    match kind {
        AuditKind::OrderSubmitRequested => ("order_submit_requested", None),
        AuditKind::RiskRejected { code } => ("risk_rejected", Some(code.clone())),
        AuditKind::OrderCreated => ("order_created", None),
        AuditKind::OrderDuplicate => ("order_duplicate", None),
        AuditKind::OrderEventApplied { status } => ("order_event_applied", Some(status.clone())),
        AuditKind::LedgerReserved => ("ledger_reserved", None),
        AuditKind::LedgerSettled => ("ledger_settled", None),
        AuditKind::LedgerReleased => ("ledger_released", None),
        AuditKind::VenueSubmitted => ("venue_submitted", None),
        AuditKind::VenueReport { exec_type } => ("venue_report", Some(exec_type.clone())),
    }
}

fn decode_row(row: AuditRow) -> Result<AuditRecord, StoreError> {
    let kind = match row.kind.as_str() {
        "order_submit_requested" => AuditKind::OrderSubmitRequested,
        "risk_rejected" => AuditKind::RiskRejected {
            code: row.detail.unwrap_or_default(),
        },
        "order_created" => AuditKind::OrderCreated,
        "order_duplicate" => AuditKind::OrderDuplicate,
        "order_event_applied" => AuditKind::OrderEventApplied {
            status: row.detail.unwrap_or_default(),
        },
        "ledger_reserved" => AuditKind::LedgerReserved,
        "ledger_settled" => AuditKind::LedgerSettled,
        "ledger_released" => AuditKind::LedgerReleased,
        "venue_submitted" => AuditKind::VenueSubmitted,
        "venue_report" => AuditKind::VenueReport {
            exec_type: row.detail.unwrap_or_default(),
        },
        other => {
            return Err(StoreError::InvalidStored {
                field: "audit.kind",
                value: other.to_owned(),
            });
        }
    };
    Ok(AuditRecord::from_parts(
        u64::try_from(row.seq).unwrap_or(0),
        u64::try_from(row.at_unix).unwrap_or(0),
        row.account_id
            .map(|a| AccountId::from_u64(u64::try_from(a).unwrap_or(0))),
        row.order_id
            .map(|o| OrderId::from_u64(u64::try_from(o).unwrap_or(0))),
        kind,
        row.correlation_id,
        row.prev_hash.unwrap_or_default(),
        row.content_hash.unwrap_or_default(),
    ))
}

#[derive(Debug, sqlx::FromRow)]
struct AuditRow {
    seq: i64,
    at_unix: i64,
    account_id: Option<i64>,
    order_id: Option<i64>,
    kind: String,
    detail: Option<String>,
    correlation_id: Option<String>,
    prev_hash: Option<String>,
    content_hash: Option<String>,
}
