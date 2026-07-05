//! Transfer resolution: merge/unmerge two single-posting transactions and
//! suggest candidate transfer pairs.

use bc_models::SourceRefId;
use bc_models::Transaction;
use bc_models::TransactionId;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// Merges, unmerges, and suggests transfer pairs.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// SQLite connection pool.
    pool: SqlitePool,
}

/// The survivor's pre-merge state, captured by a [`crate::Event::TransactionsMerged`]
/// and restored by [`Service::unmerge`].
struct SurvivorSnapshot {
    /// Survivor's value date before the merge.
    date: jiff::civil::Date,
    /// Survivor's transaction-level tags before the merge.
    tags: Vec<bc_models::TagId>,
    /// Survivor's labeled extra dates before the merge.
    extra_dates: Vec<(String, jiff::civil::Date)>,
}

impl Service {
    /// Creates a new transfer service.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool.
    #[inline]
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Merges `absorbed_id` into `survivor_id`.
    ///
    /// Moves the absorbed transaction's single posting and its source references
    /// onto the survivor, unions tags/dates, sets the survivor date to the
    /// earlier of the two, then deletes the absorbed transaction. Records a
    /// self-contained [`crate::Event::TransactionsMerged`] so the merge can be
    /// reversed by [`Service::unmerge`]. All writes share one DB transaction.
    ///
    /// # Arguments
    ///
    /// * `survivor_id` - The transaction that survives (its ID and user fields persist).
    /// * `absorbed_id` - The transaction fused into the survivor.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if either transaction is missing,
    /// [`BcError::NotMergeable`] if the pair fails the merge preconditions, or
    /// [`BcError`] on a database failure.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "one cohesive atomic merge: validate, snapshot, and mutate five projection tables"
    )]
    pub async fn merge(
        &self,
        survivor_id: &TransactionId,
        absorbed_id: &TransactionId,
    ) -> BcResult<()> {
        if survivor_id == absorbed_id {
            return Err(BcError::NotMergeable {
                reason: "cannot merge a transaction with itself".to_owned(),
            });
        }
        let txs = crate::TransactionService::new(self.pool.clone());
        let srcs = crate::SourceService::new(self.pool.clone());

        let survivor = txs.find_by_id(survivor_id).await?;
        let absorbed = txs.find_by_id(absorbed_id).await?;
        check_mergeable(&survivor, &absorbed)?;

        let absorbed_posting =
            absorbed
                .postings()
                .first()
                .ok_or_else(|| BcError::NotMergeable {
                    reason: "absorbed has no posting".to_owned(),
                })?;
        let survivor_posting_count = i64::try_from(survivor.postings().len())
            .map_err(|_err| BcError::BadData("posting count exceeds i64".into()))?;

        let absorbed_refs = srcs.list_for_transaction(absorbed_id).await?;
        let source_ref_ids: Vec<SourceRefId> =
            absorbed_refs.iter().map(|r| r.id().clone()).collect();

        let new_date = survivor.date().min(absorbed.date());
        let new_reconciliation = if reconciliation_rank(absorbed.reconciliation())
            > reconciliation_rank(survivor.reconciliation())
        {
            absorbed.reconciliation()
        } else {
            survivor.reconciliation()
        };

        let snapshot = crate::events::AbsorbedTransaction {
            id: absorbed_id.clone(),
            date: absorbed.date(),
            payee: absorbed.payee().map(str::to_owned),
            description: absorbed.description().to_owned(),
            note: absorbed.note().map(str::to_owned),
            reconciliation: absorbed.reconciliation(),
            created_at: *absorbed.created_at(),
            tag_ids: absorbed.tag_ids().to_vec(),
            extra_dates: absorbed.extra_dates().to_vec(),
            posting_id: absorbed_posting.id().clone(),
            posting_position: 0,
            source_ref_ids,
        };
        let event = crate::Event::TransactionsMerged {
            survivor_id: survivor_id.clone(),
            absorbed: snapshot,
            survivor_date_before: survivor.date(),
            survivor_tags_before: survivor.tag_ids().to_vec(),
            survivor_extra_dates_before: survivor.extra_dates().to_vec(),
        };

        let survivor_str = survivor_id.to_string();
        let absorbed_str = absorbed_id.to_string();
        let mut db_tx = self.pool.begin().await?;

        crate::events::insert_event(&event, &mut db_tx).await?;

        // Repoint the absorbed transaction's source refs onto the survivor.
        sqlx::query("UPDATE transaction_sources SET transaction_id = ? WHERE transaction_id = ?")
            .bind(&survivor_str)
            .bind(&absorbed_str)
            .execute(&mut *db_tx)
            .await?;

        // Move the absorbed posting onto the survivor at the next free position.
        sqlx::query(
            "UPDATE postings SET transaction_id = ?, position = ? WHERE transaction_id = ?",
        )
        .bind(&survivor_str)
        .bind(survivor_posting_count)
        .bind(&absorbed_str)
        .execute(&mut *db_tx)
        .await?;

        // Union the absorbed transaction's tags and dates onto the survivor.
        sqlx::query(
            "INSERT OR IGNORE INTO transaction_tags (transaction_id, tag_id) \
             SELECT ?, tag_id FROM transaction_tags WHERE transaction_id = ?",
        )
        .bind(&survivor_str)
        .bind(&absorbed_str)
        .execute(&mut *db_tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO transaction_dates (transaction_id, label, date) \
             SELECT ?, label, date FROM transaction_dates WHERE transaction_id = ?",
        )
        .bind(&survivor_str)
        .bind(&absorbed_str)
        .execute(&mut *db_tx)
        .await?;

        // Update the survivor header: earliest date, most-settled reconciliation.
        sqlx::query("UPDATE transactions SET date = ?, reconciliation = ? WHERE id = ?")
            .bind(new_date.to_string())
            .bind(crate::db::to_db_str(new_reconciliation)?)
            .bind(&survivor_str)
            .execute(&mut *db_tx)
            .await?;

        // Remove the absorbed transaction's now-orphaned child rows, then itself.
        // (postings + transaction_sources were repointed above; posting_tags follow
        // the moved posting; transaction_tags/transaction_dates do not cascade.)
        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(&absorbed_str)
            .execute(&mut *db_tx)
            .await?;
        sqlx::query("DELETE FROM transaction_dates WHERE transaction_id = ?")
            .bind(&absorbed_str)
            .execute(&mut *db_tx)
            .await?;
        sqlx::query("DELETE FROM transactions WHERE id = ?")
            .bind(&absorbed_str)
            .execute(&mut *db_tx)
            .await?;

        db_tx.commit().await?;
        Ok(())
    }

    /// Reverses the most recent un-reversed merge on `survivor_id`.
    ///
    /// Recreates the absorbed transaction with its original ID, moves its posting
    /// and source references back, restores the survivor's pre-merge date and
    /// tag/date sets, and records a [`crate::Event::TransactionUnmerged`].
    ///
    /// # Arguments
    ///
    /// * `survivor_id` - The transaction a prior merge fused into.
    ///
    /// # Returns
    ///
    /// The ID of the restored (absorbed) transaction.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotMerged`] if `survivor_id` has no un-reversed merge,
    /// or [`BcError`] on a database failure.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "one cohesive atomic unmerge: replay history, recreate the absorbed row, and restore five projection tables"
    )]
    pub async fn unmerge(&self, survivor_id: &TransactionId) -> BcResult<TransactionId> {
        let store = crate::SqliteEventStore::new(self.pool.clone());
        let records = store.replay_for(&survivor_id.to_string()).await?;

        // Pair merges with unmerges LIFO; the top of the stack is the merge to reverse.
        let mut stack: Vec<crate::events::AbsorbedTransaction> = Vec::new();
        let mut snapshots: Vec<SurvivorSnapshot> = Vec::new();
        for record in &records {
            match record.kind.as_str() {
                "TransactionsMerged" => {
                    let event: crate::Event = serde_json::from_str(&record.payload)?;
                    if let crate::Event::TransactionsMerged {
                        absorbed,
                        survivor_date_before,
                        survivor_tags_before,
                        survivor_extra_dates_before,
                        ..
                    } = event
                    {
                        stack.push(absorbed);
                        snapshots.push(SurvivorSnapshot {
                            date: survivor_date_before,
                            tags: survivor_tags_before,
                            extra_dates: survivor_extra_dates_before,
                        });
                    }
                }
                "TransactionUnmerged" => {
                    stack.pop();
                    snapshots.pop();
                }
                _ => {}
            }
        }
        let (Some(absorbed), Some(snapshot)) = (stack.pop(), snapshots.pop()) else {
            return Err(BcError::NotMerged(survivor_id.clone()));
        };

        let survivor_str = survivor_id.to_string();
        let absorbed_str = absorbed.id.to_string();
        let posting_position = i64::from(absorbed.posting_position);

        let unmerged = crate::Event::TransactionUnmerged {
            survivor_id: survivor_id.clone(),
            absorbed_id: absorbed.id.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&unmerged, &mut db_tx).await?;

        // Recreate the absorbed transaction header with its original id.
        sqlx::query(
            "INSERT INTO transactions (id, date, payee, description, note, reconciliation, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&absorbed_str)
        .bind(absorbed.date.to_string())
        .bind(absorbed.payee.as_deref())
        .bind(&absorbed.description)
        .bind(absorbed.note.as_deref())
        .bind(crate::db::to_db_str(absorbed.reconciliation)?)
        .bind(absorbed.created_at.to_string())
        .execute(&mut *db_tx)
        .await?;

        // Restore the absorbed transaction's own tags and dates. The recreated
        // absorbed row's child tables are empty (plain `INSERT` above), and
        // `absorbed.tag_ids` came from a real `transaction_tags` snapshot
        // (composite PK `(transaction_id, tag_id)`), so it cannot contain
        // duplicates — a plain `INSERT` here can never collide.
        for tag_id in &absorbed.tag_ids {
            sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
                .bind(&absorbed_str)
                .bind(tag_id.to_string())
                .execute(&mut *db_tx)
                .await?;
        }
        for (label, when) in &absorbed.extra_dates {
            sqlx::query(
                "INSERT INTO transaction_dates (transaction_id, label, date) VALUES (?, ?, ?)",
            )
            .bind(&absorbed_str)
            .bind(label)
            .bind(when.to_string())
            .execute(&mut *db_tx)
            .await?;
        }

        // Restore the survivor's pre-merge tag/date sets (replace-to-snapshot).
        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(&survivor_str)
            .execute(&mut *db_tx)
            .await?;
        for tag_id in &snapshot.tags {
            sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
                .bind(&survivor_str)
                .bind(tag_id.to_string())
                .execute(&mut *db_tx)
                .await?;
        }
        sqlx::query("DELETE FROM transaction_dates WHERE transaction_id = ?")
            .bind(&survivor_str)
            .execute(&mut *db_tx)
            .await?;
        for (label, when) in &snapshot.extra_dates {
            sqlx::query(
                "INSERT INTO transaction_dates (transaction_id, label, date) VALUES (?, ?, ?)",
            )
            .bind(&survivor_str)
            .bind(label)
            .bind(when.to_string())
            .execute(&mut *db_tx)
            .await?;
        }

        // Move the posting and source refs back to the absorbed transaction.
        sqlx::query("UPDATE postings SET transaction_id = ?, position = ? WHERE id = ?")
            .bind(&absorbed_str)
            .bind(posting_position)
            .bind(absorbed.posting_id.to_string())
            .execute(&mut *db_tx)
            .await?;
        for ref_id in &absorbed.source_ref_ids {
            sqlx::query("UPDATE transaction_sources SET transaction_id = ? WHERE id = ?")
                .bind(&absorbed_str)
                .bind(ref_id.to_string())
                .execute(&mut *db_tx)
                .await?;
        }

        // Restore the survivor's pre-merge date.
        sqlx::query("UPDATE transactions SET date = ? WHERE id = ?")
            .bind(snapshot.date.to_string())
            .bind(&survivor_str)
            .execute(&mut *db_tx)
            .await?;

        db_tx.commit().await?;
        Ok(absorbed.id)
    }
}

