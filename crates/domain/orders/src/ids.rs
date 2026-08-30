//! Identifiers for orders and venue references.

use core::fmt;

use crate::error::OrderError;

/// Internal order id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OrderId(u64);

impl OrderId {
    /// Creates an order id from a raw value.
    #[must_use]
    pub const fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Client-assigned order id (idempotency key component).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientOrderId(String);

impl ClientOrderId {
    /// Creates a client order id.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::InvalidIdentifier`] if empty after trim.
    pub fn new(id: impl Into<String>) -> Result<Self, OrderError> {
        let id = id.into().trim().to_owned();
        if id.is_empty() {
            return Err(OrderError::InvalidIdentifier);
        }
        Ok(Self(id))
    }

    /// Returns the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientOrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Venue-assigned order id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VenueOrderId(String);

impl VenueOrderId {
    /// Creates a venue order id.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::InvalidIdentifier`] if empty after trim.
    pub fn new(id: impl Into<String>) -> Result<Self, OrderError> {
        let id = id.into().trim().to_owned();
        if id.is_empty() {
            return Err(OrderError::InvalidIdentifier);
        }
        Ok(Self(id))
    }

    /// Returns the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VenueOrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Venue execution / trade id (deduped per order).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecId(String);

impl ExecId {
    /// Creates an execution id.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::InvalidIdentifier`] if empty after trim.
    pub fn new(id: impl Into<String>) -> Result<Self, OrderError> {
        let id = id.into().trim().to_owned();
        if id.is_empty() {
            return Err(OrderError::InvalidIdentifier);
        }
        Ok(Self(id))
    }

    /// Returns the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
