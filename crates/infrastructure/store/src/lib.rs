//! `PostgreSQL` persistence for OMS, ledger, audit, and outbox.
//!
//! Domain crates stay free of sqlx. This crate maps domain snapshots to SQL
//! and runs migrations from `migrations/`.

#![forbid(unsafe_code)]

mod audit;
mod error;
mod ledger;
mod orders;
mod outbox;
mod pool;

pub use audit::{insert_audit_record, load_audit_after};
pub use error::StoreError;
pub use ledger::{
    insert_ledger_entry, load_ledger_entry_by_key, LedgerEntrySnapshot, LedgerPostingSnapshot,
};
pub use orders::{
    load_order_by_client, load_order_by_id, upsert_order, OrderSnapshot, StoredSide, StoredStatus,
};
pub use outbox::{claim_unpublished, insert_outbox_event, mark_published, OutboxEvent};
pub use pool::{connect, connect_from_env, migrate, StorePool};

/// Embedded migrator for `migrations/`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
