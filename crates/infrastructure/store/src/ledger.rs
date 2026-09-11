//! Ledger entry + posting persistence (with optional outbox in one transaction).

use sqlx::PgPool;

use shinrai_instruments::InstrumentId;
use shinrai_ledger::{AccountId, BalancedEntry, Direction, LedgerAccount};
use shinrai_money::Money;

use crate::error::StoreError;
use crate::outbox;

/// One posting as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerPostingSnapshot {
    /// Chart-of-accounts kind label.
    pub account_kind: String,
    /// Customer account when applicable.
    pub account_id: Option<AccountId>,
    /// ISO currency when cash-like.
    pub currency_code: Option<String>,
    /// Instrument when position memo.
    pub instrument_id: Option<InstrumentId>,
    /// Debit or Credit.
    pub direction: Direction,
    /// Minor units (i128).
    pub minor_units: i128,
}

/// Ledger entry snapshot for insert/load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntrySnapshot {
    /// Assigned DB id after insert (0 before insert).
    pub id: i64,
    /// Idempotency key.
    pub idempotency_key: String,
    /// Causation id.
    pub causation_id: Option<String>,
    /// Correlation id.
    pub correlation_id: Option<String>,
    /// Postings.
    pub postings: Vec<LedgerPostingSnapshot>,
}

impl LedgerEntrySnapshot {
    /// Builds from a balanced domain entry (id left at 0).
    #[must_use]
    pub fn from_balanced(entry: &BalancedEntry) -> Self {
        let postings = entry
            .postings()
            .iter()
            .map(|p| encode_account(p.account(), p.direction(), p.amount()))
            .collect();
        Self {
            id: 0,
            idempotency_key: entry.idempotency_key().as_str().to_owned(),
            causation_id: entry.causation_id().map(str::to_owned),
            correlation_id: entry.correlation_id().map(str::to_owned),
            postings,
        }
    }
}

fn encode_account(
    account: LedgerAccount,
    direction: Direction,
    amount: Money,
) -> LedgerPostingSnapshot {
    let (kind, account_id, currency_code, instrument_id) = match account {
        LedgerAccount::CustomerCash { account, currency } => (
            "customer_cash",
            Some(account),
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::CustomerCashReserved { account, currency } => (
            "customer_cash_reserved",
            Some(account),
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::PaperFunding { currency } => (
            "paper_funding",
            None,
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::BrokerSettlement { currency } => (
            "broker_settlement",
            None,
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::FeesRevenue { currency } => (
            "fees_revenue",
            None,
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::HouseSuspense { currency } => (
            "house_suspense",
            None,
            Some(currency.code().as_str().to_owned()),
            None,
        ),
        LedgerAccount::CustomerPosition {
            account,
            instrument,
        } => (
            "customer_position",
            Some(account),
            None,
            Some(instrument),
        ),
    };
    LedgerPostingSnapshot {
        account_kind: kind.to_owned(),
        account_id,
        currency_code,
        instrument_id,
        direction,
        minor_units: amount.minor_units(),
    }
}

/// Inserts a ledger entry + postings. Duplicate idempotency key returns the existing id.
///
/// When `outbox_topic` is `Some`, also inserts an outbox row in the same transaction.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn insert_ledger_entry(
    pool: &PgPool,
    snap: &LedgerEntrySnapshot,
    outbox_topic: Option<&str>,
    outbox_payload: Option<serde_json::Value>,
) -> Result<i64, StoreError> {
    let mut tx = pool.begin().await?;

    let existing: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM ledger_entries WHERE idempotency_key = $1")
            .bind(&snap.idempotency_key)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((id,)) = existing {
        tx.commit().await?;
        return Ok(id);
    }

    let (entry_id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO ledger_entries (idempotency_key, causation_id, correlation_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(&snap.idempotency_key)
    .bind(snap.causation_id.as_deref())
    .bind(snap.correlation_id.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    for p in &snap.postings {
        let direction = match p.direction {
            Direction::Debit => "Debit",
            Direction::Credit => "Credit",
        };
        sqlx::query(
            r#"
            INSERT INTO ledger_postings (
                entry_id, account_kind, account_id, currency_code, instrument_id,
                direction, minor_units
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(entry_id)
        .bind(&p.account_kind)
        .bind(p.account_id.map(|a| i64::try_from(a.get()).unwrap_or(i64::MAX)))
        .bind(p.currency_code.as_deref())
        .bind(
            p.instrument_id
                .map(|i| i64::try_from(i.get()).unwrap_or(i64::MAX)),
        )
        .bind(direction)
        .bind(p.minor_units.to_string())
        .execute(&mut *tx)
        .await?;
    }

    if let (Some(topic), Some(payload)) = (outbox_topic, outbox_payload) {
        outbox::insert_outbox_tx(&mut tx, topic, &payload).await?;
    }

    tx.commit().await?;
    Ok(entry_id)
}

/// Loads a ledger entry by idempotency key.
///
/// # Errors
///
/// Returns sqlx / decode errors.
pub async fn load_ledger_entry_by_key(
    pool: &PgPool,
    key: &str,
) -> Result<Option<LedgerEntrySnapshot>, StoreError> {
    let row: Option<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, idempotency_key, causation_id, correlation_id
        FROM ledger_entries WHERE idempotency_key = $1
        "#,
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;

    let Some((id, idempotency_key, causation_id, correlation_id)) = row else {
        return Ok(None);
    };

    let posting_rows: Vec<PostingRow> = sqlx::query_as(
        r#"
        SELECT account_kind, account_id, currency_code, instrument_id, direction, minor_units
        FROM ledger_postings WHERE entry_id = $1 ORDER BY id
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let mut postings = Vec::with_capacity(posting_rows.len());
    for pr in posting_rows {
        let direction = match pr.direction.as_str() {
            "Debit" => Direction::Debit,
            "Credit" => Direction::Credit,
            other => {
                return Err(StoreError::InvalidStored {
                    field: "direction",
                    value: other.to_owned(),
                });
            }
        };
        let minor_units = pr.minor_units.parse::<i128>().map_err(|_| {
            StoreError::InvalidInteger {
                field: "minor_units",
                value: pr.minor_units.clone(),
            }
        })?;
        // Validate currency shape when present (round-trip sanity).
        if let Some(ref code) = pr.currency_code {
            let _ = shinrai_money::CurrencyCode::new(code).map_err(|_| StoreError::InvalidStored {
                field: "currency_code",
                value: code.clone(),
            })?;
        }
        postings.push(LedgerPostingSnapshot {
            account_kind: pr.account_kind,
            account_id: pr
                .account_id
                .map(|a| AccountId::from_u64(u64::try_from(a).unwrap_or(0))),
            currency_code: pr.currency_code,
            instrument_id: pr
                .instrument_id
                .map(|i| InstrumentId::from_u64(u64::try_from(i).unwrap_or(0))),
            direction,
            minor_units,
        });
    }

    Ok(Some(LedgerEntrySnapshot {
        id,
        idempotency_key,
        causation_id,
        correlation_id,
        postings,
    }))
}

#[derive(Debug, sqlx::FromRow)]
struct PostingRow {
    account_kind: String,
    account_id: Option<i64>,
    currency_code: Option<String>,
    instrument_id: Option<i64>,
    direction: String,
    minor_units: String,
}
