//! Balanced-by-construction journal entries.

use core::fmt;

use shinrai_money::Money;

use crate::account::LedgerAccount;
use crate::error::LedgerError;
use crate::posting::{Direction, Posting};

/// Sequential journal entry id assigned on first post.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(u64);

impl EntryId {
    /// Creates an entry id (used when replaying a known log).
    #[must_use]
    pub const fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// External idempotency key (`deposit_id`, `exec_id`, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Creates a key from a non-empty string.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::EmptyIdempotencyKey`] if empty after trim.
    pub fn new(key: impl Into<String>) -> Result<Self, LedgerError> {
        let key = key.into().trim().to_owned();
        if key.is_empty() {
            return Err(LedgerError::EmptyIdempotencyKey);
        }
        Ok(Self(key))
    }

    /// Returns the key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A journal entry that has passed balance and currency checks.
///
/// There is no public constructor other than [`EntryBuilder::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalancedEntry {
    idempotency_key: IdempotencyKey,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    postings: Vec<Posting>,
}

impl BalancedEntry {
    /// Idempotency key.
    #[must_use]
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Optional causation id.
    #[must_use]
    pub fn causation_id(&self) -> Option<&str> {
        self.causation_id.as_deref()
    }

    /// Optional correlation id.
    #[must_use]
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    /// Postings (at least two for a simple pair; always balanced).
    #[must_use]
    pub fn postings(&self) -> &[Posting] {
        &self.postings
    }

    /// Returns a reversing entry with a new idempotency key.
    ///
    /// # Errors
    ///
    /// Returns an error if `reversal_key` is empty.
    pub fn reverse(&self, reversal_key: impl Into<String>) -> Result<Self, LedgerError> {
        let mut builder = EntryBuilder::new(reversal_key)?;
        if let Some(c) = &self.causation_id {
            builder = builder.causation(c.clone());
        }
        if let Some(c) = &self.correlation_id {
            builder = builder.correlation(c.clone());
        }
        for posting in &self.postings {
            let flipped = match posting.direction() {
                Direction::Debit => Direction::Credit,
                Direction::Credit => Direction::Debit,
            };
            builder = builder.push(Posting::new(posting.account(), flipped, posting.amount()));
        }
        builder.build()
    }
}

/// Accumulates postings and produces a [`BalancedEntry`] only if valid.
#[derive(Debug, Clone)]
pub struct EntryBuilder {
    idempotency_key: IdempotencyKey,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    postings: Vec<Posting>,
}

impl EntryBuilder {
    /// Starts a builder with an idempotency key.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::EmptyIdempotencyKey`] if the key is empty.
    pub fn new(key: impl Into<String>) -> Result<Self, LedgerError> {
        Ok(Self {
            idempotency_key: IdempotencyKey::new(key)?,
            causation_id: None,
            correlation_id: None,
            postings: Vec::new(),
        })
    }

    /// Sets causation id.
    #[must_use]
    pub fn causation(mut self, id: impl Into<String>) -> Self {
        self.causation_id = Some(id.into());
        self
    }

    /// Sets correlation id.
    #[must_use]
    pub fn correlation(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Appends a posting.
    #[must_use]
    pub fn push(mut self, posting: Posting) -> Self {
        self.postings.push(posting);
        self
    }

    /// Debits `account` by `amount`.
    #[must_use]
    pub fn debit(self, account: LedgerAccount, amount: Money) -> Self {
        self.push(Posting::new(account, Direction::Debit, amount))
    }

    /// Credits `account` by `amount`.
    #[must_use]
    pub fn credit(self, account: LedgerAccount, amount: Money) -> Self {
        self.push(Posting::new(account, Direction::Credit, amount))
    }

    /// Validates balance and currency, producing a persistable entry.
    ///
    /// # Errors
    ///
    /// Returns unbalanced, empty, mixed-currency, or zero-amount errors.
    pub fn build(self) -> Result<BalancedEntry, LedgerError> {
        if self.postings.is_empty() {
            return Err(LedgerError::EmptyEntry);
        }

        let mut debit: i128 = 0;
        let mut credit: i128 = 0;
        let mut currency = None;

        for posting in &self.postings {
            if posting.amount().minor_units() <= 0 {
                return Err(LedgerError::ZeroAmount);
            }
            let ccy = posting.amount().currency();
            match currency {
                None => currency = Some(ccy),
                Some(existing) if existing != ccy => return Err(LedgerError::MixedCurrency),
                Some(_) => {}
            }
            match posting.direction() {
                Direction::Debit => {
                    debit = debit
                        .checked_add(posting.amount().minor_units())
                        .ok_or(LedgerError::Money(shinrai_money::MoneyError::Overflow))?;
                }
                Direction::Credit => {
                    credit = credit
                        .checked_add(posting.amount().minor_units())
                        .ok_or(LedgerError::Money(shinrai_money::MoneyError::Overflow))?;
                }
            }
        }

        if debit != credit {
            return Err(LedgerError::Unbalanced { debit, credit });
        }

        Ok(BalancedEntry {
            idempotency_key: self.idempotency_key,
            causation_id: self.causation_id,
            correlation_id: self.correlation_id,
            postings: self.postings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountId;
    use shinrai_money::{Currency, Money};

    fn cash(account: AccountId) -> LedgerAccount {
        LedgerAccount::CustomerCash {
            account,
            currency: Currency::usd(),
        }
    }

    fn funding() -> LedgerAccount {
        LedgerAccount::PaperFunding {
            currency: Currency::usd(),
        }
    }

    #[test]
    fn rejects_unbalanced() {
        let amt = Money::from_major(1, Currency::usd()).expect("m");
        let err = EntryBuilder::new("k")
            .expect("key")
            .debit(cash(AccountId::from_u64(1)), amt)
            .credit(
                funding(),
                Money::from_major(2, Currency::usd()).expect("m2"),
            )
            .build()
            .expect_err("unbalanced");
        assert!(matches!(err, LedgerError::Unbalanced { .. }));
    }

    #[test]
    fn reverse_nets_to_zero_shape() {
        let amt = Money::from_major(10, Currency::usd()).expect("m");
        let entry = EntryBuilder::new("dep")
            .expect("key")
            .debit(cash(AccountId::from_u64(1)), amt)
            .credit(funding(), amt)
            .build()
            .expect("ok");
        let rev = entry.reverse("dep-rev").expect("rev");
        assert_eq!(rev.postings().len(), 2);
        assert_eq!(rev.postings()[0].direction(), Direction::Credit);
        assert_eq!(rev.postings()[1].direction(), Direction::Debit);
    }
}
