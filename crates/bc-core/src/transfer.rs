//! Transfer resolution: merge/unmerge two single-posting transactions and
//! suggest candidate transfer pairs.

use bc_models::Amount;
use bc_models::SourceRefId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Unit;
use jiff::civil::Date;
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
    /// Survivor's reconciliation state before the merge.
    reconciliation: bc_models::Reconciliation,
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
            // `check_mergeable` guarantees the absorbed transaction has exactly one
            // posting, and postings are positioned by enumeration index, so the
            // lone posting is always at position 0.
            posting_position: 0,
            source_ref_ids,
        };
        let event = crate::Event::TransactionsMerged {
            survivor_id: survivor_id.clone(),
            absorbed: snapshot,
            survivor_date_before: survivor.date(),
            survivor_tags_before: survivor.tag_ids().to_vec(),
            survivor_extra_dates_before: survivor.extra_dates().to_vec(),
            survivor_reconciliation_before: survivor.reconciliation(),
        };

        let survivor_str = survivor_id.to_string();
        let absorbed_str = absorbed_id.to_string();
        let mut db_tx = self.pool.begin().await?;

        // The mergeability checks above ran on separate connections before this
        // transaction began. Re-assert inside it that both legs still hold exactly
        // one posting, so two concurrent merges into the same survivor cannot both
        // proceed and leave it with a duplicated posting.
        let survivor_now: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
                .bind(&survivor_str)
                .fetch_one(&mut *db_tx)
                .await?;
        let absorbed_now: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
                .bind(&absorbed_str)
                .fetch_one(&mut *db_tx)
                .await?;
        if survivor_now != survivor_posting_count || absorbed_now != 1 {
            return Err(BcError::NotMergeable {
                reason: "a concurrent change altered the postings; retry the merge".to_owned(),
            });
        }

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

        // Union the absorbed transaction's tags and dates onto the survivor. The
        // `transaction_dates` primary key is `(transaction_id, label)`, so a label
        // present on both legs cannot hold two dates: `INSERT OR IGNORE` keeps the
        // survivor's existing date and drops the absorbed leg's value for that
        // label (the union is over labels, not `(label, date)` pairs).
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
    /// Recreates the absorbed transaction with its original ID and moves its
    /// posting and source references back. The survivor is reverted only where the
    /// merge changed it: the tags and dates the merge added are removed (edits made
    /// while merged are preserved), and its date/reconciliation are restored only
    /// if they still hold the values the merge wrote. Records a
    /// [`crate::Event::TransactionUnmerged`].
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
                        survivor_reconciliation_before,
                        ..
                    } = event
                    {
                        stack.push(absorbed);
                        snapshots.push(SurvivorSnapshot {
                            date: survivor_date_before,
                            tags: survivor_tags_before,
                            extra_dates: survivor_extra_dates_before,
                            reconciliation: survivor_reconciliation_before,
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

        // Reverse only what the merge added to the survivor, so edits made while
        // merged survive. The merge unioned the absorbed leg's tags/dates onto the
        // survivor (`INSERT OR IGNORE`), so the rows it introduced are exactly the
        // absorbed keys that were absent from the survivor's pre-merge snapshot;
        // remove those and leave everything else (original + intervening edits).
        for tag_id in &absorbed.tag_ids {
            if snapshot.tags.contains(tag_id) {
                continue;
            }
            sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?")
                .bind(&survivor_str)
                .bind(tag_id.to_string())
                .execute(&mut *db_tx)
                .await?;
        }
        for (label, _) in &absorbed.extra_dates {
            if snapshot.extra_dates.iter().any(|(l, _)| l == label) {
                continue;
            }
            sqlx::query("DELETE FROM transaction_dates WHERE transaction_id = ? AND label = ?")
                .bind(&survivor_str)
                .bind(label)
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

        // Restore the survivor's pre-merge date and reconciliation, but only if the
        // field still holds the value the merge wrote — otherwise the user changed
        // it while merged and that edit must be preserved. The guarded `WHERE`
        // makes each restore a no-op when the current value has moved on.
        let merged_date = snapshot.date.min(absorbed.date);
        sqlx::query("UPDATE transactions SET date = ? WHERE id = ? AND date = ?")
            .bind(snapshot.date.to_string())
            .bind(&survivor_str)
            .bind(merged_date.to_string())
            .execute(&mut *db_tx)
            .await?;
        let merged_reconciliation = if reconciliation_rank(absorbed.reconciliation)
            > reconciliation_rank(snapshot.reconciliation)
        {
            absorbed.reconciliation
        } else {
            snapshot.reconciliation
        };
        sqlx::query(
            "UPDATE transactions SET reconciliation = ? WHERE id = ? AND reconciliation = ?",
        )
        .bind(crate::db::to_db_str(snapshot.reconciliation)?)
        .bind(&survivor_str)
        .bind(crate::db::to_db_str(merged_reconciliation)?)
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        Ok(absorbed.id)
    }

    /// Suggests candidate transfer pairs among single-posting transactions.
    ///
    /// Loads every transaction with exactly one concrete posting and pairs them
    /// via [`match_transfers`]. Already-merged transactions (two or more
    /// postings) are naturally excluded.
    ///
    /// # Returns
    ///
    /// Proposed transfer pairs for user confirmation.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on a database or data-parse failure.
    #[inline]
    pub async fn suggest_transfers(&self) -> BcResult<Vec<TransferSuggestion>> {
        let rows: Vec<(String, String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT t.id, p.account_id, a.name, t.description, p.amount, p.commodity, t.date \
             FROM transactions t \
             JOIN postings p ON p.transaction_id = t.id \
             JOIN accounts a ON a.id = p.account_id \
             WHERE t.id IN (SELECT transaction_id FROM postings GROUP BY transaction_id HAVING COUNT(*) = 1) \
               AND p.amount IS NOT NULL AND p.commodity IS NOT NULL \
             ORDER BY t.date, t.id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut candidates = Vec::with_capacity(rows.len());
        for (raw_id, raw_account, account_name, narration, raw_amount, commodity, raw_date) in rows
        {
            let id = raw_id
                .parse::<TransactionId>()
                .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))?;
            let account = raw_account
                .parse::<bc_models::AccountId>()
                .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))?;
            let value = raw_amount
                .parse::<rust_decimal::Decimal>()
                .map_err(|e| BcError::BadData(e.to_string()))?;
            let date = raw_date
                .parse::<Date>()
                .map_err(|e| BcError::BadData(e.to_string()))?;
            candidates.push(Candidate {
                id,
                account,
                account_name,
                amount: Amount::new(value, bc_models::CommodityCode::new(commodity)),
                date,
                narration,
            });
        }
        Ok(match_transfers(&candidates))
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

/// A proposed transfer pair: two opposite legs likely to be the same movement.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransferSuggestion {
    /// The outgoing (debit) leg.
    pub debit: TransactionId,
    /// The incoming (credit) leg.
    pub credit: TransactionId,
    /// The transfer magnitude (absolute value, credit side).
    pub amount: Amount,
    /// The debit leg's value date.
    pub date_debit: Date,
    /// The credit leg's value date.
    pub date_credit: Date,
    /// Display name of the debit leg's account.
    pub debit_account: String,
    /// Display name of the credit leg's account.
    pub credit_account: String,
    /// The debit leg's bank narration (its transaction description).
    pub debit_narration: String,
    /// The credit leg's bank narration (its transaction description).
    pub credit_narration: String,
}

