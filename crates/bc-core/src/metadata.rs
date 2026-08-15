//! Persistence for typed key-value metadata on transactions and postings.
//!
//! Every metadata SQL statement lives here; `transaction.rs` and `transfer.rs`
//! call into this module rather than writing the tables themselves.

use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaType;
use bc_models::MetaValue;
use bc_models::Metadata;
use jiff::Timestamp;
use rust_decimal::prelude::ToPrimitive as _;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::transaction::sql_placeholders;

/// Which of the two metadata tables a call addresses.
///
/// Both carry identical columns and differ only in their name and owner column,
/// so every statement below is built from these two literals. The deferred
/// account metadata becomes a third variant and a third table, not a reshape of
/// these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// `transaction_metadata`, owned by `transactions.id`.
    Transaction,
    /// `posting_metadata`, owned by `postings.id`.
    Posting,
}

impl Owner {
    /// Returns the table name.
    const fn table(self) -> &'static str {
        match self {
            Self::Transaction => "transaction_metadata",
            Self::Posting => "posting_metadata",
        }
    }

    /// Returns the owner-id column name.
    const fn id_column(self) -> &'static str {
        match self {
            Self::Transaction => "transaction_id",
            Self::Posting => "posting_id",
        }
    }
}

/// The four value columns a single [`MetaValue`] occupies.
struct ValueColumns {
    /// Canonical string form. Always populated.
    text: String,
    /// Sortable numeric shadow; set for numbers and amounts only.
    num: Option<f64>,
    /// Commodity code; set for amounts only.
    commodity: Option<String>,
    /// Referenced account id; set for account values only.
    account: Option<String>,
}

impl ValueColumns {
    /// Builds the columns of a value stored as flagged text.
    ///
    /// A mismatched value keeps only its canonical string: the typed columns
    /// describe a type the key is not registered as, so populating them would
    /// index the value under a type nothing queries it by.
    fn flagged(value: &MetaValue) -> Self {
        Self {
            text: value.canonical(),
            num: None,
            commodity: None,
            account: None,
        }
    }
}

/// Returns an account's colon-separated path, walking up the parent chain.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to read within.
/// * `id` - The account to resolve.
///
/// # Returns
///
/// The path, or `None` when no account carries `id`.
///
/// # Errors
///
/// Returns [`BcError`] on database read failure.
async fn account_path(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &AccountId,
) -> BcResult<Option<String>> {
    let mut segments: Vec<String> = Vec::new();
    let mut current = id.to_string();
    // An account tree deep enough to exhaust this is a cycle, which the sibling
    // uniqueness index cannot rule out on its own.
    for _step in 0_u16..64 {
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT name, parent_id FROM accounts WHERE id = ?")
                .bind(&current)
                .fetch_optional(&mut **db_tx)
                .await?;
        let Some((name, parent)) = row else {
            return Ok(None);
        };
        segments.push(name);
        let Some(parent_id) = parent else {
            segments.reverse();
            return Ok(Some(segments.join(":")));
        };
        current = parent_id;
    }
    Err(BcError::BadData(format!(
        "account {id} sits in a parent cycle or a tree deeper than 64 levels"
    )))
}

/// Decomposes a value into its storage columns.
///
/// An account value is stored with its resolved path in `value_text` and its id
/// in `value_account`, so a later account deletion leaves a tombstone naming
/// what the entry pointed at. An account that cannot be resolved falls back to
/// the id in both columns.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction, used to resolve account paths.
/// * `value` - The value to decompose.
///
/// # Errors
///
/// Returns [`BcError`] on database read failure.
async fn value_columns(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    value: &MetaValue,
) -> BcResult<ValueColumns> {
    let text = value.canonical();
    let columns = match *value {
        MetaValue::Number(number) => ValueColumns {
            text,
            num: number.to_f64(),
            commodity: None,
            account: None,
        },
        MetaValue::Amount(ref amount) => ValueColumns {
            text,
            num: amount.value().to_f64(),
            commodity: Some(amount.commodity().as_str().to_owned()),
            account: None,
        },
        MetaValue::Account(ref id) => {
            let path = account_path(db_tx, id).await?;
            ValueColumns {
                text: path.unwrap_or(text),
                num: None,
                commodity: None,
                account: Some(id.to_string()),
            }
        }
        MetaValue::Text(_)
        | MetaValue::Boolean(_)
        | MetaValue::Date(_)
        | MetaValue::Timestamp(_) => ValueColumns {
            text,
            num: None,
            commodity: None,
            account: None,
        },
    };
    Ok(columns)
}

