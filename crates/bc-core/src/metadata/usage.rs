//! Counting the entries stored under a metadata key.
//!
//! [`MetaKeyDef`] says what a key is; [`Usage`] says how much of it there is. A
//! retype replays every entry under a key and a delete destroys them, so both
//! want the count before they run.

use std::collections::HashMap;

use bc_models::MetaKey;
use bc_models::MetaKeyDef;
use sqlx::SqlitePool;

use crate::BcResult;
use crate::metadata::Owner;
use crate::metadata::registry::Service;

/// How many entries a metadata key holds.
///
/// Re-exported from the crate root as [`crate::MetaKeyUsage`].
///
/// The whole [`MetaKeyDef`] travels with the counts, so a caller listing the
/// registry with its usage makes one call rather than two.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct Usage {
    /// The key's registration.
    #[serde(flatten)]
    def: MetaKeyDef,
    /// Rows in `transaction_metadata`.
    transactions: u64,
    /// Rows in `posting_metadata`.
    postings: u64,
    /// Rows marked `mismatched`, across both owner tables.
    mismatched: u64,
}

impl Usage {
    /// Returns the key's registration.
    #[must_use]
    #[inline]
    pub fn def(&self) -> &MetaKeyDef {
        &self.def
    }

    /// Returns the key these counts describe.
    #[must_use]
    #[inline]
    pub fn key(&self) -> &MetaKey {
        self.def.key()
    }

    /// Returns the number of entries on transactions.
    #[must_use]
    #[inline]
    pub const fn transactions(&self) -> u64 {
        self.transactions
    }

    /// Returns the number of entries on postings.
    #[must_use]
    #[inline]
    pub const fn postings(&self) -> u64 {
        self.postings
    }

    /// Returns the number of entries across both owner tables that do not read
    /// as the key's registered type.
    ///
    /// This counts damage already done under the type in force, not damage a
    /// retype would do. A key registered `text` always answers zero, because
    /// every value fits a text key and a retype to text clears the mark, so a
    /// zero here does not promise that narrowing the key is free.
    #[must_use]
    #[inline]
    pub const fn mismatched(&self) -> u64 {
        self.mismatched
    }

    /// Returns the number of entries across both owner tables.
    #[must_use]
    #[inline]
    pub const fn total(&self) -> u64 {
        self.transactions.saturating_add(self.postings)
    }
}

impl Service {
    /// Counts the entries stored under one key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to count.
    ///
    /// # Returns
    ///
    /// `Some(usage)` when the key is registered, `None` otherwise. A registered
    /// key holding nothing answers `Some` with zeros, which is what separates
    /// it from an unregistered one.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::Database`] on query failure and
    /// [`crate::BcError::BadData`] if the stored registration cannot be read.
    #[inline]
    pub async fn usage(&self, key: &MetaKey) -> BcResult<Option<Usage>> {
        let Some(def) = self.get(key).await? else {
            return Ok(None);
        };
        let transactions = counts(&self.pool, Owner::Transaction, Some(key)).await?;
        let postings = counts(&self.pool, Owner::Posting, Some(key)).await?;
        Ok(Some(assemble(def, &transactions, &postings)))
    }

    /// Counts the entries stored under every registered key, ordered by key.
    ///
    /// # Returns
    ///
    /// One entry per registered key, including keys holding nothing.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::Database`] on query failure and
    /// [`crate::BcError::BadData`] if a stored registration cannot be read.
    #[inline]
    pub async fn usage_all(&self) -> BcResult<Vec<Usage>> {
        let defs = self.list().await?;
        let transactions = counts(&self.pool, Owner::Transaction, None).await?;
        let postings = counts(&self.pool, Owner::Posting, None).await?;
        Ok(defs
            .into_iter()
            .map(|def| assemble(def, &transactions, &postings))
            .collect())
    }
}

/// Reads one owner table's per-key `(entries, flagged)` counts.
///
/// # Arguments
///
/// * `pool` - The connection pool to query.
/// * `owner` - Which metadata table to count.
/// * `only` - A single key to restrict the scan to, or `None` for every key.
///
/// # Returns
///
/// A map from key to its entry count and its flagged count. A key with no rows
/// in this table is absent from the map rather than present with zeros.
///
/// # Errors
///
/// Returns [`crate::BcError::Database`] on query failure.
async fn counts(
    pool: &SqlitePool,
    owner: Owner,
    only: Option<&MetaKey>,
) -> BcResult<HashMap<String, (u64, u64)>> {
    let mut sql = format!(
        "SELECT key, COUNT(*), COALESCE(SUM(mismatched), 0) FROM {}",
        owner.table()
    );
    if only.is_some() {
        sql.push_str(" WHERE key = ?");
    }
    sql.push_str(" GROUP BY key");

    let mut query = sqlx::query_as::<_, (String, i64, i64)>(sqlx::AssertSqlSafe(sql));
    if let Some(key) = only {
        query = query.bind(key.as_str());
    }
    let rows = query.fetch_all(pool).await?;

    // COUNT and SUM over a non-negative column never go negative, so the sign
    // is discarded rather than checked.
    Ok(rows
        .into_iter()
        .map(|(key, entries, flagged)| (key, (entries.unsigned_abs(), flagged.unsigned_abs())))
        .collect())
}