/// Validates that two transactions may be merged.
///
/// Each must have exactly one concrete posting; the two postings must share a
/// commodity and be equal in magnitude and opposite in sign.
///
/// # Arguments
///
/// * `survivor` - The transaction that will survive the merge.
/// * `absorbed` - The transaction that will be fused into the survivor.
///
/// # Returns
///
/// `Ok(())` if the pair may be merged.
///
/// # Errors
///
/// Returns [`BcError::NotMergeable`] describing the first failed precondition.
fn check_mergeable(survivor: &Transaction, absorbed: &Transaction) -> BcResult<()> {
    let reject = |reason: &str| {
        Err(BcError::NotMergeable {
            reason: reason.to_owned(),
        })
    };

    let (Some(a), Some(b)) = (survivor.postings().first(), absorbed.postings().first()) else {
        return reject("both transactions must have a posting");
    };
    if survivor.postings().len() != 1 || absorbed.postings().len() != 1 {
        return reject("each transaction must have exactly one posting");
    }
    let (Some(amount_a), Some(amount_b)) = (a.amount(), b.amount()) else {
        return reject("both postings must have a concrete amount");
    };
    if amount_a.commodity() != amount_b.commodity() {
        return reject("postings must share a commodity");
    }
    if amount_a.value().is_zero() {
        return reject("posting amount must be non-zero");
    }
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "financial negation: Decimal is bounded by the type"
    )]
    let opposite = amount_a.value() == -amount_b.value();
    if !opposite {
        return reject("postings must be equal and opposite");
    }
    Ok(())
}

