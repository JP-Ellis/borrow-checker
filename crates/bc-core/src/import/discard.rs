//! Discarding an import batch: undoing one import run.
//!
//! A discarded batch is a run that never happened. Every reference it wrote is
//! hard-deleted — freeing its `(account_id, fingerprint, occurrence)` slot, the
//! opposite of the tombstone a deleted leg leaves — every posting it created
//! goes with them, and every transaction left holding no postings goes too.
//! Only a posting the run *adopted* survives, losing its provenance.
//!
//! The work is driven by the reference rows, never by the batch's recorded
//! counts, so a run that aborted before recording anything discards exactly as
//! correctly as one that completed.

use std::collections::BTreeSet;

use bc_models::ImportBatchId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::events::insert_event;

/// What a discard removed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The batch that was discarded.
    pub batch_id: ImportBatchId,
    /// Postings deleted because the run had created them.
    pub removed_postings: usize,
    /// Transactions deleted because the discard left them with no postings.
    pub removed_transactions: usize,
    /// References removed from postings the run adopted rather than created.
    /// Those postings still stand.
    pub detached_adopted: usize,
    /// Tombstoned references removed, freeing their occurrence slots.
    pub freed_tombstones: usize,
    /// Any other reference that went with a deleted transaction: one this
    /// batch did not own, whether it belonged to another run or was attached
    /// with no batch at all (via the public `SourceService::attach`).
    pub other_batch_references_removed: usize,
    /// Of [`Self::removed_postings`], those whose account or amount no longer
    /// matched the reference describing them — the user had edited them.
    pub edited_postings: usize,
    /// Of [`Self::removed_postings`], those in a transaction that was no
    /// longer unreconciled.
    pub reconciled_postings: usize,
}

/// Discards an import batch.
///
/// # Arguments
///
/// * `pool` - A SQLite connection pool connected to the BorrowChecker database.
/// * `id` - The batch to discard.
///
/// # Returns
///
/// An [`Outcome`] describing everything the discard removed.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if no batch with that ID exists,
/// [`BcError::InvalidInput`] if it has already been discarded,
/// [`BcError::BadData`] if a count exceeds `u64`, and [`BcError::Database`] on
/// any query failure.
pub(crate) async fn discard(pool: &SqlitePool, id: &ImportBatchId) -> BcResult<Outcome> {
    let id_str = id.to_string();
    let mut db_tx = pool.begin().await?;

    ensure_discardable(&mut db_tx, id, &id_str).await?;
    let plan = Plan::read(&mut db_tx, &id_str).await?;
    // Counted while the rows still exist: both compare a posting against the
    // reference describing it, and the references are about to be deleted.
    let (edited_postings, reconciled_postings) = count_edits(&mut db_tx, &id_str).await?;

    // Free the slots first. Deleting the postings first would only make the
    // ON DELETE SET NULL trigger churn rows that are about to vanish.
    sqlx::query("DELETE FROM transaction_sources WHERE import_batch_id = ?")
        .bind(&id_str)
        .execute(&mut *db_tx)
        .await?;
    delete_postings(&mut db_tx, &plan.owned_postings).await?;
    let (removed_transactions, other_batch_references_removed) =
        sweep_empty_transactions(&mut db_tx, &plan.touched).await?;

    let outcome = Outcome {
        batch_id: id.clone(),
        removed_postings: plan.owned_postings.len(),
        removed_transactions,
        detached_adopted: plan.detached_adopted,
        freed_tombstones: plan.freed_tombstones,
        other_batch_references_removed,
        edited_postings,
        reconciled_postings,
    };

    sqlx::query("UPDATE import_batches SET discarded_at = ? WHERE id = ?")
        .bind(Timestamp::now().to_string())
        .bind(&id_str)
        .execute(&mut *db_tx)
        .await?;

    insert_event(&event_for(&outcome)?, &mut db_tx).await?;
    db_tx.commit().await?;

    tracing::info!(
        batch_id = %id,
        removed_postings = outcome.removed_postings,
        removed_transactions = outcome.removed_transactions,
        detached_adopted = outcome.detached_adopted,
        freed_tombstones = outcome.freed_tombstones,
        other_batch_references_removed = outcome.other_batch_references_removed,
        edited_postings = outcome.edited_postings,
        reconciled_postings = outcome.reconciled_postings,
        "import batch discarded"
    );
    Ok(outcome)
}

