//! SQLite connection pool setup.

use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::error::BcResult;

/// Opens (or creates) the SQLite database at `url` and runs all pending migrations.
///
/// Pass `"sqlite::memory:"` for an in-memory database (useful in tests).
///
/// # Errors
///
/// Returns [`crate::error::BcError::Database`] if the pool cannot be created
/// or migrations fail.
#[expect(
    clippy::module_name_repetitions,
    reason = "open_db is the canonical name for this function regardless of module path"
)]
#[inline]
pub async fn open_db(url: &str) -> BcResult<SqlitePool> {
    let opts = url.parse::<SqliteConnectOptions>()?.create_if_missing(true);

    let pool = SqlitePool::connect_with(opts).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("database opened and migrations applied");
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    #[sqlx::test(migrations = "./migrations")]
    async fn open_db_runs_migrations(pool: sqlx::SqlitePool) {
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .expect("events table should exist");
        assert_eq!(row.0, 0);
    }
}
