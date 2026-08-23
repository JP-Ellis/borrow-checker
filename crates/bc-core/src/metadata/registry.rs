//! The global metadata key registry: lookup, registration, retype and rename.
//!
//! Every key is a user key. There are no reserved or built-in keys, `payee`
//! holds no privileged position, and every key can be renamed and retyped. A
//! key enters on first write with the type its first value carried, and
//! outlives every entry that used it.

use bc_models::MetaKey;
use bc_models::MetaKeyDef;
use bc_models::MetaType;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;
use crate::metadata::Owner;
use crate::metadata::ValueColumns;
use crate::metadata::coerce::Coerced;
use crate::metadata::coerce::coerce;
use crate::metadata::read_value;
use crate::metadata::register_key_if_absent;
use crate::metadata::value_columns;

/// One row of `metadata_keys`.
#[derive(sqlx::FromRow)]
struct KeyRow {
    /// The normalised key.
    key: String,
    /// The registered type, as its serde name.
    value_type: String,
    /// RFC 3339 registration timestamp.
    created_at: String,
}

impl TryFrom<KeyRow> for MetaKeyDef {
    type Error = BcError;

    fn try_from(row: KeyRow) -> BcResult<Self> {
        let key = MetaKey::new(row.key.clone())
            .map_err(|e| BcError::BadData(format!("invalid metadata key '{}': {e}", row.key)))?;
        let ty = from_db_str::<MetaType>(&row.value_type)?;
        let created_at = row.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;
        Ok(Self::builder()
            .key(key)
            .ty(ty)
            .created_at(created_at)
            .build())
    }
}

/// The metadata key registry.
///
/// Re-exported from the crate root as [`crate::MetadataService`].
#[derive(Debug, Clone)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the sibling `metadata::usage` module runs its own queries over this pool"
)]
pub struct Service {
    /// Shared SQLite connection pool.
    pub(super) pool: SqlitePool,
}

impl Service {
    /// Creates a registry over the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Lists every registered key, ordered by key.
    ///
    /// # Returns
    ///
    /// Every key in the registry.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure and [`BcError::BadData`]
    /// if a stored row cannot be read.
    #[inline]
    pub async fn list(&self) -> BcResult<Vec<MetaKeyDef>> {
        let rows: Vec<KeyRow> =
            sqlx::query_as("SELECT key, value_type, created_at FROM metadata_keys ORDER BY key")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(MetaKeyDef::try_from).collect()
    }

    /// Reads one key's registration.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to look up.
    ///
    /// # Returns
    ///
    /// `Some(def)` when the key is registered, `None` otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure and [`BcError::BadData`]
    /// if the stored row cannot be read.
    #[inline]
    pub async fn get(&self, key: &MetaKey) -> BcResult<Option<MetaKeyDef>> {
        let row: Option<KeyRow> =
            sqlx::query_as("SELECT key, value_type, created_at FROM metadata_keys WHERE key = ?")
                .bind(key.as_str())
                .fetch_optional(&self.pool)
                .await?;
        row.map(MetaKeyDef::try_from).transpose()
    }

    /// Registers `key` with `ty` when the registry does not already hold it.
    ///
    /// A key already present keeps the type its first value gave it, whatever
    /// this call asserts; changing it is [`Service::retype`]'s.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to register.
    /// * `ty` - The type to register it with, when it is absent.
    ///
    /// # Returns
    ///
    /// The type the registry ends up holding.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on failure and [`BcError::BadData`] if the
    /// stored type is unreadable.
    #[inline]
    pub async fn register(&self, key: &MetaKey, ty: MetaType) -> BcResult<MetaType> {
        let mut db_tx = self.pool.begin().await?;
        let registered = register_key_if_absent(&mut db_tx, key, ty).await?;
        db_tx.commit().await?;
        Ok(registered)
    }

