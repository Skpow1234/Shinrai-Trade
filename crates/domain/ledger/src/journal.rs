//! Append-only journal with running balances and idempotent posting.

use std::collections::HashMap;

use shinrai_money::{Currency, Money};

use crate::account::LedgerAccount;
use crate::entry::{BalancedEntry, EntryId, IdempotencyKey};
use crate::error::LedgerError;
use crate::posting::Direction;

/// Result of posting to the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    /// Entry was appended.
    Applied(EntryId),
    /// An entry with the same idempotency key was already present.
    Duplicate(EntryId),
}

/// In-memory append-only journal.
#[derive(Debug, Default, Clone)]
pub struct Journal {
    next_id: u64,
    entries: Vec<(EntryId, BalancedEntry)>,
    by_key: HashMap<IdempotencyKey, EntryId>,
    /// Signed balance per account: debit − credit (minor units).
    signed: HashMap<LedgerAccount, i128>,
}

impl Journal {
    /// Creates an empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored entries (excluding duplicates that were not appended).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if no entries have been applied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Posts a balanced entry. Duplicate keys are a no-op.
    ///
    /// # Errors
    ///
    /// Returns overflow if applying the entry would overflow a running balance.
    pub fn post(&mut self, entry: BalancedEntry) -> Result<PostOutcome, LedgerError> {
        if let Some(id) = self.by_key.get(entry.idempotency_key()) {
            return Ok(PostOutcome::Duplicate(*id));
        }

        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(LedgerError::Money(shinrai_money::MoneyError::Overflow))?;
        let id = EntryId::from_u64(self.next_id);

        for posting in entry.postings() {
            let delta = match posting.direction() {
                Direction::Debit => posting.amount().minor_units(),
                Direction::Credit => posting
                    .amount()
                    .minor_units()
                    .checked_neg()
                    .ok_or(LedgerError::Money(shinrai_money::MoneyError::Overflow))?,
            };
            let slot = self.signed.entry(posting.account()).or_insert(0);
            *slot = slot
                .checked_add(delta)
                .ok_or(LedgerError::Money(shinrai_money::MoneyError::Overflow))?;
        }

        self.by_key.insert(entry.idempotency_key().clone(), id);
        self.entries.push((id, entry));
        Ok(PostOutcome::Applied(id))
    }

    /// Signed balance (debit − credit) for an account, or zero if unused.
    #[must_use]
    pub fn signed_balance(&self, account: LedgerAccount) -> i128 {
        self.signed.get(&account).copied().unwrap_or(0)
    }

    /// Cash-like balance as [`Money`] when the account has a currency.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::InvalidQuantity`] if the account is a position memo.
    pub fn money_balance(&self, account: LedgerAccount) -> Result<Money, LedgerError> {
        let currency = account.currency().ok_or(LedgerError::InvalidQuantity)?;
        Ok(Money::from_minor(self.signed_balance(account), currency))
    }

    /// True when, for every currency, total posted debits equal total credits.
    #[must_use]
    pub fn trial_balance_ok(&self) -> bool {
        let mut debit: HashMap<Currency, i128> = HashMap::new();
        let mut credit: HashMap<Currency, i128> = HashMap::new();
        for (_, entry) in &self.entries {
            for posting in entry.postings() {
                let ccy = posting.amount().currency();
                let amt = posting.amount().minor_units();
                match posting.direction() {
                    Direction::Debit => {
                        *debit.entry(ccy).or_insert(0) += amt;
                    }
                    Direction::Credit => {
                        *credit.entry(ccy).or_insert(0) += amt;
                    }
                }
            }
        }
        debit == credit
    }

    /// Iterates applied entries in order.
    pub fn entries(&self) -> impl Iterator<Item = (EntryId, &BalancedEntry)> {
        self.entries.iter().map(|(id, e)| (*id, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountId;
    use crate::entry::EntryBuilder;

    #[test]
    fn duplicate_key_does_not_grow_journal() {
        let mut j = Journal::new();
        let amt = Money::from_major(5, Currency::usd()).expect("m");
        let acc = AccountId::from_u64(1);
        let build = || {
            EntryBuilder::new("dep-1")
                .expect("k")
                .debit(
                    LedgerAccount::CustomerCash {
                        account: acc,
                        currency: Currency::usd(),
                    },
                    amt,
                )
                .credit(
                    LedgerAccount::PaperFunding {
                        currency: Currency::usd(),
                    },
                    amt,
                )
                .build()
                .expect("b")
        };
        assert!(matches!(
            j.post(build()).expect("p1"),
            PostOutcome::Applied(_)
        ));
        assert_eq!(j.len(), 1);
        assert!(matches!(
            j.post(build()).expect("p2"),
            PostOutcome::Duplicate(_)
        ));
        assert_eq!(j.len(), 1);
        assert!(j.trial_balance_ok());
    }
}