/// Checks that a batch exists and has not already been discarded.
///
/// A batch that does not exist cannot be discarded, and one already discarded
/// must not be discarded again: the second run would find no references and
/// silently report having removed nothing.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id` - The batch being discarded, for the error messages.
/// * `id_str` - That batch's ID as stored.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if no batch with that ID exists,
/// [`BcError::InvalidInput`] if it has already been discarded, and
/// [`BcError::Database`] on query failure.
pub(crate) async fn ensure_discardable(
    conn: &mut sqlx::SqliteConnection,
    id: &ImportBatchId,
    id_str: &str,
) -> BcResult<()> {
    let existing: Option<Option<String>> =
        sqlx::query_scalar("SELECT discarded_at FROM import_batches WHERE id = ?")
            .bind(id_str)
            .fetch_optional(conn)
            .await?;
    match existing {
        None => Err(BcError::NotFound(format!("import batch {id}"))),
        Some(Some(_)) => Err(already_discarded_error(id)),
        Some(None) => Ok(()),
    }
}

/// Builds the error for a batch that has already been discarded.
///
/// This is the single source of that error's text. [`ensure_discardable`] is
/// the only caller, so both the predicate (has this batch already been
/// discarded?) and its message live in core; nothing outside this crate
/// decides when the error fires. [`crate::ImportBatchService::ensure_discardable`]
/// exposes the same check as a cheap short-circuit `bc-cli`'s `execute_discard`
/// can run before it pays for a snapshot — [`ensure_discardable`] here remains
/// the authoritative guard, run again inside `discard`'s transaction.
///
/// # Arguments
///
/// * `id` - The batch that has already been discarded.
///
/// # Returns
///
/// A [`BcError::InvalidInput`] naming the batch.
#[must_use]
fn already_discarded_error(id: &ImportBatchId) -> BcError {
    BcError::InvalidInput(format!("import batch {id} has already been discarded"))
}

/// What the batch's reference rows say the discard has to do.
struct Plan {
    /// Postings the run created, which go with their references.
    owned_postings: Vec<String>,
    /// References on postings the run adopted; those postings stay.
    detached_adopted: usize,
    /// References whose posting the user had already deleted.
    freed_tombstones: usize,
    /// Every transaction the batch's references name, in a deterministic order.
    touched: BTreeSet<String>,
}

impl Plan {
    /// Reads a batch's references and sorts them into a [`Plan`].
    ///
    /// # Arguments
    ///
    /// * `conn` - An open SQLite connection or transaction.
    /// * `id_str` - The batch's ID as stored.
    ///
    /// # Returns
    ///
    /// The [`Plan`] describing what the discard must remove.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure.
    async fn read(conn: &mut sqlx::SqliteConnection, id_str: &str) -> BcResult<Self> {
        let refs: Vec<(Option<String>, bool, String)> = sqlx::query_as(
            "SELECT posting_id, owns_posting, transaction_id \
             FROM transaction_sources WHERE import_batch_id = ?",
        )
        .bind(id_str)
        .fetch_all(conn)
        .await?;

        let mut plan = Self {
            owned_postings: Vec::new(),
            detached_adopted: 0,
            freed_tombstones: 0,
            touched: BTreeSet::new(),
        };
        for (posting_id, owns_posting, transaction_id) in refs {
            plan.touched.insert(transaction_id);
            match posting_id {
                // A tombstone: the leg is already gone, only its slot remains.
                None => plan.freed_tombstones = plan.freed_tombstones.saturating_add(1),
                Some(posting) if owns_posting => plan.owned_postings.push(posting),
                Some(_) => plan.detached_adopted = plan.detached_adopted.saturating_add(1),
            }
        }
        Ok(plan)
    }
}

/// Counts the batch's own postings the user has since edited or reconciled.
///
/// A posting is "edited" when it no longer agrees with the reference describing
/// it: the reference records what the document said and never moves, so a
/// corrected amount or a recategorisation shows up as a disagreement. `IS NOT`
/// is SQLite's null-safe inequality, which an elided leg's NULL amount needs.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id_str` - The batch's ID as stored.
///
/// # Returns
///
/// The edited and reconciled posting counts, in that order.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn count_edits(conn: &mut sqlx::SqliteConnection, id_str: &str) -> BcResult<(usize, usize)> {
    let edited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transaction_sources s \
         JOIN postings p ON p.id = s.posting_id \
         WHERE s.import_batch_id = ? AND s.owns_posting = 1 \
           AND (p.account_id <> s.account_id \
                OR p.amount IS NOT s.amount \
                OR p.commodity IS NOT s.commodity)",
    )
    .bind(id_str)
    .fetch_one(&mut *conn)
    .await?;

    let reconciled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transaction_sources s \
         JOIN postings p ON p.id = s.posting_id \
         JOIN transactions t ON t.id = p.transaction_id \
         WHERE s.import_batch_id = ? AND s.owns_posting = 1 \
           AND t.reconciliation <> 'unreconciled'",
    )
    .bind(id_str)
    .fetch_one(conn)
    .await?;

    Ok((to_usize(edited), to_usize(reconciled)))
}

