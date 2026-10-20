//! Durable ops audit trail for risk control-plane mutations.

use serde_json::Value;
use sqlx::PgPool;

use crate::error::StoreError;

/// One risk ops audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskOpsAuditSnapshot {
    /// Surrogate id after insert.
    pub id: u64,
    /// Stable action label (`global_kill`, `limits`, `restrict_symbol`, …).
    pub action: String,
    /// Action-specific payload.
    pub detail: Value,
    /// Operator or system actor (`ops`, `system:persist_failed`, …).
    pub actor: Option<String>,
    /// Gateway instance id when known.
    pub instance_id: Option<String>,
    /// Row creation time (UTC RFC3339-ish from Postgres).
    pub created_at: Option<String>,
}

/// Inserts a risk ops audit row.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_risk_ops_audit(
    pool: &PgPool,
    snap: &RiskOpsAuditSnapshot,
) -> Result<u64, StoreError> {
    let (id,): (i64,) = sqlx::query_as(
        r"
        INSERT INTO risk_ops_audit (action, detail_json, actor, instance_id)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        ",
    )
    .bind(&snap.action)
    .bind(&snap.detail)
    .bind(&snap.actor)
    .bind(&snap.instance_id)
    .fetch_one(pool)
    .await?;
    Ok(u64::try_from(id).unwrap_or(0))
}

/// Lists recent risk ops audit rows (newest first).
///
/// `limit` is clamped to `[1, 500]`. When `action` is `Some`, only matching rows
/// are returned.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn list_risk_ops_audit(
    pool: &PgPool,
    limit: i64,
    action: Option<&str>,
) -> Result<Vec<RiskOpsAuditSnapshot>, StoreError> {
    let limit = limit.clamp(1, 500);
    let rows = if let Some(action) = action.filter(|a| !a.is_empty()) {
        sqlx::query_as::<_, RiskOpsRow>(
            r#"
            SELECT id, action, detail_json, actor, instance_id,
                   to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at
            FROM risk_ops_audit
            WHERE action = $2
            ORDER BY id DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .bind(action)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, RiskOpsRow>(
            r#"
            SELECT id, action, detail_json, actor, instance_id,
                   to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS created_at
            FROM risk_ops_audit
            ORDER BY id DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    Ok(rows.into_iter().map(RiskOpsRow::into_snap).collect())
}

#[derive(Debug, sqlx::FromRow)]
struct RiskOpsRow {
    id: i64,
    action: String,
    detail_json: Value,
    actor: Option<String>,
    instance_id: Option<String>,
    created_at: Option<String>,
}

impl RiskOpsRow {
    fn into_snap(self) -> RiskOpsAuditSnapshot {
        RiskOpsAuditSnapshot {
            id: u64::try_from(self.id).unwrap_or(0),
            action: self.action,
            detail: self.detail_json,
            actor: self.actor,
            instance_id: self.instance_id,
            created_at: self.created_at,
        }
    }
}