    /// Changes `key`'s registered type and replays coercion over every entry
    /// stored under it, in both owner tables.
    ///
    /// A stored row is re-asserted with the type it currently reads back as —
    /// a flagged row reads back as text — and then fitted to `to`. Widening to
    /// [`MetaType::Text`] is a pure relabel: every row keeps its stored string,
    /// sheds its typed columns, and clears its flag, because a text key cannot
    /// mismatch. Narrowing parses each row and flags whatever will not.
    ///
    /// # Arguments
    ///
    /// * `key` - The registered key to retype.
    /// * `to` - The type it should hold from now on.
    ///
    /// # Returns
    ///
    /// The type the key held before this call.
    ///
    /// # Events
    ///
    /// Appends [`Event::MetadataKeyRetyped`] in the same transaction as the
    /// refit, so the type change and the entries it rewrote commit or roll
    /// back together. A retype to the type already held appends nothing.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] when `key` is not registered, and
    /// [`BcError`] on database failure.
    #[inline]
    pub async fn retype(&self, key: &MetaKey, to: MetaType) -> BcResult<MetaType> {
        let mut db_tx = self.pool.begin().await?;
        let registered: Option<String> =
            sqlx::query_scalar("SELECT value_type FROM metadata_keys WHERE key = ?")
                .bind(key.as_str())
                .fetch_optional(&mut *db_tx)
                .await?;
        let Some(stored) = registered else {
            return Err(BcError::NotFound(format!("metadata key '{key}'")));
        };
        let from = from_db_str::<MetaType>(&stored)?;
        if from == to {
            return Ok(from);
        }

        sqlx::query("UPDATE metadata_keys SET value_type = ? WHERE key = ?")
            .bind(to_db_str(to)?)
            .bind(key.as_str())
            .execute(&mut *db_tx)
            .await?;
        for owner in [Owner::Transaction, Owner::Posting] {
            replay(&mut db_tx, owner, key, from, to).await?;
        }
        insert_event(
            &Event::MetadataKeyRetyped {
                key: key.clone(),
                from,
                to,
            },
            &mut db_tx,
        )
        .await?;
        db_tx.commit().await?;
        Ok(from)
    }

    /// Renames `from` to `to`, carrying every entry under it across.
    ///
    /// The renamed key keeps its registered type and its original registration
    /// time: a rename is the same key under a new name, not a fresh
    /// registration. Renaming a key onto one that already exists is rejected
    /// rather than merged — a merge would lose which entries came from which
    /// key, and the source key's type with them, and no event could record it
    /// faithfully enough to replay.
    ///
    /// # Arguments
    ///
    /// * `from` - The registered key to rename.
    /// * `to` - Its new name.
    ///
    /// # Events
    ///
    /// Appends [`Event::MetadataKeyRenamed`] in the same transaction as the
    /// repoint. A rename to the same name, a rename onto a registered key, and
    /// a rename of an unregistered key all append nothing.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] when `from` is not registered,
    /// [`BcError::InvalidInput`] when `to` already is, and [`BcError`] on
    /// database failure.
    #[inline]
    pub async fn rename(&self, from: &MetaKey, to: &MetaKey) -> BcResult<()> {
        if from == to {
            return Ok(());
        }
        let mut db_tx = self.pool.begin().await?;
        let source: Option<(String, String)> =
            sqlx::query_as("SELECT value_type, created_at FROM metadata_keys WHERE key = ?")
                .bind(from.as_str())
                .fetch_optional(&mut *db_tx)
                .await?;
        let Some((value_type, created_at)) = source else {
            return Err(BcError::NotFound(format!("metadata key '{from}'")));
        };
        let taken: Option<String> =
            sqlx::query_scalar("SELECT key FROM metadata_keys WHERE key = ?")
                .bind(to.as_str())
                .fetch_optional(&mut *db_tx)
                .await?;
        if taken.is_some() {
            return Err(BcError::InvalidInput(format!(
                "cannot rename metadata key '{from}' to '{to}': '{to}' is already registered"
            )));
        }

        // Both metadata tables reference `metadata_keys(key)` with no
        // ON UPDATE CASCADE, and every connection sets PRAGMA foreign_keys =
        // ON. The destination must therefore exist before any entry points at
        // it, and the source must outlive the last entry that did.
        sqlx::query("INSERT INTO metadata_keys (key, value_type, created_at) VALUES (?, ?, ?)")
            .bind(to.as_str())
            .bind(&value_type)
            .bind(&created_at)
            .execute(&mut *db_tx)
            .await?;
        for owner in [Owner::Transaction, Owner::Posting] {
            let repoint = format!("UPDATE {} SET key = ? WHERE key = ?", owner.table());
            sqlx::query(sqlx::AssertSqlSafe(repoint))
                .bind(to.as_str())
                .bind(from.as_str())
                .execute(&mut *db_tx)
                .await?;
        }
        sqlx::query("DELETE FROM metadata_keys WHERE key = ?")
            .bind(from.as_str())
            .execute(&mut *db_tx)
            .await?;
        insert_event(
            &Event::MetadataKeyRenamed {
                from: from.clone(),
                to: to.clone(),
            },
            &mut db_tx,
        )
        .await?;

        db_tx.commit().await?;
        Ok(())
    }
}

