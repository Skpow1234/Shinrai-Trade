//! Individual debit or credit lines.

use shinrai_money::Money;

use crate::account::LedgerAccount;

/// Debit or credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Debit (increases asset-class signed balances).
    Debit,
    /// Credit (decreases asset-class signed balances).
    Credit,
}

/// One line of a journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posting {
    account: LedgerAccount,
    direction: Direction,
    amount: Money,
}

impl Posting {
    /// Creates a posting. Amount must be non-zero; enforced at entry build.
    #[must_use]
    pub const fn new(account: LedgerAccount, direction: Direction, amount: Money) -> Self {
        Self {
            account,
            direction,
            amount,
        }
    }

    /// Ledger account.
    #[must_use]
    pub const fn account(self) -> LedgerAccount {
        self.account
    }

    /// Debit or credit.
    #[must_use]
    pub const fn direction(self) -> Direction {
        self.direction
    }

    /// Absolute amount (always stored positive in [`Money::minor_units`] for
    /// direction; callers should pass a non-negative amount).
    #[must_use]
    pub const fn amount(self) -> Money {
        self.amount
    }
}
