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
use crate::metadata::register_key_if_absent;

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
        Ok(Self::builder().key(key).ty(ty).created_at(created_at).build())
    }
}

/// The metadata key registry.
///
/// Re-exported from the crate root as [`crate::MetadataService`].
#[derive(Debug, Clone)]
pub struct Service {
    /// Shared SQLite connection pool.
    pool: SqlitePool,
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
}

#[cfg(test)]
mod tests {
    use bc_models::MetaEntry;
    use bc_models::MetaValue;
    use bc_models::Metadata;
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

    /// Writes entries against a transaction through the storage layer, so keys
    /// register exactly as they do in production.
    async fn write_transaction_meta(pool: &SqlitePool, owner_id: &str, metadata: &Metadata) {
        let mut db_tx = pool.begin().await.expect("begin");
        insert(&mut db_tx, Owner::Transaction, owner_id, metadata)
            .await
            .expect("insert metadata");
        db_tx.commit().await.expect("commit");
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
}