/// Refits every entry under `key` in one owner table from `from` to `to`.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `owner` - Which metadata table to replay.
/// * `key` - The key whose entries are refitted.
/// * `from` - The type the entries currently read back as.
/// * `to` - The type they should read back as.
///
/// # Errors
///
/// Returns [`BcError`] on database failure.
async fn replay(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: Owner,
    key: &MetaKey,
    from: MetaType,
    to: MetaType,
) -> BcResult<()> {
    if to == MetaType::Text {
        // A pure relabel. `value_text` already holds the canonical string of
        // every row — including the resolved path of an account value, which
        // recomputing from the value would replace with a bare id — so the row
        // keeps it and only sheds what a text key has no use for.
        let relabel = format!(
            "UPDATE {} SET value_num = NULL, value_commodity = NULL, \
             value_account = NULL, mismatched = 0 WHERE key = ?",
            owner.table()
        );
        sqlx::query(sqlx::AssertSqlSafe(relabel))
            .bind(key.as_str())
            .execute(&mut **db_tx)
            .await?;
        return Ok(());
    }

    let select = format!(
        "SELECT rowid, value_text, value_account, mismatched FROM {} WHERE key = ?",
        owner.table()
    );
    // Fetched whole rather than streamed: `value_columns` borrows the
    // transaction mutably inside the loop below.
    let rows: Vec<(i64, String, Option<String>, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(select))
        .bind(key.as_str())
        .fetch_all(&mut **db_tx)
        .await?;

    let update = format!(
        "UPDATE {} SET value_text = ?, value_num = ?, value_commodity = ?, \
         value_account = ?, mismatched = ? WHERE rowid = ?",
        owner.table()
    );
    for (rowid, stored_text, stored_account, flag) in rows {
        // A flagged row reads back as text whatever `from` says, which is
        // exactly the asserted-text case of the coercion rule. Its old flag is
        // discarded, as on the write path: `mismatched` is derived from the
        // value against the registered type, never carried forward.
        let (current, _was_flagged) =
            read_value(from, stored_text.clone(), stored_account, flag != 0);
        let (columns, mismatched) = match coerce(&current, to) {
            Coerced::Fits(ref fitted) => (value_columns(db_tx, fitted).await?, 0_i64),
            // A row that will not narrow keeps the exact string it stored,
            // rather than the value's canonical form — again so an account
            // path is not replaced with an id.
            Coerced::Mismatch => (
                ValueColumns {
                    text: stored_text,
                    num: None,
                    commodity: None,
                    account: None,
                },
                1_i64,
            ),
        };
        sqlx::query(sqlx::AssertSqlSafe(update.clone()))
            .bind(columns.text)
            .bind(columns.num)
            .bind(columns.commodity)
            .bind(columns.account)
            .bind(mismatched)
            .bind(rowid)
            .execute(&mut **db_tx)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::MetaEntry;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::PostingId;
    use bc_models::TransactionId;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::metadata::Owner;
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

    /// Inserts a bare `postings` row on `transaction_id` so posting metadata
    /// can reference it.
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

    /// Writes entries against a transaction through the storage layer, so keys
    /// register exactly as they do in production.
    async fn write_transaction_meta(pool: &SqlitePool, owner_id: &str, metadata: &Metadata) {
        let mut db_tx = pool.begin().await.expect("begin");
        insert(&mut db_tx, Owner::Transaction, owner_id, metadata)
            .await
            .expect("insert metadata");
        db_tx.commit().await.expect("commit");
    }

    /// Writes entries against a posting through the storage layer.
    async fn write_posting_meta(pool: &SqlitePool, owner_id: &str, metadata: &Metadata) {
        let mut db_tx = pool.begin().await.expect("begin");
        insert(&mut db_tx, Owner::Posting, owner_id, metadata)
            .await
            .expect("insert metadata");
        db_tx.commit().await.expect("commit");
    }

    /// Reads every event of one kind out of the log, oldest first.
    async fn payloads_of_kind(pool: &SqlitePool, kind: &str) -> Vec<crate::events::Event> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM events WHERE kind = ? ORDER BY rowid ASC")
                .bind(kind)
                .fetch_all(pool)
                .await
                .expect("payloads");
        rows.iter()
            .map(|payload| serde_json::from_str(payload).expect("deserialise"))
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn writing_a_new_key_records_its_registration(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;

        assert_eq!(
            payloads_of_kind(&pool, "MetadataKeyRegistered").await,
            vec![crate::events::Event::MetadataKeyRegistered {
                key: key("invoice"),
                ty: MetaType::Number,
            }],
            "auto-registration on the write path is what puts most keys in the \
             registry, so it is what the log has to record"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn writing_a_key_a_second_time_records_nothing(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let entries = Metadata::new(vec![MetaEntry::new(
            key("invoice"),
            MetaValue::Number(dec!(1502)),
        )]);
        write_transaction_meta(&pool, &tx, &entries).await;
        write_transaction_meta(&pool, &tx, &entries).await;

        assert_eq!(
            payloads_of_kind(&pool, "MetadataKeyRegistered").await.len(),
            1,
            "an INSERT OR IGNORE that ignored is not a registration"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn registration_records_the_type_the_registry_kept(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");
        registry
            .register(&key("invoice"), MetaType::Text)
            .await
            .expect("register");

        assert_eq!(
            payloads_of_kind(&pool, "MetadataKeyRegistered").await,
            vec![crate::events::Event::MetadataKeyRegistered {
                key: key("invoice"),
                ty: MetaType::Number,
            }],
            "the second call asserts text and the registry keeps number, so \
             there is nothing new to record"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_registration_event_aggregates_on_the_key(pool: SqlitePool) {
        Service::new(pool.clone())
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");

        let aggregate: String = sqlx::query_scalar(
            "SELECT aggregate_id FROM events WHERE kind = 'MetadataKeyRegistered'",
        )
        .fetch_one(&pool)
        .await
        .expect("aggregate");
        assert_eq!(aggregate, "invoice");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_returns_every_key_in_key_order(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
                MetaEntry::new(key("invoice"), MetaValue::Number(dec!(1502))),
                MetaEntry::new(key("cleared"), MetaValue::Boolean(true)),
            ]),
        )
        .await;

        let listed = Service::new(pool).list().await.expect("list");
        let pairs: Vec<(&str, MetaType)> = listed
            .iter()
            .map(|def| (def.key().as_str(), def.ty()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("cleared", MetaType::Boolean),
                ("invoice", MetaType::Number),
                ("payee", MetaType::Text),
            ],
            "keys list alphabetically, not in registration order"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_is_empty_before_anything_is_written(pool: SqlitePool) {
        assert_eq!(Service::new(pool).list().await.expect("list"), vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_reads_one_key_and_returns_none_for_an_unregistered_one(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;

        let registry = Service::new(pool);
        let found = registry.get(&key("invoice")).await.expect("get");
        assert_eq!(found.map(|def| def.ty()), Some(MetaType::Number));
        assert!(registry.get(&key("absent")).await.expect("get").is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn register_is_insert_if_absent(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        assert_eq!(
            registry
                .register(&key("invoice"), MetaType::Number)
                .await
                .expect("register"),
            MetaType::Number
        );
        assert_eq!(
            registry
                .register(&key("invoice"), MetaType::Text)
                .await
                .expect("register"),
            MetaType::Number,
            "a registered key keeps the type its first value gave it"
        );

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metadata_keys")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_registered_key_carries_its_creation_time(pool: SqlitePool) {
        let before = Timestamp::now();
        let registry = Service::new(pool);
        registry
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");
        let def = registry
            .get(&key("invoice"))
            .await
            .expect("get")
            .expect("registered");
        assert!(
            *def.created_at() >= before,
            "created_at is stamped at registration"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_returns_the_previous_type(pool: SqlitePool) {
        let registry = Service::new(pool);
        registry
            .register(&key("invoice"), MetaType::Text)
            .await
            .expect("register");
        assert_eq!(
            registry
                .retype(&key("invoice"), MetaType::Number)
                .await
                .expect("retype"),
            MetaType::Text,
            "the previous type is what phase 4's event records as `from`"
        );
        assert_eq!(
            registry
                .get(&key("invoice"))
                .await
                .expect("get")
                .map(|def| def.ty()),
            Some(MetaType::Number)
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_the_same_type_is_a_no_op(pool: SqlitePool) {
        let registry = Service::new(pool);
        registry
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");
        assert_eq!(
            registry
                .retype(&key("invoice"), MetaType::Number)
                .await
                .expect("retype"),
            MetaType::Number
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_an_unregistered_key_is_not_found(pool: SqlitePool) {
        let outcome = Service::new(pool)
            .retype(&key("absent"), MetaType::Number)
            .await;
        assert!(
            matches!(outcome, Err(BcError::NotFound(ref what)) if what.contains("absent")),
            "retyping a key that was never registered names it"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_records_both_types(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("invoice"), MetaType::Text)
            .await
            .expect("register");
        registry
            .retype(&key("invoice"), MetaType::Number)
            .await
            .expect("retype");

        assert_eq!(
            payloads_of_kind(&pool, "MetadataKeyRetyped").await,
            vec![crate::events::Event::MetadataKeyRetyped {
                key: key("invoice"),
                from: MetaType::Text,
                to: MetaType::Number,
            }]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_the_same_type_records_nothing(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");
        registry
            .retype(&key("invoice"), MetaType::Number)
            .await
            .expect("retype");

        assert_eq!(payloads_of_kind(&pool, "MetadataKeyRetyped").await, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_retype_records_nothing(pool: SqlitePool) {
        let outcome = Service::new(pool.clone())
            .retype(&key("absent"), MetaType::Number)
            .await;
        assert!(matches!(outcome, Err(BcError::NotFound(_))));

        assert_eq!(payloads_of_kind(&pool, "MetadataKeyRetyped").await, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_narrowing_flags_what_will_not_parse(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("invoice"), MetaValue::Text("1502".to_owned())),
                MetaEntry::new(key("invoice"), MetaValue::Text("A-77".to_owned())),
            ]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("invoice"), MetaType::Number)
            .await
            .expect("retype");

        let rows: Vec<(String, Option<f64>, i64)> = sqlx::query_as(
            "SELECT value_text, value_num, mismatched FROM transaction_metadata \
             WHERE transaction_id = ? ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("rows");
        assert_eq!(
            rows,
            vec![
                ("1502".to_owned(), Some(1502.0_f64), 0),
                ("A-77".to_owned(), None, 1),
            ],
            "narrowing parses what it can and flags the rest, and a rescued \
             row earns its sortable shadow"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_narrowing_rescues_a_previously_flagged_entry(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        // `invoice` registers as number, so "1502-B" is flagged on write.
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("invoice"), MetaValue::Number(dec!(1))),
                MetaEntry::new(key("invoice"), MetaValue::Text("1502-B".to_owned())),
            ]),
        )
        .await;
        let flags: Vec<i64> = sqlx::query_scalar(
            "SELECT mismatched FROM transaction_metadata WHERE transaction_id = ? \
             ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("flags");
        assert_eq!(flags, vec![0, 1], "the second entry starts out flagged");

        Service::new(pool.clone())
            .retype(&key("invoice"), MetaType::Text)
            .await
            .expect("retype");

        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT value_text, mismatched FROM transaction_metadata \
             WHERE transaction_id = ? ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("rows");
        assert_eq!(
            rows,
            vec![("1".to_owned(), 0), ("1502-B".to_owned(), 0)],
            "widening to text is a pure relabel: a text key cannot mismatch, \
             so every flag clears"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_text_drops_the_typed_columns(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("fee"),
                MetaValue::Amount(Amount::new(dec!(1.50), CommodityCode::new("AUD"))),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("fee"), MetaType::Text)
            .await
            .expect("retype");

        let row: (String, Option<f64>, Option<String>, Option<String>, i64) = sqlx::query_as(
            "SELECT value_text, value_num, value_commodity, value_account, mismatched \
             FROM transaction_metadata WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("1.50 AUD".to_owned(), None, None, None, 0),
            "the canonical string is what a text key holds; the typed columns \
             would index it under a type nothing queries it by"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_text_keeps_an_account_path(pool: SqlitePool) {
        let accounts = crate::AccountService::new(pool.clone());
        let parent = accounts
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create parent");
        let account = accounts
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("create account");

        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("offset"), MetaType::Text)
            .await
            .expect("retype");

        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT value_text, value_account FROM transaction_metadata WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("Assets:Savings".to_owned(), None),
            "the relabel leaves value_text alone, so the path survives rather \
             than being rewritten as a bare id"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_replays_posting_metadata_too(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let posting = seed_posting(&pool, &tx).await;
        write_posting_meta(
            &pool,
            &posting,
            &Metadata::new(vec![MetaEntry::new(
                key("weight"),
                MetaValue::Text("2.5".to_owned()),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("weight"), MetaType::Number)
            .await
            .expect("retype");

        let row: (String, Option<f64>, i64) = sqlx::query_as(
            "SELECT value_text, value_num, mismatched FROM posting_metadata WHERE posting_id = ?",
        )
        .bind(&posting)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("2.5".to_owned(), Some(2.5_f64), 0),
            "both owner tables are replayed, not just transactions"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_away_from_account_clears_the_link_and_keeps_the_path(pool: SqlitePool) {
        let account = crate::AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("offset"), MetaType::Number)
            .await
            .expect("retype");

        let row: (String, Option<String>, i64) = sqlx::query_as(
            "SELECT value_text, value_account, mismatched FROM transaction_metadata \
             WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("Savings".to_owned(), None, 1),
            "an account does not narrow to a number, so the row is flagged, \
             keeps the text it stored, and loses its link for good"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_account_rescues_a_bare_id(pool: SqlitePool) {
        let account = crate::AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Text(account.to_string()),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .retype(&key("offset"), MetaType::Account)
            .await
            .expect("retype");

        let row: (String, Option<String>, i64) = sqlx::query_as(
            "SELECT value_text, value_account, mismatched FROM transaction_metadata \
             WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("Savings".to_owned(), Some(account.to_string()), 0),
            "text holding a bare id narrows to an account, gains the link, and \
             swaps the id for the resolved path"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retype_to_account_tombstones_an_id_naming_no_account(pool: SqlitePool) {
        let absent = AccountId::new();
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Text(absent.to_string()),
            )]),
        )
        .await;

        // `coerce` reads no database, so a well-formed id fits whether or not
        // an account carries it. Binding it into `value_account` would trip
        // the foreign key and abort every other row's replay with it.
        Service::new(pool.clone())
            .retype(&key("offset"), MetaType::Account)
            .await
            .expect("retype does not fail on an id naming no account");

        let row: (String, Option<String>, i64) = sqlx::query_as(
            "SELECT value_text, value_account, mismatched FROM transaction_metadata \
             WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            (absent.to_string(), None, 0),
            "the row keeps the id it stored and carries no link, which is the \
             same tombstone a deleted account leaves behind"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_to_text_to_account_does_not_round_trip(pool: SqlitePool) {
        let account = crate::AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account),
            )]),
        )
        .await;

        let registry = Service::new(pool.clone());
        registry
            .retype(&key("offset"), MetaType::Text)
            .await
            .expect("retype to text");
        registry
            .retype(&key("offset"), MetaType::Account)
            .await
            .expect("retype back to account");

        let row: (String, Option<String>, i64) = sqlx::query_as(
            "SELECT value_text, value_account, mismatched FROM transaction_metadata \
             WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row,
            ("Savings".to_owned(), None, 1),
            "the trip through text leaves a path where the id was, and a path \
             does not parse as an id, so the return trip flags the entry"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_moves_every_entry_and_retires_the_old_key(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
                MetaEntry::new(key("payee"), MetaValue::Text("Other Grocer".to_owned())),
                MetaEntry::new(key("note"), MetaValue::Text("weekly shop".to_owned())),
            ]),
        )
        .await;

        Service::new(pool.clone())
            .rename(&key("payee"), &key("counterparty"))
            .await
            .expect("rename");

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT key, value_text FROM transaction_metadata WHERE transaction_id = ? \
             ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("rows");
        assert_eq!(
            rows,
            vec![
                ("counterparty".to_owned(), "Generic Grocer".to_owned()),
                ("counterparty".to_owned(), "Other Grocer".to_owned()),
                ("note".to_owned(), "weekly shop".to_owned()),
            ],
            "every entry moves, repeats included, and other keys are untouched"
        );

        let registered: Vec<String> =
            sqlx::query_scalar("SELECT key FROM metadata_keys ORDER BY key")
                .fetch_all(&pool)
                .await
                .expect("keys");
        assert_eq!(
            registered,
            vec!["counterparty".to_owned(), "note".to_owned()]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_records_both_names(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("payee"), MetaType::Text)
            .await
            .expect("register");
        registry
            .rename(&key("payee"), &key("counterparty"))
            .await
            .expect("rename");

        assert_eq!(
            payloads_of_kind(&pool, "MetadataKeyRenamed").await,
            vec![crate::events::Event::MetadataKeyRenamed {
                from: key("payee"),
                to: key("counterparty"),
            }]
        );

        let aggregate: String =
            sqlx::query_scalar("SELECT aggregate_id FROM events WHERE kind = 'MetadataKeyRenamed'")
                .fetch_one(&pool)
                .await
                .expect("aggregate");
        assert_eq!(
            aggregate, "payee",
            "the rename is the last event of the old name's aggregate, so a \
             reader starting from the new name follows the chain backwards"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_rename_that_changes_nothing_records_nothing(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("payee"), MetaType::Text)
            .await
            .expect("register");
        registry
            .register(&key("counterparty"), MetaType::Number)
            .await
            .expect("register");

        registry
            .rename(&key("payee"), &key("payee"))
            .await
            .expect("renaming a key to its own name is accepted");
        let collision = registry.rename(&key("payee"), &key("counterparty")).await;
        assert!(matches!(collision, Err(BcError::InvalidInput(_))));
        let missing = registry.rename(&key("absent"), &key("fresh")).await;
        assert!(matches!(missing, Err(BcError::NotFound(_))));

        assert_eq!(payloads_of_kind(&pool, "MetadataKeyRenamed").await, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_registry_replays_out_of_an_empty_log(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
                MetaEntry::new(key("invoice"), MetaValue::Text("1502".to_owned())),
                MetaEntry::new(key("cleared"), MetaValue::Boolean(true)),
            ]),
        )
        .await;
        let registry = Service::new(pool.clone());
        registry
            .retype(&key("invoice"), MetaType::Number)
            .await
            .expect("retype");
        registry
            .rename(&key("payee"), &key("counterparty"))
            .await
            .expect("rename");

        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT payload FROM events WHERE kind LIKE 'MetadataKey%' ORDER BY rowid ASC",
        )
        .fetch_all(&pool)
        .await
        .expect("registry events");
        let mut replayed: std::collections::BTreeMap<MetaKey, MetaType> =
            std::collections::BTreeMap::new();
        for payload in rows {
            let event: crate::events::Event = serde_json::from_str(&payload).expect("deserialise");
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "Event is #[non_exhaustive]; only the registry variants are replayed here"
            )]
            match event {
                crate::events::Event::MetadataKeyRegistered { key: name, ty } => {
                    replayed.insert(name, ty);
                }
                crate::events::Event::MetadataKeyRetyped { key: name, to, .. } => {
                    replayed.insert(name, to);
                }
                crate::events::Event::MetadataKeyRenamed { from, to } => {
                    if let Some(ty) = replayed.remove(&from) {
                        replayed.insert(to, ty);
                    }
                }
                other => panic!("unexpected registry event {other:?}"),
            }
        }

        let stored: std::collections::BTreeMap<MetaKey, MetaType> = registry
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|def| (def.key().clone(), def.ty()))
            .collect();
        assert_eq!(
            replayed, stored,
            "the three registry events, folded over an empty registry, \
             reconstruct exactly what the projection holds"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_leaves_no_dangling_foreign_key(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write_transaction_meta(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .rename(&key("payee"), &key("counterparty"))
            .await
            .expect("rename");

        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("foreign_key_check");
        assert!(
            violations.is_empty(),
            "the destination key exists before any entry points at it, and the \
             source outlives the last entry that did"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_keeps_the_registered_type_and_creation_time(pool: SqlitePool) {
        let registry = Service::new(pool);
        registry
            .register(&key("invoice"), MetaType::Number)
            .await
            .expect("register");
        let before = registry
            .get(&key("invoice"))
            .await
            .expect("get")
            .expect("registered");

        registry
            .rename(&key("invoice"), &key("bill-number"))
            .await
            .expect("rename");

        let after = registry
            .get(&key("bill-number"))
            .await
            .expect("get")
            .expect("registered");
        assert_eq!(after.ty(), MetaType::Number);
        assert_eq!(
            after.created_at(),
            before.created_at(),
            "a rename is the same key under a new name, not a fresh registration"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_renames_posting_entries_too(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let posting = seed_posting(&pool, &tx).await;
        write_posting_meta(
            &pool,
            &posting,
            &Metadata::new(vec![MetaEntry::new(
                key("note"),
                MetaValue::Text("new medication".to_owned()),
            )]),
        )
        .await;

        Service::new(pool.clone())
            .rename(&key("note"), &key("memo"))
            .await
            .expect("rename");

        let stored: String =
            sqlx::query_scalar("SELECT key FROM posting_metadata WHERE posting_id = ?")
                .bind(&posting)
                .fetch_one(&pool)
                .await
                .expect("row");
        assert_eq!(stored, "memo");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_onto_an_existing_key_is_rejected(pool: SqlitePool) {
        let registry = Service::new(pool.clone());
        registry
            .register(&key("payee"), MetaType::Text)
            .await
            .expect("register");
        registry
            .register(&key("counterparty"), MetaType::Number)
            .await
            .expect("register");

        let outcome = registry.rename(&key("payee"), &key("counterparty")).await;
        assert!(
            matches!(outcome, Err(BcError::InvalidInput(ref why)) if why.contains("counterparty")),
            "a rename that would merge two keys names the collision instead"
        );

        let registered: Vec<String> =
            sqlx::query_scalar("SELECT key FROM metadata_keys ORDER BY key")
                .fetch_all(&pool)
                .await
                .expect("keys");
        assert_eq!(
            registered,
            vec!["counterparty".to_owned(), "payee".to_owned()],
            "the rejected rename changes nothing"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_to_itself_is_a_no_op(pool: SqlitePool) {
        let registry = Service::new(pool);
        registry
            .register(&key("payee"), MetaType::Text)
            .await
            .expect("register");
        registry
            .rename(&key("payee"), &key("payee"))
            .await
            .expect("renaming a key to its own name is accepted and does nothing");
        assert_eq!(registry.list().await.expect("list").len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_an_unregistered_key_is_not_found(pool: SqlitePool) {
        let outcome = Service::new(pool)
            .rename(&key("absent"), &key("present"))
            .await;
        assert!(matches!(outcome, Err(BcError::NotFound(ref what)) if what.contains("absent")));
    }
}
