//! Paper accounts: available / reserved cash derived from the journal.

use std::collections::HashMap;

use shinrai_instruments::InstrumentId;
use shinrai_money::{Currency, Money};

use crate::account::{AccountId, LedgerAccount};
use crate::entry::{EntryBuilder, EntryId};
use crate::error::LedgerError;
use crate::journal::{Journal, PostOutcome};

/// Lifecycle of a paper trading account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperAccountStatus {
    /// Can deposit, withdraw, and trade.
    Active,
}

/// Paper trading account metadata (balances live on the journal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperAccount {
    id: AccountId,
    status: PaperAccountStatus,
}

impl PaperAccount {
    /// Creates an active paper account.
    #[must_use]
    pub const fn new(id: AccountId) -> Self {
        Self {
            id,
            status: PaperAccountStatus::Active,
        }
    }

    /// Account id.
    #[must_use]
    pub const fn id(self) -> AccountId {
        self.id
    }

    /// Status.
    #[must_use]
    pub const fn status(self) -> PaperAccountStatus {
        self.status
    }
}

/// Paper book: journal plus position lots and account registry.
#[derive(Debug, Default, Clone)]
pub struct PaperBook {
    journal: Journal,
    accounts: HashMap<AccountId, PaperAccount>,
    /// Signed position in lot units (positive = long).
    positions: HashMap<(AccountId, InstrumentId), i64>,
}

impl PaperBook {
    /// Creates an empty paper book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Underlying journal.
    #[must_use]
    pub const fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Ensures a paper account exists (idempotent).
    pub fn open_account(&mut self, id: AccountId) -> PaperAccount {
        *self
            .accounts
            .entry(id)
            .or_insert_with(|| PaperAccount::new(id))
    }

    /// Available (unreserved) cash.
    #[must_use]
    pub fn available(&self, account: AccountId, currency: Currency) -> Money {
        Money::from_minor(
            self.journal
                .signed_balance(LedgerAccount::CustomerCash { account, currency }),
            currency,
        )
    }

    /// Reserved cash for working orders.
    #[must_use]
    pub fn reserved(&self, account: AccountId, currency: Currency) -> Money {
        Money::from_minor(
            self.journal
                .signed_balance(LedgerAccount::CustomerCashReserved { account, currency }),
            currency,
        )
    }

    /// Position lots (signed).
    #[must_use]
    pub fn position(&self, account: AccountId, instrument: InstrumentId) -> i64 {
        self.positions
            .get(&(account, instrument))
            .copied()
            .unwrap_or(0)
    }