/// Deletes the given postings and their tag memberships.
///
/// `posting_tags` does not cascade from `postings`, so it goes explicitly.
/// sqlx cannot bind a list, so these run per ID, as elsewhere in the crate.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `posting_ids` - The postings to delete.
///
/// # Errors
///
/// Returns [`BcError::Database`] on delete failure.
async fn delete_postings(
    conn: &mut sqlx::SqliteConnection,
    posting_ids: &[String],
) -> BcResult<()> {
    for posting_id in posting_ids {
        sqlx::query("DELETE FROM posting_tags WHERE posting_id = ?")
            .bind(posting_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM postings WHERE id = ?")
            .bind(posting_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Deletes every named transaction that now holds no postings.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `transaction_ids` - The transactions the discard touched.
///
/// # Returns
///
/// The number of transactions deleted, and the number of other references —
/// this batch's own are already gone by this point — that went with them.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query or delete failure.
async fn sweep_empty_transactions(
    conn: &mut sqlx::SqliteConnection,
    transaction_ids: &BTreeSet<String>,
) -> BcResult<(usize, usize)> {
    let mut removed_transactions: usize = 0;
    let mut other_batch_references_removed: usize = 0;

    for transaction_id in transaction_ids {
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
                .bind(transaction_id)
                .fetch_one(&mut *conn)
                .await?;
        if remaining > 0 {
            continue;
        }

        // Counted after this batch's own references are gone, so it names
        // every other reference left on the transaction — another run's, or
        // one attached with no batch at all. Those cascade with the
        // transaction, freeing their slots too — correct, since the
        // transaction itself no longer exists to be duplicated.
        let collateral: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM transaction_sources WHERE transaction_id = ?")
                .bind(transaction_id)
                .fetch_one(&mut *conn)
                .await?;

        // Only transaction_sources cascades; the rest must go by hand.
        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(transaction_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM transaction_dates WHERE transaction_id = ?")
            .bind(transaction_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM transactions WHERE id = ?")
            .bind(transaction_id)
            .execute(&mut *conn)
            .await?;

        removed_transactions = removed_transactions.saturating_add(1);
        other_batch_references_removed =
            other_batch_references_removed.saturating_add(to_usize(collateral));
    }

    Ok((removed_transactions, other_batch_references_removed))
}

/// Converts a `COUNT(*)` result to a `usize`, saturating rather than failing.
///
/// # Arguments
///
/// * `value` - A row count, which SQLite never reports as negative.
///
/// # Returns
///
/// The count as a `usize`, or [`usize::MAX`] if it somehow does not fit.
fn to_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Builds the audit event for a completed discard.
///
/// # Arguments
///
/// * `outcome` - What the discard removed.
///
/// # Returns
///
/// The [`crate::Event::ImportBatchDiscarded`] to append.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if a count exceeds `u64`.
fn event_for(outcome: &Outcome) -> BcResult<crate::Event> {
    let to_u64 = |value: usize| {
        u64::try_from(value).map_err(|_err| BcError::BadData("discard count exceeds u64".into()))
    };
    Ok(crate::Event::ImportBatchDiscarded {
        batch_id: outcome.batch_id.clone(),
        removed_postings: to_u64(outcome.removed_postings)?,
        removed_transactions: to_u64(outcome.removed_transactions)?,
        detached_adopted: to_u64(outcome.detached_adopted)?,
        freed_tombstones: to_u64(outcome.freed_tombstones)?,
        other_batch_references_removed: to_u64(outcome.other_batch_references_removed)?,
        edited_postings: to_u64(outcome.edited_postings)?,
        reconciled_postings: to_u64(outcome.reconciled_postings)?,
    })
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::ImportBatchId;
    use bc_models::PostingId;
    use bc_models::SourceRef;
    use bc_models::SourceRefId;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use crate::ImportBatchService;

    /// Creates a top-level account and returns its ID.
    async fn account(pool: &SqlitePool, name: &str) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    /// Inserts an unreconciled transaction with one posting on `account_id`.
    ///
    /// Unreconciled deliberately: `make_tx` in `source.rs` builds a reconciled
    /// one, which would make every discard here report a reconciled posting.
    async fn transaction_with_posting(
        pool: &SqlitePool,
        account_id: &AccountId,
    ) -> (TransactionId, PostingId) {
        let posting_id = PostingId::new();
        let tx = bc_models::Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("ACME")
            .postings(vec![
                bc_models::Posting::builder()
                    .id(posting_id.clone())
                    .account_id(account_id.clone())
                    .amount(Amount::new(
                        Decimal::from(50_i32),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
            ])
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        crate::TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create tx");
        (id, posting_id)
    }

    /// Appends a posting on `account_id` to an existing transaction.
    ///
    /// Carries the same 50 AUD `attach` records, so a reference pointing at it
    /// reads as unedited. The resulting transaction is unbalanced, which this
    /// codebase permits — `transaction_with_posting` is already one-sided.
    async fn add_posting(
        pool: &SqlitePool,
        transaction_id: &TransactionId,
        account_id: &AccountId,
    ) -> PostingId {
        let posting_id = PostingId::new();
        let posting = bc_models::Posting::builder()
            .id(posting_id.clone())
            .account_id(account_id.clone())
            .amount(Amount::new(
                Decimal::from(50_i32),
                CommodityCode::new("AUD"),
            ))
            .build();
        let transactions = crate::TransactionService::new(pool.clone());
        let mut db_tx = pool.begin().await.expect("begin");
        transactions
            .add_postings_in_tx(&mut db_tx, transaction_id, &[posting])
            .await
            .expect("add posting");
        db_tx.commit().await.expect("commit");
        posting_id
    }

    /// Attaches a reference owned by `batch`, pointing at `posting_id`.
    async fn attach(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        transaction_id: &TransactionId,
        posting_id: &PostingId,
        account_id: &AccountId,
        owns_posting: bool,
    ) -> SourceRefId {
        attach_at(
            pool,
            batch,
            transaction_id,
            posting_id,
            account_id,
            owns_posting,
            0,
        )
        .await
    }

    /// Attaches a reference owned by `batch`, pointing at `posting_id`, at a
    /// specific `occurrence` — for tests that need a second reference to share
    /// an account and fingerprint without colliding on the dedup slot.
    async fn attach_at(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        transaction_id: &TransactionId,
        posting_id: &PostingId,
        account_id: &AccountId,
        owns_posting: bool,
        occurrence: u32,
    ) -> SourceRefId {
        let id = SourceRefId::new();
        let source = SourceRef::builder()
            .id(id.clone())
            .transaction_id(transaction_id.clone())
            .posting_id(Some(posting_id.clone()))
            .account_id(account_id.clone())
            .date(date(2026, 1, 15))
            .narration("ACME")
            .amount(Some(Amount::new(
                Decimal::from(50_i32),
                CommodityCode::new("AUD"),
            )))
            .reference(None)
            .occurrence(occurrence)
            .import_batch_id(Some(batch.clone()))
            .owns_posting(owns_posting)
            .created_at(Timestamp::now())
            .build();
        crate::SourceService::new(pool.clone())
            .attach(&source)
            .await
            .expect("attach");
        id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_owned_posting_and_its_empty_transaction_are_removed(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.removed_transactions, 1);
        assert_eq!(outcome.detached_adopted, 0);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            0
        );
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM postings").await, 0);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM transactions").await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_adopted_posting_survives_with_its_transaction(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, false).await;

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_postings, 0);
        assert_eq!(outcome.removed_transactions, 0);
        assert_eq!(outcome.detached_adopted, 1);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM postings").await,
            1,
            "the posting was the user's; only its provenance was the run's"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            0
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_transaction_keeping_a_posting_is_not_removed(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let first = batches.open(None, "csv").await.expect("open first");
        let second = batches.open(None, "csv").await.expect("open second");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &first, &tx, &posting, &acct, true).await;
        let second_posting = add_posting(&pool, &tx, &other).await;
        attach(&pool, &second, &tx, &second_posting, &other, true).await;

        let outcome = batches.discard(&second).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(
            outcome.removed_transactions, 0,
            "the first run's leg still holds the transaction up"
        );
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM postings").await, 1);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            1,
            "the first run's provenance is untouched"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tombstone_is_removed_and_counted(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        let ref_id = attach(&pool, &batch, &tx, &posting, &acct, true).await;
        // Tombstone it the way an edit does: clear the posting link.
        sqlx::query("UPDATE transaction_sources SET posting_id = NULL WHERE id = ?")
            .bind(ref_id.to_string())
            .execute(&pool)
            .await
            .expect("tombstone");

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.freed_tombstones, 1);
        assert_eq!(outcome.removed_postings, 0, "the posting was already gone");
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            0
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_transaction_takes_another_runs_references_with_it(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let first = batches.open(None, "csv").await.expect("open first");
        let second = batches.open(None, "csv").await.expect("open second");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        let stale = attach(&pool, &first, &tx, &posting, &acct, true).await;
        // The second run's leg is added while the first still stands: appending
        // to a transaction with no postings is refused.
        let second_posting = add_posting(&pool, &tx, &other).await;
        attach(&pool, &second, &tx, &second_posting, &other, true).await;
        // The first run's own leg was then deleted by the user, leaving a
        // tombstone, so only the second run's leg holds the transaction up.
        sqlx::query("UPDATE transaction_sources SET posting_id = NULL WHERE id = ?")
            .bind(stale.to_string())
            .execute(&pool)
            .await
            .expect("tombstone");
        sqlx::query("DELETE FROM postings WHERE id = ?")
            .bind(posting.to_string())
            .execute(&pool)
            .await
            .expect("delete posting");

        let outcome = batches.discard(&second).await.expect("discard");

        assert_eq!(outcome.removed_transactions, 1);
        assert_eq!(
            outcome.other_batch_references_removed, 1,
            "the first run's tombstone went with the transaction"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            0
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_surviving_transactions_other_batch_reference_is_tombstoned(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let first = batches.open(None, "csv").await.expect("open first");
        let second = batches.open(None, "csv").await.expect("open second");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &first, &tx, &posting, &acct, true).await;
        // The second run's own leg keeps the transaction alive on its own.
        let second_posting = add_posting(&pool, &tx, &other).await;
        attach(&pool, &second, &tx, &second_posting, &other, true).await;
        // The second run also adopted the first run's leg — a reference to the
        // same posting, but not owning it. A distinct occurrence, since the
        // first run's own reference already holds slot 0 on this fingerprint.
        let adopted_ref = attach_at(&pool, &second, &tx, &posting, &acct, false, 1).await;

        let outcome = batches.discard(&first).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1, "the first run's own leg goes");
        assert_eq!(
            outcome.removed_transactions, 0,
            "the second run's own leg still holds the transaction up"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_sources").await,
            2,
            "both of the second run's references remain"
        );
        let tombstoned_posting: Option<String> =
            sqlx::query_scalar("SELECT posting_id FROM transaction_sources WHERE id = ?")
                .bind(adopted_ref.to_string())
                .fetch_one(&pool)
                .await
                .expect("fetch adopted reference");
        assert_eq!(
            tombstoned_posting, None,
            "deleting the posting it named tombstones the adopted reference, \
             it does not delete the reference row"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_edited_posting_is_removed_and_counted(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let elsewhere = account(&pool, "Savings").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;
        // The user recategorises the leg. The reference keeps the account the
        // document named, so the two now disagree.
        sqlx::query("UPDATE postings SET account_id = ? WHERE id = ?")
            .bind(elsewhere.to_string())
            .bind(posting.to_string())
            .execute(&pool)
            .await
            .expect("recategorise");

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(
            outcome.removed_postings, 1,
            "an edit does not save a posting the run created"
        );
        assert_eq!(outcome.edited_postings, 1);
        assert_eq!(outcome.reconciled_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_reconciled_posting_is_removed_and_counted(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;
        sqlx::query("UPDATE transactions SET reconciliation = 'reconciled' WHERE id = ?")
            .bind(tx.to_string())
            .execute(&pool)
            .await
            .expect("reconcile");

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.reconciled_postings, 1);
        assert_eq!(
            outcome.edited_postings, 0,
            "reconciling changes no value the reference records"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_untouched_posting_counts_as_neither(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.edited_postings, 0);
        assert_eq!(outcome.reconciled_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_incomplete_batch_discards_from_its_rows(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;
        // Never closed: the counts say nothing, the rows say everything.

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1);
        let record = batches.find_by_id(&batch).await.expect("find");
        assert!(record.discarded_at.is_some());
        assert_eq!(record.counts, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn discarding_twice_is_refused(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        batches.discard(&batch).await.expect("first discard");

        let result = batches.discard(&batch).await;

        assert!(matches!(result, Err(crate::BcError::InvalidInput(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn discarding_an_unknown_batch_is_not_found(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let result = batches.discard(&ImportBatchId::new()).await;
        assert!(matches!(result, Err(crate::BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_discard_appends_one_event(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;

        batches.discard(&batch).await.expect("discard");

        let kinds: Vec<String> =
            sqlx::query_scalar("SELECT kind FROM events WHERE kind = 'ImportBatchDiscarded'")
                .fetch_all(&pool)
                .await
                .expect("query events");
        assert_eq!(
            kinds.len(),
            1,
            "one event describes the whole discard, not one per row"
        );
    }

    /// Counts rows for a `SELECT COUNT(*)` query.
    async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(pool)
            .await
            .expect("count")
    }
}