/// Ranks reconciliation states so a merge can keep the most-settled one.
fn reconciliation_rank(state: bc_models::Reconciliation) -> u8 {
    match state {
        bc_models::Reconciliation::Flagged => 1,
        bc_models::Reconciliation::Reconciled => 2,
        // Covers `Unreconciled` and any unrecognized future variant (the enum
        // is `#[non_exhaustive]`): least-settled, so it never wins over a
        // known, more-settled state.
        bc_models::Reconciliation::Unreconciled | _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use rust_decimal::Decimal;

    use super::*;

    fn tx(_account: &str, amount: i64, commodity: &str) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 6, 27))
            .description("row")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .amount(Amount::new(
                        Decimal::from(amount),
                        CommodityCode::new(commodity),
                    ))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn accepts_equal_opposite_same_commodity() {
        check_mergeable(&tx("a", -100, "AUD"), &tx("b", 100, "AUD")).expect("should be mergeable");
    }

    #[test]
    fn rejects_same_sign() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", -100, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_unequal_magnitude() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", 90, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_different_commodity() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", 100, "USD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_zero_amount() {
        assert!(matches!(
            check_mergeable(&tx("a", 0, "AUD"), &tx("b", 0, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }
}

#[cfg(test)]
mod db_tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::SourceRef;
    use bc_models::SourceRefId;
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use super::*;
    use crate::RawTransaction;

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

    /// Creates a single-posting transaction on `acct` for `amount`, with a
    /// source ref scoped to `acct`, and returns its ID.
    async fn leg(pool: &SqlitePool, acct: &AccountId, amount: i64, when: Date) -> TransactionId {
        leg_with(
            pool,
            acct,
            amount,
            when,
            Reconciliation::Reconciled,
            vec![],
            vec![],
        )
        .await
    }

    /// Like [`leg`], but lets the caller control the reconciliation state,
    /// tags, and extra dates attached to the transaction at creation time.
    async fn leg_with(
        pool: &SqlitePool,
        acct: &AccountId,
        amount: i64,
        when: Date,
        reconciliation: Reconciliation,
        tag_ids: Vec<TagId>,
        extra_dates: Vec<(String, Date)>,
    ) -> TransactionId {
        let money = Amount::new(Decimal::from(amount), CommodityCode::new("AUD"));
        let tx_id = TransactionId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(when)
            .description("TRANSFER")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acct.clone())
                    .amount(money.clone())
                    .build(),
            ])
            .reconciliation(reconciliation)
            .tag_ids(tag_ids)
            .extra_dates(extra_dates)
            .created_at(Timestamp::now())
            .build();
        crate::TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create leg");
        let source = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx_id.clone())
            .account_id(acct.clone())
            .date(when)
            .narration("TRANSFER")
            .amount(money)
            .occurrence(0)
            .created_at(Timestamp::now())
            .build();
        crate::SourceService::new(pool.clone())
            .attach(&source)
            .await
            .expect("attach source");
        tx_id
    }

    async fn posting_count(pool: &SqlitePool, tx: &TransactionId) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
            .bind(tx.to_string())
            .fetch_one(pool)
            .await
            .expect("count postings")
    }

    async fn tx_exists(pool: &SqlitePool, tx: &TransactionId) -> bool {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE id = ?")
            .bind(tx.to_string())
            .fetch_one(pool)
            .await
            .expect("count tx");
        n == 1
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_fuses_legs_and_preserves_refs(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let debit = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        let credit = leg(&pool, &mortgage, 100, date(2025, 6, 27)).await;

        let svc = Service::new(pool.clone());
        svc.merge(&debit, &credit).await.expect("merge");

        // Survivor now holds both postings; absorbed is gone.
        assert_eq!(posting_count(&pool, &debit).await, 2);
        assert!(!tx_exists(&pool, &credit).await, "absorbed tx deleted");

        // Both source refs now hang off the survivor.
        let refs = crate::SourceService::new(pool.clone())
            .list_for_transaction(&debit)
            .await
            .expect("list refs");
        assert_eq!(refs.len(), 2);

        // Survivor date is the earliest (the debit date).
        let survivor_date: String =
            sqlx::query_scalar("SELECT date FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("date");
        assert_eq!(survivor_date, "2025-06-26");

        // A TransactionsMerged event was recorded against the survivor.
        let store = crate::SqliteEventStore::new(pool.clone());
        let events = store.replay_for(&debit.to_string()).await.expect("replay");
        assert!(events.iter().any(|e| e.kind == "TransactionsMerged"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_rejects_unbalanced_pair(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let a = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        let b = leg(&pool, &mortgage, 90, date(2025, 6, 27)).await;
        let svc = Service::new(pool.clone());
        assert!(matches!(
            svc.merge(&a, &b).await,
            Err(crate::BcError::NotMergeable { .. })
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reimport_after_merge_is_noop(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let debit = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        let credit = leg(&pool, &mortgage, 100, date(2025, 6, 27)).await;
        Service::new(pool.clone())
            .merge(&debit, &credit)
            .await
            .expect("merge");

        // Re-importing the Mortgage statement row finds its (moved) source ref and skips.
        let txs = crate::TransactionService::new(pool.clone());
        let srcs = crate::SourceService::new(pool.clone());
        let raw = RawTransaction::new(
            date(2025, 6, 27),
            Amount::new(Decimal::from(100_i64), CommodityCode::new("AUD")),
            None,
            None,
            "TRANSFER".to_owned(),
            None,
        );
        let imported = crate::execute_import(&txs, &srcs, &mortgage, &[raw])
            .await
            .expect("reimport");
        assert_eq!(imported, 0, "the moved ref still dedups the mortgage leg");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_keeps_most_settled_reconciliation(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let debit = leg_with(
            &pool,
            &savings,
            -100,
            date(2025, 6, 26),
            Reconciliation::Unreconciled,
            vec![],
            vec![],
        )
        .await;
        let credit = leg_with(
            &pool,
            &mortgage,
            100,
            date(2025, 6, 27),
            Reconciliation::Reconciled,
            vec![],
            vec![],
        )
        .await;

        Service::new(pool.clone())
            .merge(&debit, &credit)
            .await
            .expect("merge");

        let survivor_reconciliation: String =
            sqlx::query_scalar("SELECT reconciliation FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("reconciliation");
        assert_eq!(
            survivor_reconciliation, "reconciled",
            "survivor keeps the most-settled reconciliation state"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_unions_tags_and_dates_without_pk_collision(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;

        let tag_svc = crate::TagService::new(pool.clone());
        let shared_tag = tag_svc
            .create_path(&"shared".parse().expect("valid tag path"))
            .await
            .expect("create shared tag");
        let debit_only_tag = tag_svc
            .create_path(&"debit-only".parse().expect("valid tag path"))
            .await
            .expect("create debit-only tag");
        let credit_only_tag = tag_svc
            .create_path(&"credit-only".parse().expect("valid tag path"))
            .await
            .expect("create credit-only tag");

        // Both legs carry an extra date under the SAME label with DIFFERENT
        // values: without `INSERT OR IGNORE` in `merge`, unioning this onto
        // the survivor violates the `transaction_dates` PK and the merge
        // fails (Fix 1 regression guard).
        let debit = leg_with(
            &pool,
            &savings,
            -100,
            date(2025, 6, 26),
            Reconciliation::Reconciled,
            vec![shared_tag.clone(), debit_only_tag.clone()],
            vec![("value_date".to_owned(), date(2025, 6, 20))],
        )
        .await;
        let credit = leg_with(
            &pool,
            &mortgage,
            100,
            date(2025, 6, 27),
            Reconciliation::Reconciled,
            vec![shared_tag.clone(), credit_only_tag.clone()],
            vec![("value_date".to_owned(), date(2025, 6, 21))],
        )
        .await;

        Service::new(pool.clone())
            .merge(&debit, &credit)
            .await
            .expect("merge should succeed despite the shared value_date label");

        let mut survivor_tags: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM transaction_tags WHERE transaction_id = ?")
                .bind(debit.to_string())
                .fetch_all(&pool)
                .await
                .expect("survivor tags");
        survivor_tags.sort();
        let mut expected_tags = vec![
            shared_tag.to_string(),
            debit_only_tag.to_string(),
            credit_only_tag.to_string(),
        ];
        expected_tags.sort();
        assert_eq!(
            survivor_tags, expected_tags,
            "survivor ends with the unioned tags, shared tag not duplicated"
        );

        let survivor_dates: Vec<(String, String)> =
            sqlx::query_as("SELECT label, date FROM transaction_dates WHERE transaction_id = ?")
                .bind(debit.to_string())
                .fetch_all(&pool)
                .await
                .expect("survivor dates");
        assert_eq!(
            survivor_dates,
            vec![("value_date".to_owned(), "2025-06-20".to_owned())],
            "survivor keeps its own value_date on a label collision; absorbed's duplicate is dropped"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_then_unmerge_restores_state(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let debit = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        let credit = leg(&pool, &mortgage, 100, date(2025, 6, 27)).await;

        let svc = Service::new(pool.clone());
        svc.merge(&debit, &credit).await.expect("merge");
        let restored = svc.unmerge(&debit).await.expect("unmerge");
        assert_eq!(restored, credit, "the original absorbed id is restored");

        // Both transactions are back to a single posting each.
        assert_eq!(posting_count(&pool, &debit).await, 1);
        assert_eq!(posting_count(&pool, &credit).await, 1);
        assert!(tx_exists(&pool, &credit).await, "absorbed tx recreated");

        // The survivor's date is restored to its pre-merge value.
        let survivor_date: String =
            sqlx::query_scalar("SELECT date FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("date");
        assert_eq!(survivor_date, "2025-06-26");

        // Each transaction owns exactly its own source ref again.
        let srcs = crate::SourceService::new(pool.clone());
        assert_eq!(srcs.list_for_transaction(&debit).await.expect("d").len(), 1);
        assert_eq!(
            srcs.list_for_transaction(&credit).await.expect("c").len(),
            1
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unmerge_restores_exact_pre_merge_tags_and_dates(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;

        let tag_svc = crate::TagService::new(pool.clone());
        let shared_tag = tag_svc
            .create_path(&"shared".parse().expect("valid tag path"))
            .await
            .expect("create shared tag");
        let debit_only_tag = tag_svc
            .create_path(&"debit-only".parse().expect("valid tag path"))
            .await
            .expect("create debit-only tag");
        let credit_only_tag = tag_svc
            .create_path(&"credit-only".parse().expect("valid tag path"))
            .await
            .expect("create credit-only tag");

        // Each leg carries a distinct tag, a shared tag, and an extra date
        // under the SAME label but a DIFFERENT value, so the round-trip can
        // distinguish "restored correctly" from "coincidentally overlapped".
        let debit = leg_with(
            &pool,
            &savings,
            -100,
            date(2025, 6, 26),
            Reconciliation::Reconciled,
            vec![shared_tag.clone(), debit_only_tag.clone()],
            vec![("value_date".to_owned(), date(2025, 6, 20))],
        )
        .await;
        let credit = leg_with(
            &pool,
            &mortgage,
            100,
            date(2025, 6, 27),
            Reconciliation::Reconciled,
            vec![shared_tag.clone(), credit_only_tag.clone()],
            vec![("value_date".to_owned(), date(2025, 6, 21))],
        )
        .await;

        let svc = Service::new(pool.clone());
        svc.merge(&debit, &credit).await.expect("merge");
        let restored = svc.unmerge(&debit).await.expect("unmerge");
        assert_eq!(restored, credit, "the original absorbed id is restored");

        let mut survivor_tags: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM transaction_tags WHERE transaction_id = ?")
                .bind(debit.to_string())
                .fetch_all(&pool)
                .await
                .expect("survivor tags");
        survivor_tags.sort();
        let mut expected_survivor_tags = vec![shared_tag.to_string(), debit_only_tag.to_string()];
        expected_survivor_tags.sort();
        assert_eq!(
            survivor_tags, expected_survivor_tags,
            "survivor's tags are restored to exactly its pre-merge set"
        );

        let mut survivor_dates: Vec<(String, String)> =
            sqlx::query_as("SELECT label, date FROM transaction_dates WHERE transaction_id = ?")
                .bind(debit.to_string())
                .fetch_all(&pool)
                .await
                .expect("survivor dates");
        survivor_dates.sort();
        assert_eq!(
            survivor_dates,
            vec![("value_date".to_owned(), "2025-06-20".to_owned())],
            "survivor's extra dates are restored to exactly its pre-merge set, \
             not the absorbed's overlapping-label value"
        );

        let mut absorbed_tags: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM transaction_tags WHERE transaction_id = ?")
                .bind(credit.to_string())
                .fetch_all(&pool)
                .await
                .expect("absorbed tags");
        absorbed_tags.sort();
        let mut expected_absorbed_tags = vec![shared_tag.to_string(), credit_only_tag.to_string()];
        expected_absorbed_tags.sort();
        assert_eq!(
            absorbed_tags, expected_absorbed_tags,
            "absorbed's tags are restored to exactly its pre-merge set"
        );

        let mut absorbed_dates: Vec<(String, String)> =
            sqlx::query_as("SELECT label, date FROM transaction_dates WHERE transaction_id = ?")
                .bind(credit.to_string())
                .fetch_all(&pool)
                .await
                .expect("absorbed dates");
        absorbed_dates.sort();
        assert_eq!(
            absorbed_dates,
            vec![("value_date".to_owned(), "2025-06-21".to_owned())],
            "absorbed's extra dates are restored to exactly its pre-merge set"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unmerge_without_history_errors(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let lone = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        assert!(matches!(
            Service::new(pool.clone()).unmerge(&lone).await,
            Err(crate::BcError::NotMerged(_))
        ));
    }
}
