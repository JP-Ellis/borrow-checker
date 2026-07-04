//! SQLite connection pool setup and shared database utilities.

use std::path::Path;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqliteSynchronous;

use crate::BackupKind;
use crate::BackupPolicy;
use crate::BackupService;
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

/// Opens the database, taking an automatic snapshot before applying pending
/// migrations, then runs migrations.
///
/// The snapshot is taken only when all of the following hold: the policy has
/// `auto_pre_migration` enabled, the database file already existed and was
/// non-empty before this call, and there are migrations not yet applied. A fresh
/// or up-to-date database is never backed up here.
///
/// # Arguments
///
/// * `path` - Filesystem path to the SQLite database file.
/// * `policy` - Backup directory and retention policy.
///
/// # Returns
///
/// A connected and migrated [`SqlitePool`].
///
/// # Errors
///
/// Returns [`BcError::Database`](crate::BcError::Database) if the pool cannot be
/// created, the snapshot fails, or migrations fail.
#[inline]
pub async fn open_db_with_backup(path: &Path, policy: &BackupPolicy) -> BcResult<SqlitePool> {
    let pre_existing = std::fs::metadata(path).is_ok_and(|m| m.len() > 0);

    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .pragma("foreign_keys", "ON")
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);
    let pool = SqlitePool::connect_with(opts).await?;

    if policy.auto_pre_migration && pre_existing && has_pending_migrations(&pool).await? {
        let svc = BackupService::new(pool.clone(), path.to_path_buf(), policy.clone());
        svc.backup(BackupKind::Automatic, None).await?;
        tracing::info!("pre-migration backup written");
    }

    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("database opened and migrations applied");
    Ok(pool)
}

/// Returns `true` if the bundled migrator has versions beyond those recorded in
/// `_sqlx_migrations` (or that table does not yet exist).
async fn has_pending_migrations(pool: &SqlitePool) -> BcResult<bool> {
    // Missing table (fresh DB) ⇒ everything is pending; treat query error as such.
    let applied: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .flatten();
    let latest = sqlx::migrate!("./migrations")
        .migrations
        .iter()
        .map(|m| m.version)
        .max();
    Ok(latest > applied)
}

// Schema tables (managed by migrations in ./migrations/):
//   events, accounts, commodities, account_commodities, tags, account_tags,
//   transactions, postings, transaction_tags, posting_tags,
//   transaction_links, transaction_link_members,
//   balances (read-cache, deferred — computed live, not yet used),
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

    use crate::BackupPolicy;

    #[tokio::test]
    async fn pre_migration_backup_taken_when_migrations_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        let backups = dir.path().join("backups");

        // Seed a pre-existing, non-empty DB WITHOUT running BC migrations, so
        // "pending migrations" is a genuine migratable state (no _sqlx_migrations
        // table yet) rather than a re-run of already-applied, non-idempotent DDL.
        {
            let opts = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = sqlx::SqlitePool::connect_with(opts)
                .await
                .expect("seed raw db");
            sqlx::query("CREATE TABLE seed_marker (x INTEGER)")
                .execute(&pool)
                .await
                .expect("create seed table");
            pool.close().await;
        }

        let policy = BackupPolicy::new(backups.clone(), Some(5), None, true);
        let pool = crate::open_db_with_backup(&db_path, &policy)
            .await
            .expect("open with backup");
        pool.close().await;

        let count = std::fs::read_dir(&backups).map_or(0, core::iter::Iterator::count);
        assert!(
            count >= 1,
            "a pre-migration backup should have been written"
        );
    }

    #[tokio::test]
    async fn no_backup_for_fresh_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        let backups = dir.path().join("backups");
        let policy = BackupPolicy::new(backups.clone(), Some(5), None, true);

        let pool = crate::open_db_with_backup(&db_path, &policy)
            .await
            .expect("open fresh");
        pool.close().await;

        assert!(
            !backups.exists()
                || std::fs::read_dir(&backups).map_or(0, core::iter::Iterator::count) == 0,
            "a brand-new database has nothing to back up"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn open_db_runs_migrations(pool: sqlx::SqlitePool) {
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .expect("events table should exist");
        assert_eq!(row.0, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sibling_tags_with_same_name_are_rejected(pool: sqlx::SqlitePool) {
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind("tag_a")
            .bind("person")
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("first root insert should succeed");

        let dup = sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind("tag_b")
            .bind("person")
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await;

        assert!(
            dup.is_err(),
            "duplicate root name must violate the unique index"
        );
    }
}
