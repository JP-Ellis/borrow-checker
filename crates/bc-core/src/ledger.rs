//! The `ledger-id` row in `meta`, which names a database's backup pool.

use bc_models::LedgerId;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// The `meta` key holding the ledger ID.
const KEY: &str = "ledger-id";

/// Reads the stored ledger ID, if any.
///
/// A file without a `meta` table (one that predates the schema) reads as
/// `None`, so the pre-migration snapshot can still pick a pool.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the stored value is not a `ledger` `TypeID`.
pub(crate) async fn read(pool: &SqlitePool) -> BcResult<Option<LedgerId>> {
    let has_meta: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
    )
    .fetch_one(pool)
    .await?;
    if !has_meta {
        return Ok(None);
    }
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await?;
    raw.map(|value| {
        let text: String = serde_json::from_str(&value)
            .map_err(|e| BcError::BadData(format!("invalid ledger-id in meta: {e}")))?;
        text.parse::<LedgerId>()
            .map_err(|e| BcError::BadData(format!("invalid ledger-id in meta: {e}")))
    })
    .transpose()
}

/// Stores `id` unless the database already has a ledger ID, then returns the
/// stored one.
///
/// # Errors
///
/// Returns [`BcError`] if the write or the read-back fails.
pub(crate) async fn persist(pool: &SqlitePool, id: &LedgerId) -> BcResult<LedgerId> {
    let value = serde_json::to_string(&id.to_string())?;
    sqlx::query("INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)")
        .bind(KEY)
        .bind(value)
        .execute(pool)
        .await?;
    read(pool)
        .await?
        .ok_or_else(|| BcError::BadData("ledger-id missing after insert".to_owned()))
}

/// Overwrites the stored ledger ID.
///
/// # Errors
///
/// Returns [`BcError`] if the write fails.
pub(crate) async fn replace(pool: &SqlitePool, id: &LedgerId) -> BcResult<()> {
    let value = serde_json::to_string(&id.to_string())?;
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES (?, ?) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(KEY)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns the database's ledger ID, minting and storing one if absent.
///
/// # Arguments
///
/// * `pool` - A migrated pool.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the stored value is malformed, or
/// [`BcError::Database`] if the query fails.
#[inline]
pub async fn ensure_ledger_id(pool: &SqlitePool) -> BcResult<LedgerId> {
    match read(pool).await? {
        Some(id) => Ok(id),
        None => persist(pool, &LedgerId::new()).await,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;

    use crate::BcError;

    #[tokio::test]
    async fn a_ledger_id_persists_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("db.sqlite");

        let first_pool = crate::open_db_at(&db).await.expect("open");
        let first = crate::ensure_ledger_id(&first_pool).await.expect("id");
        first_pool.close().await;

        let second_pool = crate::open_db_at(&db).await.expect("reopen");
        let second = crate::ensure_ledger_id(&second_pool).await.expect("id");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn two_databases_get_different_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = crate::open_db_at(&dir.path().join("a.sqlite"))
            .await
            .expect("a");
        let b = crate::open_db_at(&dir.path().join("b.sqlite"))
            .await
            .expect("b");

        assert_ne!(
            crate::ensure_ledger_id(&a).await.expect("a id"),
            crate::ensure_ledger_id(&b).await.expect("b id"),
        );
    }

    #[tokio::test]
    async fn read_tolerates_a_file_without_the_meta_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("raw.sqlite"))
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.expect("raw");

        assert_eq!(super::read(&pool).await.expect("read"), None);
    }

    #[tokio::test]
    async fn a_malformed_ledger_id_is_bad_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::open_db_at(&dir.path().join("db.sqlite"))
            .await
            .expect("open");
        sqlx::query("UPDATE meta SET value = '\"account_01h455vb4pex5vsknk084sn02q\"' WHERE key = 'ledger-id'")
            .execute(&pool)
            .await
            .expect("corrupt");

        let err = crate::ensure_ledger_id(&pool).await.expect_err("must fail");

        assert!(matches!(err, BcError::BadData(_)), "{err:?}");
    }

    #[tokio::test]
    async fn replace_overwrites_the_stored_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::open_db_at(&dir.path().join("db.sqlite"))
            .await
            .expect("open");
        let fresh = bc_models::LedgerId::new();

        super::replace(&pool, &fresh).await.expect("replace");

        assert_eq!(crate::ensure_ledger_id(&pool).await.expect("id"), fresh);
    }
}