impl TransferSuggestion {
    /// Returns the debit (outgoing) leg's transaction ID.
    #[must_use]
    #[inline]
    pub fn debit(&self) -> TransactionId {
        self.debit.clone()
    }

    /// Returns the credit (incoming) leg's transaction ID.
    #[must_use]
    #[inline]
    pub fn credit(&self) -> TransactionId {
        self.credit.clone()
    }
}

/// A single-posting transaction considered as a merge candidate.
#[derive(Debug, Clone)]
struct Candidate {
    /// The transaction ID.
    id: TransactionId,
    /// The account the lone posting debits or credits.
    account: bc_models::AccountId,
    /// The display name of that account.
    account_name: String,
    /// The lone posting's signed amount.
    amount: Amount,
    /// The transaction's value date.
    date: Date,
    /// The transaction's bank narration (its description column).
    narration: String,
}

/// Pairs candidates that look like the two legs of one transfer.
///
/// Two candidates pair when they debit and credit **different** accounts; their
/// amounts are equal in magnitude, opposite in sign, and share a commodity; the
/// debit (negative) leg is dated on-or-before the credit (positive) leg; and the
/// two dates are within seven days.
///
/// Matching is greedy and one-to-one: candidates are visited in the caller's
/// order and each is paired at most once, so a leg never appears in more than one
/// suggestion (avoiding a combinatorial blow-up when several equal transfers
/// share a window).
///
/// # Arguments
///
/// * `candidates` - Single-posting transactions to consider, in a stable order.
///
/// # Returns
///
/// One [`TransferSuggestion`] per qualifying pair.
#[must_use]
fn match_transfers(candidates: &[Candidate]) -> Vec<TransferSuggestion> {
    let mut out = Vec::new();
    let mut paired: std::collections::HashSet<&TransactionId> = std::collections::HashSet::new();
    for (i, a) in candidates.iter().enumerate() {
        if paired.contains(&a.id) {
            continue;
        }
        for b in candidates.iter().skip(i.saturating_add(1)) {
            if paired.contains(&b.id) {
                continue;
            }
            // A transfer moves money between two different accounts; an
            // opposite-sign pair within one account (e.g. a charge and its
            // refund) is not a transfer.
            if a.account == b.account {
                continue;
            }
            if a.amount.commodity() != b.amount.commodity() {
                continue;
            }
            if a.amount.value().is_zero() {
                continue;
            }
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "financial negation: Decimal is bounded by the type"
            )]
            let opposite = a.amount.value() == -b.amount.value();
            if !opposite {
                continue;
            }
            let (debit, credit) = if a.amount.value().is_sign_negative() {
                (a, b)
            } else {
                (b, a)
            };
            // Debit on-or-before credit, within 7 days.
            let days = debit
                .date
                .until((Unit::Day, credit.date))
                .map_or(i64::MIN, |span| i64::from(span.get_days()));
            if !(0..=7).contains(&days) {
                continue;
            }
            out.push(TransferSuggestion {
                debit: debit.id.clone(),
                credit: credit.id.clone(),
                amount: credit.amount.clone(),
                date_debit: debit.date,
                date_credit: credit.date,
                debit_account: debit.account_name.clone(),
                credit_account: credit.account_name.clone(),
                debit_narration: debit.narration.clone(),
                credit_narration: credit.narration.clone(),
            });
            paired.insert(&a.id);
            paired.insert(&b.id);
            break;
        }
    }
    out
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
    use crate::RawPosting;
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
        let posting_id = PostingId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(when)
            .description("TRANSFER")
            .postings(vec![
                Posting::builder()
                    .id(posting_id.clone())
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
            .posting_id(Some(posting_id))
            .account_id(acct.clone())
            .date(when)
            .narration("TRANSFER")
            .amount(Some(money))
            .occurrence(0)
            .owns_posting(false)
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
        let accts = crate::AccountService::new(pool.clone());
        let batches = crate::ImportBatchService::new(pool.clone());
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("TRANSFER")
            .postings(vec![
                RawPosting::builder()
                    .account("Mortgage")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(100_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
            ])
            .build();
        let outcome = crate::execute_import(&txs, &srcs, &accts, &batches, None, "test", &[raw])
            .await
            .expect("reimport");
        // Without these two, the assertion below would pass just as well if the
        // account path had failed to resolve and the leg had been silently
        // skipped — proving nothing about the moved reference.
        assert!(
            outcome.unresolved_accounts.is_empty(),
            "the Mortgage path must resolve, or the dedup claim is vacuous"
        );
        assert_eq!(outcome.skipped_postings, 0, "the leg reached the matcher");
        assert_eq!(
            outcome.new_transactions, 0,
            "the moved ref still dedups the mortgage leg"
        );
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
    async fn unmerge_restores_survivor_reconciliation(pool: SqlitePool) {
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

        let svc = Service::new(pool.clone());
        svc.merge(&debit, &credit).await.expect("merge");

        let merged_reconciliation: String =
            sqlx::query_scalar("SELECT reconciliation FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("reconciliation after merge");
        assert_eq!(
            merged_reconciliation, "reconciled",
            "merge keeps the most-settled reconciliation state"
        );

        svc.unmerge(&debit).await.expect("unmerge");

        let restored_reconciliation: String =
            sqlx::query_scalar("SELECT reconciliation FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("reconciliation after unmerge");
        assert_eq!(
            restored_reconciliation, "unreconciled",
            "unmerge restores the survivor's exact pre-merge reconciliation state"
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
    async fn unmerge_preserves_edits_made_while_merged(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;

        let tag_svc = crate::TagService::new(pool.clone());
        let debit_tag = tag_svc
            .create_path(&"debit-only".parse().expect("valid tag path"))
            .await
            .expect("create debit tag");
        let credit_tag = tag_svc
            .create_path(&"credit-only".parse().expect("valid tag path"))
            .await
            .expect("create credit tag");
        let reviewed_tag = tag_svc
            .create_path(&"reviewed".parse().expect("valid tag path"))
            .await
            .expect("create reviewed tag");

        let debit = leg_with(
            &pool,
            &savings,
            -100,
            date(2025, 6, 26),
            Reconciliation::Unreconciled,
            vec![debit_tag.clone()],
            vec![],
        )
        .await;
        let credit = leg_with(
            &pool,
            &mortgage,
            100,
            date(2025, 6, 27),
            Reconciliation::Unreconciled,
            vec![credit_tag.clone()],
            vec![],
        )
        .await;

        let svc = Service::new(pool.clone());
        svc.merge(&debit, &credit).await.expect("merge");

        // Simulate the user editing the survivor while it is merged: add a new tag
        // and re-date it (the merge had set the date to the 6-26 debit date).
        sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
            .bind(debit.to_string())
            .bind(reviewed_tag.to_string())
            .execute(&pool)
            .await
            .expect("add intervening tag");
        sqlx::query("UPDATE transactions SET date = ? WHERE id = ?")
            .bind("2025-07-01")
            .bind(debit.to_string())
            .execute(&pool)
            .await
            .expect("re-date survivor");

        svc.unmerge(&debit).await.expect("unmerge");

        let mut survivor_tags: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM transaction_tags WHERE transaction_id = ?")
                .bind(debit.to_string())
                .fetch_all(&pool)
                .await
                .expect("survivor tags");
        survivor_tags.sort();
        let mut expected = vec![debit_tag.to_string(), reviewed_tag.to_string()];
        expected.sort();
        assert_eq!(
            survivor_tags, expected,
            "unmerge removes the merge-added tag but keeps the survivor's own tag \
             and the tag added while merged"
        );

        let survivor_date: String =
            sqlx::query_scalar("SELECT date FROM transactions WHERE id = ?")
                .bind(debit.to_string())
                .fetch_one(&pool)
                .await
                .expect("survivor date");
        assert_eq!(
            survivor_date, "2025-07-01",
            "unmerge preserves a date the user changed while merged"
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

    #[sqlx::test(migrations = "./migrations")]
    async fn suggest_finds_the_pair(pool: SqlitePool) {
        let savings = account(&pool, "Savings").await;
        let mortgage = account(&pool, "Mortgage").await;
        let debit = leg(&pool, &savings, -100, date(2025, 6, 26)).await;
        let credit = leg(&pool, &mortgage, 100, date(2025, 6, 27)).await;

        let suggestions = Service::new(pool.clone())
            .suggest_transfers()
            .await
            .expect("suggest");
        assert_eq!(suggestions.len(), 1);
        let s = suggestions.first().expect("one");
        assert_eq!(s.debit(), debit);
        assert_eq!(s.credit(), credit);
        assert_eq!(s.debit_account, "Savings");
        assert_eq!(s.credit_account, "Mortgage");
        assert_eq!(s.debit_narration, "TRANSFER");
        assert_eq!(s.credit_narration, "TRANSFER");
    }
}

#[cfg(test)]
mod suggest_tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::TransactionId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::Candidate;
    use super::match_transfers;

    fn cand(amount: i64, when: (i16, i8, i8)) -> Candidate {
        // Each candidate lands in its own account so opposite-sign pairs are
        // treated as cross-account transfers unless a test shares an account.
        cand_in(amount, when, bc_models::AccountId::new())
    }

    fn cand_in(amount: i64, when: (i16, i8, i8), account: bc_models::AccountId) -> Candidate {
        Candidate {
            id: TransactionId::new(),
            account,
            account_name: "Account".to_owned(),
            amount: Amount::new(Decimal::from(amount), CommodityCode::new("AUD")),
            date: date(when.0, when.1, when.2),
            narration: "TRANSFER".to_owned(),
        }
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn pairs_equal_opposite_within_window() {
        let debit = cand(-100, (2025, 6, 26));
        let credit = cand(100, (2025, 6, 27));
        let out = match_transfers(&[debit.clone(), credit.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].debit(), debit.id);
        assert_eq!(out[0].credit(), credit.id);
    }

    #[test]
    fn skips_when_debit_after_credit() {
        // Negative (debit) dated AFTER the positive (credit) — violates ordering.
        let out = match_transfers(&[cand(-100, (2025, 6, 28)), cand(100, (2025, 6, 27))]);
        assert!(out.is_empty());
    }

    #[test]
    fn skips_beyond_seven_days() {
        let out = match_transfers(&[cand(-100, (2025, 6, 1)), cand(100, (2025, 6, 20))]);
        assert!(out.is_empty());
    }

    #[test]
    fn skips_same_sign_or_unequal() {
        assert!(
            match_transfers(&[cand(-100, (2025, 6, 26)), cand(-100, (2025, 6, 27))]).is_empty()
        );
        assert!(match_transfers(&[cand(-100, (2025, 6, 26)), cand(90, (2025, 6, 27))]).is_empty());
    }

    #[test]
    fn skips_opposite_legs_in_the_same_account() {
        // A charge and its refund within one account are equal-and-opposite but
        // are not a transfer between accounts.
        let account = bc_models::AccountId::new();
        let out = match_transfers(&[
            cand_in(-100, (2025, 6, 26), account.clone()),
            cand_in(100, (2025, 6, 27), account),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn pairs_each_leg_at_most_once() {
        // Two identical debits and two identical credits, all in-window: greedy
        // one-to-one matching yields two suggestions, not the four an all-pairs
        // matcher would emit.
        let out = match_transfers(&[
            cand(-100, (2025, 6, 26)),
            cand(-100, (2025, 6, 26)),
            cand(100, (2025, 6, 27)),
            cand(100, (2025, 6, 27)),
        ]);
        assert_eq!(out.len(), 2);
        let debits: std::collections::HashSet<_> =
            out.iter().map(super::TransferSuggestion::debit).collect();
        let credits: std::collections::HashSet<_> =
            out.iter().map(super::TransferSuggestion::credit).collect();
        assert_eq!(debits.len(), 2, "each debit used once");
        assert_eq!(credits.len(), 2, "each credit used once");
    }
}
