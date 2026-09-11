//! Store errors.

use thiserror::Error;

/// Persistence and migration failures.
#[derive(Debug, Error)]
pub enum StoreError {
    /// sqlx / Postgres error.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    /// Migration runner error.
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// `SHINRAI_DATABASE_URL` (or `DATABASE_URL`) is missing or empty.
    #[error("database URL not configured (set SHINRAI_DATABASE_URL)")]
    MissingDatabaseUrl,
    /// Stored enum / text could not be decoded.
    #[error("invalid stored value for {field}: {value}")]
    InvalidStored {
        /// Column or field name.
        field: &'static str,
        /// Raw value.
        value: String,
    },
    /// Money / integer parse failure from TEXT columns.
    #[error("invalid integer text for {field}: {value}")]
    InvalidInteger {
        /// Column name.
        field: &'static str,
        /// Raw value.
        value: String,
    },
}
