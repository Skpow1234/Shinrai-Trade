//! Connection pool and migrations.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::error::StoreError;
use crate::MIGRATOR;

/// Shared Postgres pool.
pub type StorePool = PgPool;

/// Connects using `SHINRAI_DATABASE_URL`, falling back to `DATABASE_URL`.
///
/// # Errors
///
/// Returns [`StoreError::MissingDatabaseUrl`] or connection errors.
pub async fn connect_from_env() -> Result<StorePool, StoreError> {
    let url = std::env::var("SHINRAI_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .map_err(|_| StoreError::MissingDatabaseUrl)?;
    if url.trim().is_empty() {
        return Err(StoreError::MissingDatabaseUrl);
    }
    let max = std::env::var("SHINRAI_DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    connect(&url, max).await
}

/// Connects to Postgres with the given URL and pool size.
///
/// # Errors
///
/// Returns sqlx connection errors.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<StorePool, StoreError> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections.max(1))
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Applies embedded migrations.
///
/// # Errors
///
/// Returns migration errors.
pub async fn migrate(pool: &StorePool) -> Result<(), StoreError> {
    MIGRATOR.run(pool).await?;
    Ok(())
}
