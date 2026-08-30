//! Double-entry journal and paper-account projections.
//!
//! Balances are derived from immutable, balanced ledger entries. The journal
//! never updates a posted amount; corrections are reversing entries.

#![forbid(unsafe_code)]

mod account;
mod entry;
mod error;
mod journal;
mod paper;
mod posting;

pub use account::{AccountId, LedgerAccount};
pub use entry::{BalancedEntry, EntryBuilder, EntryId, IdempotencyKey};
pub use error::LedgerError;
pub use journal::{Journal, PostOutcome};
pub use paper::{PaperAccount, PaperAccountStatus, PaperBook};
pub use posting::{Direction, Posting};
