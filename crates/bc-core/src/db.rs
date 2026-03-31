//! SQLite connection pool setup and shared database utilities.

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqliteSynchronous;

use crate::BcError;
use crate::BcResult;

/// Opens (or creates) the SQLite database at `url` and runs all pending migrations.
///
/// Pass `"sqlite::memory:"` for an in-memory database (useful in tests).
///
/// Intended for in-memory / test use; production code should use [`open_db_at`].
///
/// # Errors
///
/// Returns [`BcError::Database`](crate::BcError::Database) if the pool
/// cannot be created or migrations fail.
#[inline]
pub async fn open_db(url: &str) -> BcResult<SqlitePool> {
    // Enable SQLite foreign-key enforcement per-connection.
    // NOTE: account_commodities and account_tags have FKs to commodities and tags.
    // Inserting into those join tables requires the referenced commodity/tag records
    // to already exist, so any test or service that inserts into those tables must
    // first insert the parent commodity or tag row.
    let opts = url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .pragma("foreign_keys", "ON")
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePool::connect_with(opts).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("database opened and migrations applied");
    Ok(pool)
}

/// Opens (or creates) the SQLite database at the given filesystem path and
/// runs all pending migrations.
///
/// Prefer this over [`open_db`] for production callers — it uses
/// [`SqliteConnectOptions::filename`] which handles platform path separators
/// correctly (avoids backslash issues on Windows).
///
/// # Arguments
///
/// * `path` - Filesystem path to the SQLite database file.
///
/// # Returns
///
/// A connected and migrated [`SqlitePool`].
///
/// # Errors
///
/// Returns [`BcError::Database`] if the pool cannot be created or migrations fail.
#[inline]
pub async fn open_db_at(path: &std::path::Path) -> BcResult<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .pragma("foreign_keys", "ON")
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("database opened and migrations applied");
    Ok(pool)
}

// Schema tables (managed by migrations in ./migrations/):
//   events, accounts, commodities, account_commodities, tags, account_tags,
//   transactions, postings, transaction_tags, posting_tags,
//   transaction_links, transaction_link_members,
//   balances (read-cache, deferred — see migration 0007),
//   meta (key-value settings store).
//
// import_profiles table: deferred to Milestone 2 (Format Compatibility).
// See DESIGN.md §4.2 and §5.3.

/// Serialises a serde-enabled unit enum to its canonical database string.
///
/// Uses the type's [`serde::Serialize`] implementation, which must produce a
/// JSON string value (i.e. a unit enum with `#[serde(rename_all = "...")]`).
///
/// # Arguments
///
/// * `val` - The enum value to serialise.
///
/// # Returns
///
/// The string representation as stored in the database (e.g. `"snake_case"`).
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the serde output is not a plain string (future-proofing
/// against `#[non_exhaustive]` additions).
/// Returns [`BcError::Serialisation`] if serialisation itself fails.
#[inline]
pub(crate) fn to_db_str<T: serde::Serialize>(val: T) -> BcResult<String> {
    match serde_json::to_value(val)? {
        serde_json::Value::String(s) => Ok(s),
        other @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Array(_)
        | serde_json::Value::Object(_)) => Err(BcError::BadData(format!(
            "expected a string serde value, got: {other:?}"
        ))),
    }
}

/// Deserialises a serde-enabled unit enum from its canonical database string.
///
/// Uses the type's [`serde::Deserialize`] implementation.
///
/// # Arguments
///
/// * `s` - The string as stored in the database.
///
/// # Returns
///
/// The deserialised enum value.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if the string is not recognised by the
/// type's deserialiser (e.g. unknown variant).
#[inline]
pub(crate) fn from_db_str<T: serde::de::DeserializeOwned>(s: &str) -> BcResult<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).map_err(Into::into)
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