/// Registers `key` with `ty` if the registry does not already hold it, and
/// returns the type the registry ends up holding.
///
/// Phase 2 registers and never retypes: a key already present keeps the type
/// its first value gave it, whatever this call asserts. The returned type is
/// what every entry under the key is read back as, so callers compare against
/// it to decide whether a value is mismatched.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `key` - The key to register.
/// * `ty` - The type to register it with, when it is absent.
///
/// # Returns
///
/// The registered type: `ty` for a new key, the existing type otherwise.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the stored type is unreadable, and
/// [`BcError`] on database failure.
pub(crate) async fn register_key_if_absent(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &MetaKey,
    ty: MetaType,
) -> BcResult<MetaType> {
    sqlx::query(
        "INSERT OR IGNORE INTO metadata_keys (key, value_type, created_at) VALUES (?, ?, ?)",
    )
    .bind(key.as_str())
    .bind(to_db_str(ty)?)
    .bind(Timestamp::now().to_string())
    .execute(&mut **db_tx)
    .await?;

    let stored: String = sqlx::query_scalar("SELECT value_type FROM metadata_keys WHERE key = ?")
        .bind(key.as_str())
        .fetch_one(&mut **db_tx)
        .await?;
    from_db_str::<MetaType>(&stored)
}

/// Writes every entry of `metadata` against `owner_id`, registering any key the
/// registry does not hold.
///
/// `position` is the entry's index in `metadata`, so it orders the owner's whole
/// list across keys rather than within a key.
///
/// A value whose type differs from its key's registered type is stored as its
/// canonical string and flagged `mismatched`, never rejected. Rescuing such a
/// value by parsing it into the registered type is phase 3's; this phase flags
/// every one of them.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `owner` - Which metadata table to write.
/// * `owner_id` - The owning transaction or posting id.
/// * `metadata` - The entries to write, in display order.
///
/// # Errors
///
/// Returns [`BcError::BadData`] when the entry count exceeds `i64::MAX`, and
/// [`BcError`] on database failure.
pub(crate) async fn insert(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: Owner,
    owner_id: &str,
    metadata: &Metadata,
) -> BcResult<()> {
    let statement = format!(
        "INSERT INTO {} ({}, key, position, value_text, value_num, value_commodity, \
         value_account, mismatched) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        owner.table(),
        owner.id_column()
    );

    for (index, entry) in metadata.iter().enumerate() {
        let registered = register_key_if_absent(db_tx, entry.key(), entry.value().ty()).await?;
        let mismatched = entry.mismatched() || entry.value().ty() != registered;
        let position = i64::try_from(index)
            .map_err(|_err| BcError::BadData("metadata position exceeds i64::MAX".into()))?;
        let columns = if mismatched {
            ValueColumns::flagged(entry.value())
        } else {
            value_columns(db_tx, entry.value()).await?
        };

        sqlx::query(sqlx::AssertSqlSafe(statement.clone()))
            .bind(owner_id)
            .bind(entry.key().as_str())
            .bind(position)
            .bind(columns.text)
            .bind(columns.num)
            .bind(columns.commodity)
            .bind(columns.account)
            .bind(i64::from(mismatched))
            .execute(&mut **db_tx)
            .await?;
    }
    Ok(())
}

