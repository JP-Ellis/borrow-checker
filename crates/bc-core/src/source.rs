//! Source reference (import provenance) persistence service and import planner.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::PostingId;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::TransactionId;
use jiff::Timestamp;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::BcResult;
use crate::events::insert_event;

/// Raw `transaction_sources` row tuple, mirroring the `SELECT` column list.
type SourceRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    String,
);

/// A stored source reference, reduced to what import matching needs.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLeg {
    /// The occurrence ordinal this reference occupies.
    pub occurrence: u32,
    /// The transaction that owns the posting this reference points at.
    pub transaction_id: TransactionId,
}

/// Persists and queries [`SourceRef`] import-provenance records.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new source-reference service.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool.
    #[inline]
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Attaches a source reference to its transaction.
    ///
    /// Convenience wrapper around [`Service::attach_in_tx`] that owns a
    /// single-purpose database transaction.
    ///
    /// # Arguments
    ///
    /// * `source` - The fully-built [`SourceRef`] to persist.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `source.posting_id()` is not a
    /// posting of the target transaction on `source.account_id()`. Returns
    /// [`crate::BcError`] on event or row insert failure (including a `UNIQUE`
    /// violation if this `(account, fingerprint, occurrence)` already exists).
    #[inline]
    pub async fn attach(&self, source: &SourceRef) -> BcResult<()> {
        let mut db_tx = self.pool.begin().await?;
        self.attach_in_tx(&mut db_tx, source).await?;
        db_tx.commit().await?;
        Ok(())
    }

    /// Attaches a source reference within an already-open database transaction.
    ///
    /// Validates that the reference's account is one of the target transaction's
    /// posting accounts, appends a [`crate::Event::TransactionSourceAttached`],
    /// and inserts the projection row — all using `db_tx`, so an importer can
    /// bundle the source attach with the transaction create into one atomic
    /// unit. The caller owns `db_tx` and must commit it; nothing is durable until
    /// then.
    ///
    /// # Arguments
    ///
    /// * `db_tx` - An open SQLite transaction to write within.
    /// * `source` - The fully-built [`SourceRef`] to persist.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `source.posting_id()` is not a
    /// posting of the target transaction on `source.account_id()`. Returns
    /// [`crate::BcError`] on event or row insert failure (including a `UNIQUE`
    /// violation if this `(account, fingerprint, occurrence)` already exists).
    pub(crate) async fn attach_in_tx(
        &self,
        db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        source: &SourceRef,
    ) -> BcResult<()> {
        // Provenance must point at a posting that belongs to the named
        // transaction and account, or dedup would be scoped to the wrong leg.
        let is_matching_posting: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM postings \
             WHERE id = ? AND transaction_id = ? AND account_id = ?)",
        )
        .bind(source.posting_id().to_string())
        .bind(source.transaction_id().to_string())
        .bind(source.account_id().to_string())
        .fetch_one(&mut **db_tx)
        .await?;
        if is_matching_posting == 0 {
            return Err(crate::BcError::InvalidInput(format!(
                "source posting {} is not a posting of transaction {} on account {}",
                source.posting_id(),
                source.transaction_id(),
                source.account_id()
            )));
        }

        let fingerprint = source.fingerprint();
        let event = crate::Event::TransactionSourceAttached {
            id: source.id().clone(),
            transaction_id: source.transaction_id().clone(),
            posting_id: source.posting_id().clone(),
            account_id: source.account_id().clone(),
            date: source.date(),
            narration: source.narration().to_owned(),
            amount: source.amount().cloned(),
            reference: source.reference().map(str::to_owned),
            occurrence: source.occurrence(),
        };

        insert_event(&event, db_tx).await?;

        sqlx::query(
            "INSERT INTO transaction_sources \
             (id, transaction_id, posting_id, account_id, date, narration, amount, commodity, \
              reference, occurrence, fingerprint, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(source.id().to_string())
        .bind(source.transaction_id().to_string())
        .bind(source.posting_id().to_string())
        .bind(source.account_id().to_string())
        .bind(source.date().to_string())
        .bind(source.narration())
        .bind(source.amount().map(|a| a.value().to_string()))
        .bind(source.amount().map(|a| a.commodity().as_str().to_owned()))
        .bind(source.reference())
        .bind(i64::from(source.occurrence()))
        .bind(&fingerprint)
        .bind(source.created_at().to_string())
        .execute(&mut **db_tx)
        .await?;

        Ok(())
    }

    /// Returns, per fingerprint, the set of occurrence ordinals already stored
    /// for an account.
    ///
    /// The import planner consumes this as an existence check: a row is skipped
    /// only when its exact `(fingerprint, occurrence)` slot is already present.
    /// Keying on the stored occurrence rather than a dense count means detaching
    /// an arbitrary reference can never desynchronise a later re-import.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to load occurrences for.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query failure or a malformed stored occurrence.
    #[inline]
    pub async fn existing_occurrences(
        &self,
        account_id: &AccountId,
    ) -> BcResult<HashMap<String, HashSet<u32>>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT fingerprint, occurrence FROM transaction_sources WHERE account_id = ?",
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut map: HashMap<String, HashSet<u32>> = HashMap::new();
        for (fingerprint, raw_occurrence) in rows {
            let occurrence = u32::try_from(raw_occurrence)
                .map_err(|_err| crate::BcError::BadData("occurrence exceeds u32".into()))?;
            map.entry(fingerprint).or_default().insert(occurrence);
        }
        Ok(map)
    }

    /// Returns stored source legs for several accounts at once, keyed by
    /// `(account id, fingerprint)`.
    ///
    /// Import matching needs both halves of each stored leg: the occurrence, to
    /// decide whether a given slot is taken, and the owning transaction, so a
    /// later pass can attach a leg that an earlier pass could not resolve.
    ///
    /// # Arguments
    ///
    /// * `account_ids` - Accounts to load legs for. Duplicates are harmless.
    ///
    /// # Returns
    ///
    /// A map from `(account id string, fingerprint)` to the stored legs in that
    /// slot group. Absent keys mean nothing is stored for that pair.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query failure or a malformed stored
    /// occurrence.
    #[inline]
    pub async fn existing_legs(
        &self,
        account_ids: &[AccountId],
    ) -> BcResult<HashMap<(String, String), Vec<StoredLeg>>> {
        let mut map: HashMap<(String, String), Vec<StoredLeg>> = HashMap::new();
        // Queried per account rather than with a dynamic IN list: sqlx cannot
        // bind a variable-length list, and an import touches few accounts
        // relative to the rows it writes.
        for account_id in account_ids {
            let rows: Vec<(String, i64, String)> = sqlx::query_as(
                "SELECT fingerprint, occurrence, transaction_id \
                 FROM transaction_sources WHERE account_id = ?",
            )
            .bind(account_id.to_string())
            .fetch_all(&self.pool)
            .await?;

            for (fingerprint, raw_occurrence, raw_tx) in rows {
                let occurrence = u32::try_from(raw_occurrence)
                    .map_err(|_err| crate::BcError::BadData("occurrence exceeds u32".into()))?;
                let transaction_id = raw_tx
                    .parse::<TransactionId>()
                    .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
                map.entry((account_id.to_string(), fingerprint))
                    .or_default()
                    .push(StoredLeg {
                        occurrence,
                        transaction_id,
                    });
            }
        }
        Ok(map)
    }

    /// Lists all source references attached to a transaction, in occurrence order.
    ///
    /// Ordered by `occurrence` (stable across imports) rather than `created_at`,
    /// whose sub-second ties within a single import batch would be unstable.
    ///
    /// # Arguments
    ///
    /// * `transaction_id` - The transaction whose references to list.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query failure or malformed stored data.
    #[inline]
    pub async fn list_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> BcResult<Vec<SourceRef>> {
        let rows: Vec<SourceRow> = sqlx::query_as(
            "SELECT id, transaction_id, posting_id, account_id, date, narration, amount, \
                        commodity, reference, occurrence, created_at \
                 FROM transaction_sources WHERE transaction_id = ? ORDER BY occurrence ASC",
        )
        .bind(transaction_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(parse_source_row).collect()
    }

    /// Detaches (deletes) a source reference by ID.
    ///
    /// Appends a [`crate::Event::TransactionSourceDetached`] and deletes the row.
    ///
    /// # Arguments
    ///
    /// * `id` - The source reference to remove.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no such reference exists, or a
    /// database error on failure.
    #[inline]
    pub async fn detach(&self, id: &SourceRefId) -> BcResult<()> {
        let raw_transaction_id: Option<String> =
            sqlx::query_scalar("SELECT transaction_id FROM transaction_sources WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        let Some(found_transaction_id) = raw_transaction_id else {
            return Err(crate::BcError::NotFound(format!("source reference {id}")));
        };
        let transaction_id = found_transaction_id
            .parse::<TransactionId>()
            .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;

        let event = crate::Event::TransactionSourceDetached {
            id: id.clone(),
            transaction_id,
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;
        sqlx::query("DELETE FROM transaction_sources WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *db_tx)
            .await?;
        db_tx.commit().await?;
        Ok(())
    }
}

/// Parses a `transaction_sources` row tuple into a [`SourceRef`].
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if any stored ID, date, amount, or count is malformed.
fn parse_source_row(row: SourceRow) -> BcResult<SourceRef> {
    let (
        raw_id,
        raw_tx,
        raw_posting,
        raw_acct,
        raw_date,
        narration,
        raw_amount,
        commodity,
        reference,
        raw_occurrence,
        raw_created,
    ) = row;

    let id = raw_id
        .parse::<SourceRefId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let transaction_id = raw_tx
        .parse::<TransactionId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let posting_id = raw_posting
        .parse::<PostingId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let account_id = raw_acct
        .parse::<AccountId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let date = raw_date
        .parse::<Date>()
        .map_err(|e| crate::BcError::BadData(e.to_string()))?;
    let amount = match (raw_amount, commodity) {
        (Some(raw_value), Some(code)) => {
            let value = raw_value
                .parse::<rust_decimal::Decimal>()
                .map_err(|e| crate::BcError::BadData(e.to_string()))?;
            Some(Amount::new(value, CommodityCode::new(&code)))
        }
        _ => None,
    };
    let occurrence = u32::try_from(raw_occurrence)
        .map_err(|_err| crate::BcError::BadData("occurrence exceeds u32".into()))?;
    let created_at = raw_created
        .parse::<Timestamp>()
        .map_err(|e| crate::BcError::BadData(e.to_string()))?;

    let with_reference = SourceRef::builder()
        .id(id)
        .transaction_id(transaction_id)
        .posting_id(posting_id)
        .account_id(account_id)
        .date(date)
        .narration(narration)
        .amount(amount)
        .occurrence(occurrence)
        .created_at(created_at)
        .reference(reference);
    Ok(with_reference.build())
}

/// A per-row decision produced by [`plan_import`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecision {
    /// Index of this row in the input `fingerprints` slice (file order).
    pub index: usize,
    /// The row's dedup fingerprint.
    pub fingerprint: String,
    /// The row's occurrence ordinal among identical fingerprints in this batch.
    pub occurrence: u32,
    /// `true` if a stored source reference already covers this occurrence.
    pub already_imported: bool,
}

/// Plans an import: decides, per row, whether it is already imported.
///
/// Walks `fingerprints` in file order, counting occurrences of each fingerprint.
/// A row's `occurrence` is the number of identical fingerprints seen before it in
/// this batch; it is `already_imported` when that exact `(fingerprint, occurrence)`
/// slot is already stored in `existing`. This makes whole-hierarchy re-imports a
/// no-op while still importing genuinely new (or genuinely duplicated) rows, and —
/// because the check is per-slot rather than against a dense count — a re-import
/// correctly re-creates a reference that was previously detached.
///
/// # Arguments
///
/// * `existing` - Stored occurrence ordinals per fingerprint for the target account.
/// * `fingerprints` - Row fingerprints in file order.
///
/// # Returns
///
/// One [`ImportDecision`] per input fingerprint, in the same order.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "callers always use the default std HashMap hasher"
)]
pub fn plan_import(
    existing: &HashMap<String, HashSet<u32>>,
    fingerprints: &[String],
) -> Vec<ImportDecision> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    fingerprints
        .iter()
        .enumerate()
        .map(|(index, fingerprint)| {
            let occurrence = seen.get(fingerprint).copied().unwrap_or(0);
            seen.insert(fingerprint.clone(), occurrence.saturating_add(1));
            let already_imported = existing
                .get(fingerprint)
                .is_some_and(|occurrences| occurrences.contains(&occurrence));
            ImportDecision {
                index,
                fingerprint: fingerprint.clone(),
                occurrence,
                already_imported,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::*;

    async fn make_account(pool: &SqlitePool) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    /// Creates a two-legged transaction and returns its ID and the posting
    /// ID of the leg on `account`.
    ///
    /// The counter account is given a fresh unique name each call (rather than a
    /// fixed "Counter"), so calling this twice in one test — to build two distinct
    /// transactions — does not collide with the sibling-unique index on `accounts`.
    async fn make_tx(pool: &SqlitePool, account: &AccountId) -> (TransactionId, PostingId) {
        let counter = crate::AccountService::new(pool.clone())
            .create()
            .name(&format!("Counter-{}", AccountId::new()))
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create counter");
        let posting_id = PostingId::new();
        let tx = bc_models::Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 6, 27))
            .description("row")
            .postings(vec![
                bc_models::Posting::builder()
                    .id(posting_id.clone())
                    .account_id(account.clone())
                    .amount(Amount::new(
                        Decimal::from(100_i32),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(counter)
                    .amount(Amount::new(
                        Decimal::from(-100_i32),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
            ])
            .reconciliation(bc_models::Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        crate::TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create tx");
        (id, posting_id)
    }

    fn source(
        tx: &TransactionId,
        posting: &PostingId,
        account: &AccountId,
        occurrence: u32,
    ) -> SourceRef {
        SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx.clone())
            .posting_id(posting.clone())
            .account_id(account.clone())
            .date(date(2025, 6, 27))
            .narration("ACME")
            .amount(Some(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            )))
            .occurrence(occurrence)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attach_then_count_and_list(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, posting) = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());

        svc.attach(&source(&tx, &posting, &account, 0))
            .await
            .expect("attach");

        let occurrences = svc
            .existing_occurrences(&account)
            .await
            .expect("occurrences");
        let fp = SourceRef::compute_fingerprint(
            jiff::civil::date(2025, 6, 27),
            "ACME",
            Some(&Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            )),
            None,
        );
        assert_eq!(occurrences.get(&fp), Some(&HashSet::from([0])));

        let listed = svc.list_for_transaction(&tx).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed.first().expect("one source ref").account_id(),
            &account
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn detach_removes_the_ref(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, posting) = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());
        let sr = source(&tx, &posting, &account, 0);
        let id = sr.id().clone();
        svc.attach(&sr).await.expect("attach");

        svc.detach(&id).await.expect("detach");

        assert!(
            svc.list_for_transaction(&tx)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attach_rejects_non_posting_account(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, posting) = make_tx(&pool, &account).await;
        // A third account that is NOT a posting account of `tx`.
        let stranger = crate::AccountService::new(pool.clone())
            .create()
            .name("Stranger")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create stranger");
        let svc = Service::new(pool.clone());

        let result = svc.attach(&source(&tx, &posting, &stranger, 0)).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "attaching to a non-posting account must be rejected, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attach_rejects_posting_from_a_different_transaction(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, _posting) = make_tx(&pool, &account).await;
        // A second, unrelated transaction that also posts to `account` — under the
        // old account-scoped check this posting id would have been accepted for
        // `tx` because the account matches; the new check must reject it because
        // the posting itself belongs to a different transaction.
        let (_other_tx, other_posting) = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());

        let result = svc.attach(&source(&tx, &other_posting, &account, 0)).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "attaching a posting from a different transaction must be rejected, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn detach_then_reattach_same_occurrence_succeeds(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, posting) = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());

        // Three identical references at occurrences 0, 1, 2.
        let occ1 = source(&tx, &posting, &account, 1);
        let occ1_id = occ1.id().clone();
        svc.attach(&source(&tx, &posting, &account, 0))
            .await
            .expect("attach 0");
        svc.attach(&occ1).await.expect("attach 1");
        svc.attach(&source(&tx, &posting, &account, 2))
            .await
            .expect("attach 2");

        // Detach the middle one, leaving a gap in the stored slots.
        svc.detach(&occ1_id).await.expect("detach 1");

        let fp = SourceRef::compute_fingerprint(
            date(2025, 6, 27),
            "ACME",
            Some(&Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            )),
            None,
        );
        let occurrences = svc
            .existing_occurrences(&account)
            .await
            .expect("occurrences");
        assert_eq!(occurrences.get(&fp), Some(&HashSet::from([0, 2])));

        // Re-attaching occurrence 1 (fresh id) must succeed — its slot is free,
        // so the UNIQUE (account, fingerprint, occurrence) key is not violated.
        svc.attach(&source(&tx, &posting, &account, 1))
            .await
            .expect("re-attach occurrence 1 after detach");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn existing_legs_reports_the_owning_transaction(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let (tx, posting_id) = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());

        let amount = Amount::new(Decimal::from(-5_i64), CommodityCode::new("AUD"));
        let source = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx.clone())
            .posting_id(posting_id)
            .account_id(account.clone())
            .date(date(2025, 6, 27))
            .narration("COFFEE")
            .amount(Some(amount.clone()))
            .reference(None)
            .occurrence(0)
            .created_at(Timestamp::now())
            .build();
        svc.attach(&source).await.expect("attach");

        let legs = svc
            .existing_legs(core::slice::from_ref(&account))
            .await
            .expect("existing_legs");

        let fingerprint =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "COFFEE", Some(&amount), None);
        let stored = legs
            .get(&(account.to_string(), fingerprint))
            .expect("leg present");
        assert_eq!(stored.len(), 1);
        let leg = stored.first().expect("leg present");
        assert_eq!(leg.occurrence, 0);
        assert_eq!(
            leg.transaction_id, tx,
            "the owning transaction is what lets a later pass attach a missing leg"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn existing_legs_is_empty_for_untouched_accounts(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let svc = Service::new(pool.clone());
        let legs = svc.existing_legs(&[account]).await.expect("existing_legs");
        assert!(legs.is_empty());
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn plan_import_marks_new_rows_and_skips_seen() {
        let mut existing = HashMap::new();
        existing.insert("A".to_owned(), HashSet::from([0_u32]));

        let fps = vec!["A".to_owned(), "A".to_owned(), "B".to_owned()];
        let decisions = plan_import(&existing, &fps);

        assert_eq!(decisions[0].occurrence, 0);
        assert!(decisions[0].already_imported);
        assert_eq!(decisions[1].occurrence, 1);
        assert!(!decisions[1].already_imported);
        assert_eq!(decisions[2].occurrence, 0);
        assert!(!decisions[2].already_imported);
    }

    #[test]
    fn plan_import_is_idempotent_when_slots_filled() {
        let mut existing = HashMap::new();
        existing.insert("A".to_owned(), HashSet::from([0_u32, 1]));
        let fps = vec!["A".to_owned(), "A".to_owned()];
        let decisions = plan_import(&existing, &fps);
        assert!(
            decisions.iter().all(|d| d.already_imported),
            "re-import is a no-op"
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn plan_import_reimports_detached_occurrence() {
        // Occurrence 1 was detached, leaving a gap: stored slots are {0, 2}.
        let mut existing = HashMap::new();
        existing.insert("A".to_owned(), HashSet::from([0_u32, 2]));

        let fps = vec!["A".to_owned(), "A".to_owned(), "A".to_owned()];
        let decisions = plan_import(&existing, &fps);

        assert!(decisions[0].already_imported, "occ 0 still stored");
        assert!(
            !decisions[1].already_imported,
            "occ 1 was detached, so re-import re-creates it"
        );
        assert!(decisions[2].already_imported, "occ 2 still stored");
    }
}
