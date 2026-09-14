//! `PostgreSQL` persistence for OMS, ledger, audit, and outbox.
//!
//! Domain crates stay free of sqlx. This crate maps domain snapshots to SQL
//! and runs migrations from `migrations/`.

#![forbid(unsafe_code)]

mod audit;
mod batch;
mod error;
mod inbox;
mod ledger;
mod orders;
mod outbox;
mod pool;
mod positions;

pub use audit::{insert_audit_record, load_audit_after};
pub use batch::{persist_trading_batch, TradingBatch};
pub use error::StoreError;
pub use inbox::try_claim_inbox;
pub use ledger::{
    insert_ledger_entry, list_ledger_entries, load_ledger_entry_by_key, LedgerEntrySnapshot,
    LedgerPostingSnapshot,
};
pub use orders::{
    has_durable_state, list_orders, load_order_by_client, load_order_by_id, upsert_order,
    OrderSnapshot, StoredSide, StoredStatus,
};
pub use outbox::{claim_unpublished, insert_outbox_event, mark_published, OutboxEvent};
pub use pool::{connect, connect_from_env, migrate, StorePool};
pub use positions::{list_paper_positions, upsert_paper_position, PaperPositionSnapshot};

/// Embedded migrator for `migrations/`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
