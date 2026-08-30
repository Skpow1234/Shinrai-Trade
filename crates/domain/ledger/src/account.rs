//! Ledger and paper account identifiers.

use core::fmt;

use shinrai_instruments::InstrumentId;
use shinrai_money::Currency;

/// Customer / book account identifier (paper or later live).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(u64);

impl AccountId {
    /// Creates an account id from a raw `u64`.
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

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Chart-of-accounts key for a posting.
///
/// Asset-like accounts use debit-positive balances. Revenue is typically
/// credit-positive (a negative signed balance in [`crate::Journal::signed_balance`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LedgerAccount {
    /// Available customer cash.
    CustomerCash {
        /// Owning paper/live account.
        account: AccountId,
        /// Currency of the cash bucket.
        currency: Currency,
    },
    /// Cash reserved for working orders.
    CustomerCashReserved {
        /// Owning paper/live account.
        account: AccountId,
        /// Currency of the reserved bucket.
        currency: Currency,
    },
    /// Counterparty for paper deposits and withdrawals (balanced funding).
    PaperFunding {
        /// Funding currency.
        currency: Currency,
    },
    /// Broker / venue settlement account (paper).
    BrokerSettlement {
        /// Settlement currency.
        currency: Currency,
    },
    /// Fees charged on the customer cash book (Phase 1).
    ///
    /// A buy fee debits this account and credits customer cash (available down).
    FeesRevenue {
        /// Fee currency.
        currency: Currency,
    },
    /// Should remain at zero; used for breaks.
    HouseSuspense {
        /// Suspense currency.
        currency: Currency,
    },
    /// Inventory memo account (quantity tracked separately on the paper book).
    CustomerPosition {
        /// Owning account.
        account: AccountId,
        /// Instrument held.
        instrument: InstrumentId,
    },
}

impl LedgerAccount {
    /// Currency for cash-like accounts; `None` for quantity positions.
    #[must_use]
    pub const fn currency(self) -> Option<Currency> {
        match self {
            Self::CustomerCash { currency, .. }
            | Self::CustomerCashReserved { currency, .. }
            | Self::PaperFunding { currency }
            | Self::BrokerSettlement { currency }
            | Self::FeesRevenue { currency }
            | Self::HouseSuspense { currency } => Some(currency),
            Self::CustomerPosition { .. } => None,
        }
    }
}

impl fmt::Display for LedgerAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CustomerCash { account, currency } => {
                write!(f, "customer_cash:{account}:{currency}")
            }
            Self::CustomerCashReserved { account, currency } => {
                write!(f, "customer_cash_reserved:{account}:{currency}")
            }
            Self::PaperFunding { currency } => write!(f, "paper_funding:{currency}"),
            Self::BrokerSettlement { currency } => write!(f, "broker_settlement:{currency}"),
            Self::FeesRevenue { currency } => write!(f, "fees_revenue:{currency}"),
            Self::HouseSuspense { currency } => write!(f, "house_suspense:{currency}"),
            Self::CustomerPosition {
                account,
                instrument,
            } => write!(f, "customer_position:{account}:{instrument}"),
        }
    }
}