/// Joins one registration to the two owner tables' counts.
///
/// # Arguments
///
/// * `def` - The key's registration.
/// * `transactions` - `transaction_metadata` counts, from [`counts`].
/// * `postings` - `posting_metadata` counts, from [`counts`].
fn assemble(
    def: MetaKeyDef,
    transactions: &HashMap<String, (u64, u64)>,
    postings: &HashMap<String, (u64, u64)>,
) -> Usage {
    let name = def.key().as_str();
    let (tx_entries, tx_flagged) = transactions.get(name).copied().unwrap_or((0, 0));
    let (post_entries, post_flagged) = postings.get(name).copied().unwrap_or((0, 0));
    Usage {
        transactions: tx_entries,
        postings: post_entries,
        mismatched: tx_flagged.saturating_add(post_flagged),
        def,
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::MetaEntry;
    use bc_models::MetaType;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::PostingId;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::metadata::insert;

    /// Builds a metadata key, for tests that know their literal is valid.
    fn key(name: &str) -> MetaKey {
        MetaKey::new(name).expect("key should be valid")
    }

    /// Inserts a bare `transactions` row so metadata can reference it.
    async fn seed_transaction(pool: &SqlitePool) -> String {
        let id = TransactionId::new().to_string();
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, '2026-01-15', 'Groceries', 'unreconciled', ?)",
        )
        .bind(&id)
        .bind(Timestamp::now().to_string())
        .execute(pool)
        .await
        .expect("seed transaction");
        id
    }

    /// Inserts a bare `postings` row on `transaction_id`.
    async fn seed_posting(pool: &SqlitePool, transaction_id: &str) -> String {
        let account = crate::AccountService::new(pool.clone())
            .create()
            .name("Cash")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let id = PostingId::new().to_string();
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, '10.00', 'AUD', 0)",
        )
        .bind(&id)
        .bind(transaction_id)
        .bind(account.to_string())
        .execute(pool)
        .await
        .expect("seed posting");
        id
    }

    /// Writes entries through the storage layer, so keys register exactly as
    /// they do in production and `mismatched` is derived rather than asserted.
    async fn write_meta(pool: &SqlitePool, owner: Owner, owner_id: &str, metadata: &Metadata) {
        let mut db_tx = pool.begin().await.expect("begin");
        insert(&mut db_tx, owner, owner_id, metadata)
            .await
            .expect("insert metadata");
        db_tx.commit().await.expect("commit");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn usage_counts_both_owner_tables(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let posting = seed_posting(&pool, &tx).await;
        write_meta(
            &pool,
            Owner::Transaction,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;
        write_meta(
            &pool,
            Owner::Posting,
            &posting,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1503)),
            )]),
        )
        .await;

        let service = Service::new(pool);
        let usage = service
            .usage(&key("invoice"))
            .await
            .expect("usage")
            .expect("registered");

        assert_eq!(usage.transactions(), 1, "one entry sits on the transaction");
        assert_eq!(usage.postings(), 1, "one entry sits on the posting");
        assert_eq!(usage.total(), 2, "total spans both owner tables");
        assert_eq!(usage.mismatched(), 0, "both values fit a number key");
        assert_eq!(usage.def().ty(), MetaType::Number, "usage carries the def");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn usage_counts_mismatched_entries(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_meta(
            &pool,
            Owner::Transaction,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;
        let second = seed_transaction(&pool).await;
        write_meta(
            &pool,
            Owner::Transaction,
            &second,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Text("pending".to_owned()),
            )]),
        )
        .await;

        let service = Service::new(pool);
        let usage = service
            .usage(&key("invoice"))
            .await
            .expect("usage")
            .expect("registered");

        assert_eq!(usage.transactions(), 2, "both entries are counted");
        assert_eq!(
            usage.mismatched(),
            1,
            "'pending' does not read as a number, so the store marks it \
             mismatched"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_registered_key_with_no_entries_counts_zero(pool: SqlitePool) {
        let service = Service::new(pool);
        service
            .register(&key("owner"), MetaType::Account)
            .await
            .expect("register");

        let usage = service
            .usage(&key("owner"))
            .await
            .expect("usage")
            .expect("registered");

        assert_eq!(usage.total(), 0, "a declared key holds nothing yet");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unregistered_key_has_no_usage(pool: SqlitePool) {
        let service = Service::new(pool);

        assert_eq!(
            service.usage(&key("absent")).await.expect("usage"),
            None,
            "None distinguishes an unregistered key from one with zero entries"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn usage_all_covers_every_registered_key(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_meta(
            &pool,
            Owner::Transaction,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;
        let service = Service::new(pool);
        service
            .register(&key("owner"), MetaType::Account)
            .await
            .expect("register");

        let all = service.usage_all().await.expect("usage_all");

        assert_eq!(
            all.iter()
                .map(|usage| (usage.key().as_str().to_owned(), usage.total()))
                .collect::<Vec<_>>(),
            vec![("invoice".to_owned(), 1), ("owner".to_owned(), 0)],
            "every registered key appears, ordered by key, zeros included"
        );
    }
}