    /// Non-zero positions for an account.
    pub fn positions_for(
        &self,
        account: AccountId,
    ) -> impl Iterator<Item = (InstrumentId, i64)> + '_ {
        self.positions
            .iter()
            .filter_map(move |((acc, inst), lots)| {
                if *acc == account && *lots != 0 {
                    Some((*inst, *lots))
                } else {
                    None
                }
            })
    }

    fn require_funds(have: Money, need: Money) -> Result<(), LedgerError> {
        if have.currency() != need.currency() {
            return Err(LedgerError::Money(
                shinrai_money::MoneyError::CurrencyMismatch {
                    left: *have.currency().code().as_bytes(),
                    right: *need.currency().code().as_bytes(),
                },
            ));
        }
        if have.minor_units() < need.minor_units() {
            return Err(LedgerError::InsufficientFunds);
        }
        Ok(())
    }

    /// Paper deposit: funds appear in available cash, balanced against paper funding.
    ///
    /// # Errors
    ///
    /// Returns ledger errors if the entry cannot be posted.
    pub fn deposit(
        &mut self,
        account: AccountId,
        amount: Money,
        key: impl Into<String>,
    ) -> Result<EntryId, LedgerError> {
        self.open_account(account);
        if amount.minor_units() <= 0 {
            return Err(LedgerError::ZeroAmount);
        }
        let ccy = amount.currency();
        let entry = EntryBuilder::new(key)?
            .debit(
                LedgerAccount::CustomerCash {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .credit(LedgerAccount::PaperFunding { currency: ccy }, amount)
            .build()?;
        match self.journal.post(entry)? {
            PostOutcome::Applied(id) | PostOutcome::Duplicate(id) => Ok(id),
        }
    }

    /// Paper withdrawal from available cash.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::InsufficientFunds`] if available cash is too low.
    pub fn withdraw(
        &mut self,
        account: AccountId,
        amount: Money,
        key: impl Into<String>,
    ) -> Result<EntryId, LedgerError> {
        self.open_account(account);
        if amount.minor_units() <= 0 {
            return Err(LedgerError::ZeroAmount);
        }
        Self::require_funds(self.available(account, amount.currency()), amount)?;
        let ccy = amount.currency();
        let entry = EntryBuilder::new(key)?
            .debit(LedgerAccount::PaperFunding { currency: ccy }, amount)
            .credit(
                LedgerAccount::CustomerCash {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .build()?;
        match self.journal.post(entry)? {
            PostOutcome::Applied(id) | PostOutcome::Duplicate(id) => Ok(id),
        }
    }

    /// Locks available cash into the reserved bucket (pre-trade).
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::InsufficientFunds`] if available cash is too low.
    pub fn reserve_for_order(
        &mut self,
        account: AccountId,
        amount: Money,
        key: impl Into<String>,
    ) -> Result<EntryId, LedgerError> {
        self.open_account(account);
        if amount.minor_units() <= 0 {
            return Err(LedgerError::ZeroAmount);
        }
        let before_len = self.journal.len();
        Self::require_funds(self.available(account, amount.currency()), amount)?;
        let ccy = amount.currency();
        let entry = EntryBuilder::new(key)?
            .debit(
                LedgerAccount::CustomerCashReserved {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .credit(
                LedgerAccount::CustomerCash {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .build()?;
        let outcome = self.journal.post(entry)?;
        debug_assert!(self.journal.trial_balance_ok());
        match outcome {
            PostOutcome::Applied(id) => {
                debug_assert_eq!(self.journal.len(), before_len + 1);
                Ok(id)
            }
            PostOutcome::Duplicate(id) => Ok(id),
        }
    }

    /// Releases reserved cash back to available (cancel / reject).
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::InsufficientReserved`] if reserved is too low.
    pub fn release_reserve(
        &mut self,
        account: AccountId,
        amount: Money,
        key: impl Into<String>,
    ) -> Result<EntryId, LedgerError> {
        self.open_account(account);
        if amount.minor_units() <= 0 {
            return Err(LedgerError::ZeroAmount);
        }
        let reserved = self.reserved(account, amount.currency());
        if reserved.minor_units() < amount.minor_units() {
            return Err(LedgerError::InsufficientReserved);
        }
        let ccy = amount.currency();
        let entry = EntryBuilder::new(key)?
            .debit(
                LedgerAccount::CustomerCash {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .credit(
                LedgerAccount::CustomerCashReserved {
                    account,
                    currency: ccy,
                },
                amount,
            )
            .build()?;
        match self.journal.post(entry)? {
            PostOutcome::Applied(id) | PostOutcome::Duplicate(id) => Ok(id),
        }
    }

    /// Settles a buy: consumes reserved notional, posts fee, increments position.
    ///
    /// # Errors
    ///
    /// Returns reserved/funds/quantity errors. On insufficient reserved, the
    /// journal is left unchanged.
    pub fn settle_buy(
        &mut self,
        account: AccountId,
        instrument: InstrumentId,
        qty_lots: i64,
        notional: Money,
        fee: Money,
        key: impl Into<String>,
    ) -> Result<EntryId, LedgerError> {
        self.open_account(account);
        if qty_lots <= 0 {
            return Err(LedgerError::InvalidQuantity);
        }
        if notional.minor_units() <= 0 {
            return Err(LedgerError::ZeroAmount);
        }
        if fee.minor_units() < 0 {
            return Err(LedgerError::ZeroAmount);
        }
        if notional.currency() != fee.currency() {
            return Err(LedgerError::MixedCurrency);
        }
        let ccy = notional.currency();
        let reserved = self.reserved(account, ccy);
        if reserved.minor_units() < notional.minor_units() {
            return Err(LedgerError::InsufficientReserved);
        }
        if fee.minor_units() > 0 {
            Self::require_funds(self.available(account, ccy), fee)?;
        }

        let len_before = self.journal.len();
        let mut builder = EntryBuilder::new(key)?
            .credit(
                LedgerAccount::CustomerCashReserved {
                    account,
                    currency: ccy,
                },
                notional,
            )
            .debit(LedgerAccount::BrokerSettlement { currency: ccy }, notional);

        if fee.minor_units() > 0 {
            builder = builder
                .credit(
                    LedgerAccount::CustomerCash {
                        account,
                        currency: ccy,
                    },
                    fee,
                )
                .debit(LedgerAccount::FeesRevenue { currency: ccy }, fee);
        }

        let entry = builder.build()?;
        let outcome = self.journal.post(entry)?;
        match outcome {
            PostOutcome::Duplicate(id) => Ok(id),
            PostOutcome::Applied(id) => {
                let pos = self.positions.entry((account, instrument)).or_insert(0);
                *pos = pos
                    .checked_add(qty_lots)
                    .ok_or(LedgerError::InvalidQuantity)?;
                debug_assert!(self.journal.trial_balance_ok());
                debug_assert_eq!(self.journal.len(), len_before + 1);
                Ok(id)
            }
        }
    }

    /// Posts an already-balanced entry (reversals, adjustments).
    ///
    /// # Errors
    ///
    /// Returns overflow errors from the journal.
    pub fn post(&mut self, entry: crate::entry::BalancedEntry) -> Result<PostOutcome, LedgerError> {
        self.journal.post(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;
    use shinrai_money::Currency;

    fn usd(major: i128) -> Money {
        Money::from_major(major, Currency::usd()).expect("usd")
    }

    #[test]
    fn deposit_then_available() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        book.deposit(acc, usd(10_000), "dep").expect("dep");
        assert_eq!(
            book.available(acc, Currency::usd()).minor_units(),
            1_000_000
        );
        assert!(book.reserved(acc, Currency::usd()).is_zero());
        assert!(book.journal().trial_balance_ok());
    }

    #[test]
    fn reserve_conserves_cash_plus_reserved() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        book.deposit(acc, usd(10_000), "dep").expect("dep");
        book.reserve_for_order(acc, usd(2_500), "rsv").expect("rsv");
        let avail = book.available(acc, Currency::usd()).minor_units();
        let rsv = book.reserved(acc, Currency::usd()).minor_units();
        assert_eq!(avail, 750_000);
        assert_eq!(rsv, 250_000);
        assert_eq!(avail + rsv, 1_000_000);
        assert!(book.journal().trial_balance_ok());
    }

    #[test]
    fn release_reserve_restores_available() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        book.deposit(acc, usd(10_000), "dep").expect("dep");
        book.reserve_for_order(acc, usd(2_500), "rsv").expect("rsv");
        book.release_reserve(acc, usd(2_500), "rel").expect("rel");
        assert_eq!(
            book.available(acc, Currency::usd()).minor_units(),
            1_000_000
        );
        assert!(book.reserved(acc, Currency::usd()).is_zero());
    }

    #[test]
    fn settle_buy_opens_position_and_posts_fee() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        let inst = InstrumentId::from_u64(1);
        book.deposit(acc, usd(10_000), "dep").expect("dep");
        let notional = Money::parse_major("100.00", Currency::usd()).expect("n");
        let fee = Money::parse_major("1.00", Currency::usd()).expect("f");
        book.reserve_for_order(acc, notional, "rsv").expect("rsv");
        book.settle_buy(acc, inst, 1, notional, fee, "fill")
            .expect("set");
        assert!(book.reserved(acc, Currency::usd()).is_zero());
        // 10000 - 100 notional - 1 fee
        assert_eq!(book.available(acc, Currency::usd()).minor_units(), 989_900);
        assert_eq!(book.position(acc, inst), 1);
        assert!(book.journal().trial_balance_ok());
    }

    #[test]
    fn insufficient_funds_leaves_journal_unchanged() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        book.deposit(acc, usd(10), "dep").expect("dep");
        let len = book.journal().len();
        let err = book
            .reserve_for_order(acc, usd(50), "rsv")
            .expect_err("insuf");
        assert_eq!(err, LedgerError::InsufficientFunds);
        assert_eq!(book.journal().len(), len);
        assert_eq!(book.available(acc, Currency::usd()).minor_units(), 1_000);
    }

    #[test]
    fn reverse_deposit_nets_balances() {
        let mut book = PaperBook::new();
        let acc = AccountId::from_u64(1);
        book.deposit(acc, usd(100), "dep").expect("dep");
        let original = book.journal().entries().next().expect("e").1.clone();
        let rev = original.reverse("dep-rev").expect("rev");
        book.post(rev).expect("post rev");
        assert!(book.available(acc, Currency::usd()).is_zero());
        assert!(book.journal().trial_balance_ok());
    }
}