/// Removes every metadata entry belonging to `owner_id`.
///
/// The registry is untouched: a key outlives every entry that used it.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `owner` - Which metadata table to clear.
/// * `owner_id` - The owning transaction or posting id.
///
/// # Errors
///
/// Returns [`BcError`] on database write failure.
pub(crate) async fn delete_for(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: Owner,
    owner_id: &str,
) -> BcResult<()> {
    let statement = format!(
        "DELETE FROM {} WHERE {} = ?",
        owner.table(),
        owner.id_column()
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(owner_id)
        .execute(&mut **db_tx)
        .await?;
    Ok(())
}

/// Removes the posting metadata of every posting belonging to `transaction_id`.
///
/// `posting_metadata.posting_id` is a plain foreign key, so this must run before
/// any statement that deletes those postings.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `transaction_id` - The owning transaction.
///
/// # Errors
///
/// Returns [`BcError`] on database write failure.
pub(crate) async fn delete_for_transaction_postings(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    transaction_id: &str,
) -> BcResult<()> {
    sqlx::query(
        "DELETE FROM posting_metadata WHERE posting_id IN \
         (SELECT id FROM postings WHERE transaction_id = ?)",
    )
    .bind(transaction_id)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

/// Rebuilds one owner's entries, replacing whatever it currently holds.
///
/// Deleting and re-inserting keeps `position` contiguous and zero-based, which
/// a targeted update cannot promise once repeated keys are in play.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `owner` - Which metadata table to rewrite.
/// * `owner_id` - The owning transaction or posting id.
/// * `metadata` - The entries the owner should end up with.
///
/// # Errors
///
/// Returns [`BcError`] on database failure.
pub(crate) async fn replace(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: Owner,
    owner_id: &str,
    metadata: &Metadata,
) -> BcResult<()> {
    delete_for(db_tx, owner, owner_id).await?;
    insert(db_tx, owner, owner_id, metadata).await
}

/// Concatenates `absorbed` onto `survivor`, dropping any absorbed entry exactly
/// equal to one the survivor already holds.
///
/// This is what merging two transactions' metadata means. Repeated keys are
/// legal, so a key present on both sides is simply two entries; only an exact
/// duplicate — same key, same value, same flag — is redundant.
///
/// # Arguments
///
/// * `survivor` - The surviving transaction's entries; their order is kept.
/// * `absorbed` - The absorbed transaction's entries, appended after them.
///
/// # Returns
///
/// The merged list.
#[must_use]
pub(crate) fn union(survivor: &Metadata, absorbed: &Metadata) -> Metadata {
    let original = survivor.entries();
    let mut merged: Vec<MetaEntry> = original.to_vec();
    // Redundancy is judged against the survivor's own entries only. Testing
    // against `merged` would also collapse two identical entries the absorbed
    // leg carries by itself, which the survivor never held.
    merged.extend(
        absorbed
            .iter()
            .filter(|entry| !original.contains(entry))
            .cloned(),
    );
    Metadata::new(merged)
}

/// Removes from `current` the entries a merge appended to the survivor.
///
/// The inverse of [`union`] on the survivor's side. A merge appended exactly
/// those absorbed entries absent from `before`, so dropping one occurrence of
/// each leaves the survivor's own entries alongside every edit made while the
/// two were merged.
///
/// An entry the merge appended and the user then added again by hand is
/// indistinguishable from a single entry counted twice, so one occurrence
/// survives — the reading that keeps the user's work.
///
/// # Arguments
///
/// * `current` - The survivor's entries as they stand now; their order is kept.
/// * `before` - The survivor's entries as they stood before the merge.
/// * `absorbed` - The absorbed transaction's entries at merge time.
///
/// # Returns
///
/// The survivor's entries with the merge's contribution removed.
#[must_use]
pub(crate) fn subtract_merged(
    current: &Metadata,
    before: &Metadata,
    absorbed: &Metadata,
) -> Metadata {
    let original = before.entries();
    let mut remaining: Vec<MetaEntry> = current.entries().to_vec();
    for entry in absorbed.iter().filter(|e| !original.contains(e)) {
        if let Some(position) = remaining.iter().position(|held| held == entry) {
            remaining.remove(position);
        }
    }
    Metadata::new(remaining)
}

/// Loads the metadata of every owner in `owner_ids` in one query.
///
/// Owners with no entries are absent from the returned map, so callers read them
/// with `unwrap_or_default`.
///
/// A stored value is read back through its key's registered type. A flagged
/// value comes back as [`MetaValue::Text`], which is what it was stored as. An
/// account value comes back from `value_account`; when that link is NULL the
/// entry has been tombstoned by an account deletion, so it comes back as the
/// path in `value_text`, flagged — a path is not an id.
///
/// # Arguments
///
/// * `pool` - The connection pool to read through.
/// * `owner` - Which metadata table to read.
/// * `owner_ids` - The owning transaction or posting ids. An empty slice issues
///   no query.
///
/// # Returns
///
/// A map from owner id to its entries, each list in display order.
///
/// # Errors
///
/// Returns [`BcError::BadData`] when a stored key or registered type is
/// unreadable, and [`BcError`] on database read failure.
pub(crate) async fn load_for(
    pool: &SqlitePool,
    owner: Owner,
    owner_ids: &[&str],
) -> BcResult<HashMap<String, Metadata>> {
    if owner_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = sql_placeholders(owner_ids.len());
    let statement = format!(
        "SELECT m.{id_column}, m.key, m.value_text, m.value_account, m.mismatched, k.value_type \
         FROM {table} m JOIN metadata_keys k ON m.key = k.key \
         WHERE m.{id_column} IN ({placeholders}) \
         ORDER BY m.{id_column}, m.position ASC",
        id_column = owner.id_column(),
        table = owner.table(),
    );
    let mut stmt = sqlx::query_as(sqlx::AssertSqlSafe(statement));
    for id in owner_ids {
        stmt = stmt.bind(*id);
    }
    let rows: Vec<(String, String, String, Option<String>, i64, String)> =
        stmt.fetch_all(pool).await?;

    let mut grouped: HashMap<String, Vec<MetaEntry>> = HashMap::new();
    for (owner_id, key_str, value_text, value_account, flag, type_str) in rows {
        let key = MetaKey::new(key_str.clone())
            .map_err(|e| BcError::BadData(format!("invalid metadata key '{key_str}': {e}")))?;
        let ty = from_db_str::<MetaType>(&type_str)?;
        let (value, mismatched) = read_value(ty, value_text, value_account, flag != 0);
        // A flagged row always stores text, so the pair is representable. The
        // error arm is unreachable through `read_value` and exists because
        // `MetaEntry` refuses to hold a flagged non-text value at all.
        let entry = match (mismatched, value) {
            (true, MetaValue::Text(raw)) => MetaEntry::mismatch(key, raw),
            (true, other) => {
                return Err(BcError::BadData(format!(
                    "flagged metadata entry for '{key_str}' holds {:?}, not text",
                    other.ty()
                )));
            }
            (false, fitted) => MetaEntry::new(key, fitted),
        };
        grouped.entry(owner_id).or_default().push(entry);
    }

    Ok(grouped
        .into_iter()
        .map(|(id, entries)| (id, Metadata::new(entries)))
        .collect())
}

/// Reconstructs one stored row's value and mismatch flag.
///
/// An unflagged row that will not parse is corrupt rather than expected — the
/// write path cannot produce one — but it still comes back as flagged text
/// rather than failing the load, so a bad row costs its own fidelity and
/// nothing else.
fn read_value(
    ty: MetaType,
    value_text: String,
    value_account: Option<String>,
    flagged: bool,
) -> (MetaValue, bool) {
    if flagged {
        return (MetaValue::Text(value_text), true);
    }
    if ty == MetaType::Account {
        // `value_text` holds the path, so the id has to come from the link.
        // A cleared link is a tombstone: the account is gone and the path is
        // all that survives.
        return match value_account.and_then(|id| id.parse::<AccountId>().ok()) {
            Some(id) => (MetaValue::Account(id), false),
            None => (MetaValue::Text(value_text), true),
        };
    }
    match ty.parse_value(&value_text) {
        Ok(parsed) => (parsed, false),
        Err(_err) => (MetaValue::Text(value_text), true),
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::TransactionId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

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

    /// Writes one transaction's entries through the module under test.
    async fn write(pool: &SqlitePool, owner_id: &str, metadata: &Metadata) {
        let mut db_tx = pool.begin().await.expect("begin");
        insert(&mut db_tx, Owner::Transaction, owner_id, metadata)
            .await
            .expect("insert metadata");
        db_tx.commit().await.expect("commit");
    }

    /// Reads one transaction's entries back.
    async fn read(pool: &SqlitePool, owner_id: &str) -> Metadata {
        load_for(pool, Owner::Transaction, &[owner_id])
            .await
            .expect("load metadata")
            .remove(owner_id)
            .unwrap_or_default()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn every_value_type_round_trips(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let stamp = Timestamp::from_second(1_700_000_000).expect("valid timestamp");
        let written = Metadata::new(vec![
            MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
            MetaEntry::new(key("invoice"), MetaValue::Number(dec!(1502.50))),
            MetaEntry::new(key("reimbursed"), MetaValue::Boolean(true)),
            MetaEntry::new(key("cleared"), MetaValue::Date(date(2026, 1, 17))),
            MetaEntry::new(key("seen-at"), MetaValue::Timestamp(stamp)),
            MetaEntry::new(
                key("fee"),
                MetaValue::Amount(Amount::new(dec!(1.50), CommodityCode::new("AUD"))),
            ),
        ]);

        write(&pool, &tx, &written).await;

        assert_eq!(read(&pool, &tx).await, written);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repeated_keys_survive_in_insertion_order(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let written = Metadata::new(vec![
            MetaEntry::new(key("note"), MetaValue::Text("first".to_owned())),
            MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
            MetaEntry::new(key("note"), MetaValue::Text("second".to_owned())),
        ]);

        write(&pool, &tx, &written).await;
        let back = read(&pool, &tx).await;

        let keys: Vec<&str> = back.iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, vec!["note", "payee", "note"]);
        assert_eq!(back, written);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn position_is_zero_based_across_all_keys(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("note"), MetaValue::Text("first".to_owned())),
                MetaEntry::new(key("note"), MetaValue::Text("second".to_owned())),
                MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
            ]),
        )
        .await;

        let positions: Vec<(String, i64)> = sqlx::query_as(
            "SELECT key, position FROM transaction_metadata \
             WHERE transaction_id = ? ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("positions");

        assert_eq!(
            positions,
            vec![
                ("note".to_owned(), 0),
                ("note".to_owned(), 1),
                ("payee".to_owned(), 2),
            ],
            "position orders the whole entry list, not each key separately"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_new_key_is_registered_with_its_own_value_type(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502)),
            )]),
        )
        .await;

        let ty: String =
            sqlx::query_scalar("SELECT value_type FROM metadata_keys WHERE key = 'invoice'")
                .fetch_one(&pool)
                .await
                .expect("registered key");
        assert_eq!(ty, "number");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn registration_is_insert_if_absent_and_never_retypes(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("invoice"), MetaValue::Number(dec!(1502))),
                MetaEntry::new(key("invoice"), MetaValue::Text("A-77".to_owned())),
            ]),
        )
        .await;

        let rows: Vec<String> =
            sqlx::query_scalar("SELECT value_type FROM metadata_keys WHERE key = 'invoice'")
                .fetch_all(&pool)
                .await
                .expect("registry rows");
        assert_eq!(
            rows,
            vec!["number".to_owned()],
            "the first write fixes the type; phase 2 neither coerces nor retypes"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_value_of_the_wrong_type_is_stored_flagged(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("invoice"), MetaValue::Number(dec!(1502))),
                MetaEntry::new(key("invoice"), MetaValue::Text("A-77".to_owned())),
            ]),
        )
        .await;

        let stored: Vec<(String, i64, Option<f64>)> = sqlx::query_as(
            "SELECT value_text, mismatched, value_num FROM transaction_metadata \
             WHERE transaction_id = ? ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("rows");
        assert_eq!(
            stored,
            vec![
                ("1502".to_owned(), 0, Some(1502.0_f64)),
                ("A-77".to_owned(), 1, None),
            ],
            "the second value does not fit the key's registered type, so it is \
             stored as its canonical string with no typed columns and flagged"
        );

        let back = read(&pool, &tx).await;
        let read_back: Vec<(&MetaValue, bool)> =
            back.iter().map(|e| (e.value(), e.mismatched())).collect();
        assert_eq!(
            read_back,
            vec![
                (&MetaValue::Number(dec!(1502)), false),
                (&MetaValue::Text("A-77".to_owned()), true),
            ],
            "the second value reads back flagged, so phase 8 can badge it"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_explicitly_mismatched_entry_keeps_its_flag(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        let entry = MetaEntry::mismatch(key("invoice"), "not-a-number");
        write(&pool, &tx, &Metadata::new(vec![entry.clone()])).await;

        assert_eq!(read(&pool, &tx).await, Metadata::new(vec![entry]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_number_writes_a_sortable_shadow(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("invoice"),
                MetaValue::Number(dec!(1502.50)),
            )]),
        )
        .await;

        let (text, num): (String, Option<f64>) = sqlx::query_as(
            "SELECT value_text, value_num FROM transaction_metadata WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(text, "1502.50");
        assert_eq!(num, Some(1502.50_f64));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_amount_writes_its_commodity_and_shadow(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("fee"),
                MetaValue::Amount(Amount::new(dec!(1.50), CommodityCode::new("AUD"))),
            )]),
        )
        .await;

        let (text, num, commodity): (String, Option<f64>, Option<String>) = sqlx::query_as(
            "SELECT value_text, value_num, value_commodity FROM transaction_metadata \
             WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(text, "1.50 AUD");
        assert_eq!(num, Some(1.50_f64));
        assert_eq!(commodity, Some("AUD".to_owned()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_value_stores_its_path_and_a_real_foreign_key(pool: SqlitePool) {
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
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account.clone()),
            )]),
        )
        .await;

        let (text, stored): (String, Option<String>) = sqlx::query_as(
            "SELECT value_text, value_account FROM transaction_metadata WHERE transaction_id = ?",
        )
        .bind(&tx)
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            text, "Assets:Savings",
            "value_text holds the path, which is what survives the account"
        );
        assert_eq!(stored, Some(account.to_string()));

        assert_eq!(
            read(&pool, &tx).await,
            Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account)
            )]),
            "the value comes back from the foreign key, not from the path"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_parent_cycle_is_rejected(pool: SqlitePool) {
        let accounts = crate::AccountService::new(pool.clone());
        let parent = accounts
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create parent");
        let child = accounts
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("create child");

        // Close the loop behind the service's back: no API builds a cycle, but a
        // hand-edited database can hold one and the walk must not spin.
        sqlx::query("UPDATE accounts SET parent_id = ? WHERE id = ?")
            .bind(child.to_string())
            .bind(parent.to_string())
            .execute(&pool)
            .await
            .expect("close the cycle");

        let tx = seed_transaction(&pool).await;
        let mut db_tx = pool.begin().await.expect("begin");
        let result = insert(
            &mut db_tx,
            Owner::Transaction,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(child),
            )]),
        )
        .await;

        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "a cyclic parent chain is bad data, not an infinite walk, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_a_referenced_account_tombstones_the_entry(pool: SqlitePool) {
        let account = crate::AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("offset"),
                MetaValue::Account(account.clone()),
            )]),
        )
        .await;

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(account.to_string())
            .execute(&pool)
            .await
            .expect("the delete is accepted rather than rejected by the foreign key");

        let entries = read(&pool, &tx).await;
        let entry = entries
            .iter()
            .next()
            .expect("the entry survives its account");
        assert_eq!(
            entry.value(),
            &MetaValue::Text("Savings".to_owned()),
            "the cleared link leaves the path behind, so the entry still names \
             what it pointed at"
        );
        assert!(entry.mismatched(), "a path is not an account id");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_for_batches_many_owners_and_omits_the_bare_ones(pool: SqlitePool) {
        let with = seed_transaction(&pool).await;
        let without = seed_transaction(&pool).await;
        write(
            &pool,
            &with,
            &Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
        )
        .await;

        let loaded = load_for(&pool, Owner::Transaction, &[&with, &without])
            .await
            .expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get(&without), None);
        assert_eq!(loaded.get(&with).map(Metadata::len), Some(1));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_for_no_owners_issues_no_query(pool: SqlitePool) {
        let loaded = load_for(&pool, Owner::Transaction, &[])
            .await
            .expect("load");
        assert!(loaded.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_for_removes_every_entry_and_leaves_the_registry(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
        )
        .await;

        let mut db_tx = pool.begin().await.expect("begin");
        delete_for(&mut db_tx, Owner::Transaction, &tx)
            .await
            .expect("delete");
        db_tx.commit().await.expect("commit");

        assert!(read(&pool, &tx).await.is_empty());
        let keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metadata_keys")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(keys, 1, "a key outlives every entry that used it");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_metadata_writes_nothing(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(&pool, &tx, &Metadata::default()).await;
        let keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metadata_keys")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(keys, 0, "an empty metadata list registers no key");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn replace_rewrites_positions_contiguously(pool: SqlitePool) {
        let tx = seed_transaction(&pool).await;
        write(
            &pool,
            &tx,
            &Metadata::new(vec![
                MetaEntry::new(key("note"), MetaValue::Text("a".to_owned())),
                MetaEntry::new(key("note"), MetaValue::Text("b".to_owned())),
                MetaEntry::new(key("note"), MetaValue::Text("c".to_owned())),
            ]),
        )
        .await;

        let mut db_tx = pool.begin().await.expect("begin");
        replace(
            &mut db_tx,
            Owner::Transaction,
            &tx,
            &Metadata::new(vec![MetaEntry::new(
                key("note"),
                MetaValue::Text("only".to_owned()),
            )]),
        )
        .await
        .expect("replace");
        db_tx.commit().await.expect("commit");

        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT value_text, position FROM transaction_metadata \
             WHERE transaction_id = ? ORDER BY position",
        )
        .bind(&tx)
        .fetch_all(&pool)
        .await
        .expect("rows");
        assert_eq!(rows, vec![("only".to_owned(), 0)]);
    }

    #[test]
    fn union_appends_and_drops_only_exact_duplicates() {
        let survivor = Metadata::new(vec![
            MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
            MetaEntry::new(key("note"), MetaValue::Text("shared".to_owned())),
        ]);
        let absorbed = Metadata::new(vec![
            MetaEntry::new(key("note"), MetaValue::Text("shared".to_owned())),
            MetaEntry::new(key("note"), MetaValue::Text("distinct".to_owned())),
            MetaEntry::new(key("payee"), MetaValue::Text("Other Grocer".to_owned())),
        ]);

        let merged = union(&survivor, &absorbed);

        let pairs: Vec<(&str, &MetaValue)> = merged
            .iter()
            .map(|e| (e.key().as_str(), e.value()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("payee", &MetaValue::Text("Generic Grocer".to_owned())),
                ("note", &MetaValue::Text("shared".to_owned())),
                ("note", &MetaValue::Text("distinct".to_owned())),
                ("payee", &MetaValue::Text("Other Grocer".to_owned())),
            ],
            "a repeated key is two entries; only the exact duplicate is dropped"
        );
    }

    #[test]
    fn union_keeps_duplicates_the_absorbed_leg_carries_alone() {
        let survivor = Metadata::new(vec![MetaEntry::new(
            key("payee"),
            MetaValue::Text("Generic Grocer".to_owned()),
        )]);
        let absorbed = Metadata::new(vec![
            MetaEntry::new(key("note"), MetaValue::Text("twice".to_owned())),
            MetaEntry::new(key("note"), MetaValue::Text("twice".to_owned())),
        ]);

        let merged = union(&survivor, &absorbed);

        let pairs: Vec<(&str, &MetaValue)> = merged
            .iter()
            .map(|e| (e.key().as_str(), e.value()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("payee", &MetaValue::Text("Generic Grocer".to_owned())),
                ("note", &MetaValue::Text("twice".to_owned())),
                ("note", &MetaValue::Text("twice".to_owned())),
            ],
            "redundancy is judged against the survivor, so the absorbed leg keeps its own repeats"
        );
    }
}
