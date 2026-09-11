//! Discarding an import batch: undoing one import run.
//!
//! A discarded batch is a run that never happened. Every reference it wrote is
//! hard-deleted — freeing its `(account_id, fingerprint, occurrence)` slot, the
//! opposite of the tombstone a deleted leg leaves — every posting it created
//! goes with them, every transaction left holding no postings goes too, and
//! every tag it created goes unless something else has since named it: a
//! membership added by hand or by a later run, a budget filter, or a child tag
//! that stays. A tag the user renamed but never applied is not recognised as
//! curated and goes with the run. Only a posting the run *adopted* survives,
//! losing its provenance.
//!
//! The work is driven by the reference rows, never by the batch's recorded
//! counts, so a run that aborted before recording anything discards exactly as
//! correctly as one that completed.
//!
//! A batch a later run built on is refused. When a later, undiscarded batch
//! owns a live leg on a transaction this batch owns a live leg on, removing
//! this batch's leg would leave the later one describing money from nowhere;
//! the error names the later batches, newest first, which is the order to
//! discard them in. A later run that merely *adopted* one of this batch's
//! legs is no dependant: its reference becomes a tombstone, which the outcome
//! reports.

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
    /// Any other reference left naming a posting this discard deleted, whose
    /// transaction survived. It becomes a tombstone rather than going away, so
    /// unlike [`Self::other_batch_references_removed`] it keeps its slot.
    pub other_batch_references_tombstoned: usize,
    /// Of [`Self::removed_postings`], those whose account, amount or commodity
    /// no longer matched the reference describing them — the user had edited
    /// them.
    pub edited_postings: usize,
    /// Of [`Self::removed_postings`], those in a transaction the user had
    /// reconciled against a statement.
    pub reconciled_postings: usize,
    /// Of [`Self::removed_postings`], those in a transaction the user had
    /// flagged for review. Counted apart from [`Self::reconciled_postings`]:
    /// losing a flag and losing a statement confirmation are different losses.
    pub flagged_postings: usize,
    /// Tags the run created that nothing else names, deleted with it.
    pub removed_tags: usize,
    /// Tags the run created that have since been applied elsewhere — by hand,
    /// by a later run, as a budget filter, or as the parent of a kept tag —
    /// and therefore stay.
    pub kept_tags: usize,
}

/// A later batch that owns legs on a batch's transactions, blocking its
/// discard.
///
/// Re-exported from the crate root as [`crate::DiscardDependant`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependant {
    /// The later batch.
    pub batch_id: ImportBatchId,
    /// Its importer, for the diagnostic.
    pub importer: String,
    /// When it started; later than the blocked batch by construction.
    pub started_at: Timestamp,
    /// Live postings it owns on the blocked batch's transactions.
    pub postings: usize,
    /// Distinct such transactions.
    pub transactions: usize,
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
/// [`BcError::DiscardBlocked`] if a later batch owns legs on its transactions,
/// [`BcError::BadData`] if a count exceeds `u64` or a stored timestamp will not
/// parse, and [`BcError::Database`] on any query failure.
pub(crate) async fn discard(pool: &SqlitePool, id: &ImportBatchId) -> BcResult<Outcome> {
    let id_str = id.to_string();
    let mut db_tx = pool.begin().await?;

    ensure_discardable(&mut db_tx, id).await?;
    let plan = Plan::read(&mut db_tx, &id_str).await?;
    // Counted while the rows still exist: each compares a posting against the
    // reference describing it, and the references are about to be deleted.
    let edits = count_edits(&mut db_tx, &id_str).await?;

    // Free the slots first. Deleting the postings first would only make the
    // ON DELETE SET NULL clause churn rows that are about to vanish.
    sqlx::query("DELETE FROM transaction_sources WHERE import_batch_id = ?")
        .bind(&id_str)
        .execute(&mut *db_tx)
        .await?;

    // Read once this batch's own references are gone, so every row it names
    // belongs to someone else. Those on a swept transaction disappear with it;
    // the rest are still here afterwards, as tombstones.
    let collateral = collateral_references(&mut db_tx, &plan.owned_postings).await?;

    delete_postings(&mut db_tx, &plan.owned_postings).await?;
    let swept = sweep_empty_transactions(&mut db_tx, &plan.touched).await?;
    renumber_positions(&mut db_tx, &swept.survivors).await?;
    // After the postings and transactions, so a membership that went with them
    // no longer counts as naming the tag.
    let reversed = reverse_tags(&mut db_tx, &id_str).await?;

    let outcome = Outcome {
        batch_id: id.clone(),
        removed_postings: plan.owned_postings.len(),
        removed_transactions: swept.removed_transactions,
        detached_adopted: plan.detached_adopted,
        freed_tombstones: plan.freed_tombstones,
        other_batch_references_removed: swept.other_batch_references_removed,
        other_batch_references_tombstoned: count_surviving(&mut db_tx, &collateral).await?,
        edited_postings: edits.edited,
        reconciled_postings: edits.reconciled,
        flagged_postings: edits.flagged,
        removed_tags: reversed.removed,
        kept_tags: reversed.kept,
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
        other_batch_references_tombstoned = outcome.other_batch_references_tombstoned,
        edited_postings = outcome.edited_postings,
        reconciled_postings = outcome.reconciled_postings,
        flagged_postings = outcome.flagged_postings,
        removed_tags = outcome.removed_tags,
        kept_tags = outcome.kept_tags,
        "import batch discarded"
    );
    Ok(outcome)
}

/// Checks that a batch exists, has not already been discarded, and has no
/// later batch built on it.
///
/// A batch that does not exist cannot be discarded, and one already discarded
/// must not be discarded again: the second run would find no references and
/// silently report having removed nothing. One a later run added legs to is
/// refused so that run's legs are not left without their funding side; see
/// the module doc.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id` - The batch being discarded, both for the lookup and the error
///   messages. Taking one value rather than an ID and a pre-rendered string
///   leaves no way for the row checked and the batch named to disagree.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if no batch with that ID exists,
/// [`BcError::InvalidInput`] if it has already been discarded,
/// [`BcError::DiscardBlocked`] if a later batch owns legs on its transactions,
/// [`BcError::BadData`] if a stored timestamp or ID will not parse, and
/// [`BcError::Database`] on query failure.
pub(crate) async fn ensure_discardable(
    conn: &mut sqlx::SqliteConnection,
    id: &ImportBatchId,
) -> BcResult<()> {
    let id_str = id.to_string();
    let existing: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT discarded_at, started_at FROM import_batches WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(&mut *conn)
            .await?;
    let started_at = match existing {
        None => return Err(BcError::NotFound(format!("import batch {id}"))),
        Some((Some(_), _)) => return Err(already_discarded_error(id)),
        Some((None, raw)) => parse_timestamp(&raw)?,
    };

    let dependants = dependants(conn, &id_str, started_at).await?;
    if dependants.is_empty() {
        Ok(())
    } else {
        Err(BcError::DiscardBlocked {
            batch: id.clone(),
            dependants,
        })
    }
}

/// Finds the later, undiscarded batches that own live legs on transactions
/// this batch owns live legs on.
///
/// The ordering runs in Rust rather than SQL: `started_at` is stored as text
/// with a fractional second only when it is non-zero, so `2026-01-01T00:00:00Z`
/// sorts *after* `2026-01-01T00:00:00.5Z` as a string.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id_str` - The batch's ID as stored.
/// * `started_at` - When that batch started; only batches started after it
///   count.
///
/// # Returns
///
/// The dependants, newest first.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure and [`BcError::BadData`] if
/// a stored ID or timestamp will not parse.
async fn dependants(
    conn: &mut sqlx::SqliteConnection,
    id_str: &str,
    started_at: Timestamp,
) -> BcResult<Vec<Dependant>> {
    // Self-join on the transaction: a leg this batch owns beside a live leg
    // another live batch owns. DISTINCT on the reference, since this batch
    // owning two legs on one transaction would otherwise double the other
    // side's count.
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT b.id, b.importer, b.started_at, \
                COUNT(DISTINCT theirs.id), COUNT(DISTINCT theirs.transaction_id) \
         FROM transaction_sources mine \
         JOIN transaction_sources theirs ON theirs.transaction_id = mine.transaction_id \
         JOIN import_batches b ON b.id = theirs.import_batch_id \
         WHERE mine.import_batch_id = ?1 \
           AND mine.owns_posting = 1 AND mine.posting_id IS NOT NULL \
           AND theirs.import_batch_id <> ?1 \
           AND theirs.owns_posting = 1 AND theirs.posting_id IS NOT NULL \
           AND b.discarded_at IS NULL \
         GROUP BY b.id",
    )
    .bind(id_str)
    .fetch_all(conn)
    .await?;

    let mut out = Vec::new();
    for (raw_id, importer, raw_started, postings, transactions) in rows {
        let other_started = parse_timestamp(&raw_started)?;
        if other_started <= started_at {
            continue;
        }
        out.push(Dependant {
            batch_id: raw_id
                .parse::<ImportBatchId>()
                .map_err(|e| BcError::BadData(e.to_string()))?,
            importer,
            started_at: other_started,
            postings: to_usize(postings),
            transactions: to_usize(transactions),
        });
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.started_at));
    Ok(out)
}

/// Parses a stored timestamp.
///
/// # Arguments
///
/// * `raw` - The column value.
///
/// # Returns
///
/// The timestamp.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if it will not parse.
fn parse_timestamp(raw: &str) -> BcResult<Timestamp> {
    raw.parse::<Timestamp>()
        .map_err(|e| BcError::BadData(e.to_string()))
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

/// Curation the discard is about to destroy, counted per posting.
struct Edits {
    /// Postings that no longer agree with the reference describing them.
    edited: usize,
    /// Postings in a transaction reconciled against a statement.
    reconciled: usize,
    /// Postings in a transaction flagged for review.
    flagged: usize,
}

/// Counts the batch's own postings the user has since edited, reconciled or
/// flagged.
///
/// A posting is "edited" when it no longer agrees with the reference describing
/// it: the reference records what the document said and never moves, so a
/// corrected amount or a recategorisation shows up as a disagreement.
///
/// A NULL on the reference is not a disagreement. It means the document stated
/// no amount, and an elided leg is then given the document's residual at import
/// time — so a naive null-safe comparison reports every such leg as edited by a
/// user who never touched it. Only a reference that recorded a value can
/// evidence a change to it; `IS NOT` still catches a value the user has since
/// cleared.
///
/// Reconciled and flagged are counted apart. Both mean the user did something
/// deliberate to the transaction, but only the first means it was confirmed
/// against a statement, and the report says so in those words.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id_str` - The batch's ID as stored.
///
/// # Returns
///
/// The [`Edits`] the discard is about to destroy.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn count_edits(conn: &mut sqlx::SqliteConnection, id_str: &str) -> BcResult<Edits> {
    let edited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transaction_sources s \
         JOIN postings p ON p.id = s.posting_id \
         WHERE s.import_batch_id = ? AND s.owns_posting = 1 \
           AND (p.account_id <> s.account_id \
                OR (s.amount IS NOT NULL AND p.amount IS NOT s.amount) \
                OR (s.commodity IS NOT NULL AND p.commodity IS NOT s.commodity))",
    )
    .bind(id_str)
    .fetch_one(&mut *conn)
    .await?;

    let reconciled = count_by_reconciliation(&mut *conn, id_str, "reconciled").await?;
    let flagged = count_by_reconciliation(conn, id_str, "flagged").await?;

    Ok(Edits {
        edited: to_usize(edited),
        reconciled,
        flagged,
    })
}

/// Counts the batch's own postings sitting in a transaction in the given
/// reconciliation state.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id_str` - The batch's ID as stored.
/// * `state` - The stored `reconciliation` value to match.
///
/// # Returns
///
/// The number of the batch's own postings in that state.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn count_by_reconciliation(
    conn: &mut sqlx::SqliteConnection,
    id_str: &str,
    state: &str,
) -> BcResult<usize> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transaction_sources s \
         JOIN postings p ON p.id = s.posting_id \
         JOIN transactions t ON t.id = p.transaction_id \
         WHERE s.import_batch_id = ? AND s.owns_posting = 1 \
           AND t.reconciliation = ?",
    )
    .bind(id_str)
    .bind(state)
    .fetch_one(conn)
    .await?;

    Ok(to_usize(count))
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
        sqlx::query("DELETE FROM posting_metadata WHERE posting_id = ?")
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

/// Collects the references — none of them this batch's, whose own are already
/// deleted by the time this runs — still naming a posting the discard is about
/// to remove.
///
/// Read before the postings go, because afterwards `ON DELETE SET NULL` has
/// blanked the link that identifies them and they are indistinguishable from
/// tombstones that were already there.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `posting_ids` - The postings about to be deleted.
///
/// # Returns
///
/// The IDs of those references.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn collateral_references(
    conn: &mut sqlx::SqliteConnection,
    posting_ids: &[String],
) -> BcResult<Vec<String>> {
    let mut ids = Vec::new();
    for posting_id in posting_ids {
        let mut found: Vec<String> =
            sqlx::query_scalar("SELECT id FROM transaction_sources WHERE posting_id = ?")
                .bind(posting_id)
                .fetch_all(&mut *conn)
                .await?;
        ids.append(&mut found);
    }
    Ok(ids)
}

/// Counts how many of the given references are still present.
///
/// Run after the sweep: one that outlived it sits on a surviving transaction
/// and is now a tombstone, where one that did not went with its transaction and
/// is already counted as removed.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `reference_ids` - The references to look for.
///
/// # Returns
///
/// How many of them still exist.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn count_surviving(
    conn: &mut sqlx::SqliteConnection,
    reference_ids: &[String],
) -> BcResult<usize> {
    let mut surviving: usize = 0;
    for reference_id in reference_ids {
        let present: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM transaction_sources WHERE id = ?")
                .bind(reference_id)
                .fetch_one(&mut *conn)
                .await?;
        if present > 0 {
            surviving = surviving.saturating_add(1);
        }
    }
    Ok(surviving)
}

/// What the sweep did.
struct Swept {
    /// Transactions deleted for holding no postings.
    removed_transactions: usize,
    /// Other references that went with them.
    other_batch_references_removed: usize,
    /// Touched transactions that still stand.
    survivors: Vec<String>,
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
/// What the sweep removed, and which of the touched transactions survived.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query or delete failure.
async fn sweep_empty_transactions(
    conn: &mut sqlx::SqliteConnection,
    transaction_ids: &BTreeSet<String>,
) -> BcResult<Swept> {
    let mut removed_transactions: usize = 0;
    let mut other_batch_references_removed: usize = 0;
    let mut survivors: Vec<String> = Vec::new();

    for transaction_id in transaction_ids {
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
                .bind(transaction_id)
                .fetch_one(&mut *conn)
                .await?;
        if remaining > 0 {
            survivors.push(transaction_id.clone());
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
        sqlx::query("DELETE FROM transaction_metadata WHERE transaction_id = ?")
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

    Ok(Swept {
        removed_transactions,
        other_batch_references_removed,
        survivors,
    })
}

/// Closes the gaps a discard leaves in a surviving transaction's posting
/// positions.
///
/// Postings are numbered contiguously from zero everywhere else: the projection
/// renumbers on write, and `add_postings_in_tx` appends at `MAX(position) + 1`.
/// Discard is the first path that removes a posting from a transaction that
/// stays, so without this a later merge — which places its posting at the
/// survivor's posting *count* — would collide with an existing position and
/// leave the transaction's legs in an arbitrary order.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `transaction_ids` - The transactions that survived the sweep.
///
/// # Errors
///
/// Returns [`BcError::Database`] on update failure.
async fn renumber_positions(
    conn: &mut sqlx::SqliteConnection,
    transaction_ids: &[String],
) -> BcResult<()> {
    for transaction_id in transaction_ids {
        sqlx::query(
            "UPDATE postings SET position = ( \
               SELECT COUNT(*) FROM postings AS earlier \
               WHERE earlier.transaction_id = postings.transaction_id \
                 AND earlier.position < postings.position \
             ) WHERE transaction_id = ?",
        )
        .bind(transaction_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// What reversing the batch's tags did.
struct Reversed {
    /// Tags deleted because nothing named them.
    removed: usize,
    /// Tags kept because something did.
    kept: usize,
}

/// Deletes the tags the run created that nothing now names, and forgets the
/// rest.
///
/// Walks the recorded tags deepest first, so a leaf is judged before its
/// parent and a kept leaf keeps its ancestors. "Named" means a posting,
/// transaction or account membership, a budget revision's filter, or a child
/// tag — one this run created and kept, or one anyone else created.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `id_str` - The batch's ID as stored.
///
/// # Returns
///
/// How many tags went and how many stayed.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query or delete failure.
async fn reverse_tags(conn: &mut sqlx::SqliteConnection, id_str: &str) -> BcResult<Reversed> {
    // Depth is the length of each recorded tag's parent chain; the recursion
    // stops at a root, whose parent join finds nothing.
    let recorded: Vec<(String, i64)> = sqlx::query_as(
        "WITH RECURSIVE chain(tag_id, ancestor_id, depth) AS ( \
             SELECT t.id, t.parent_id, 0 FROM tags t \
             JOIN import_batch_tags r ON r.tag_id = t.id \
             WHERE r.import_batch_id = ? \
           UNION ALL \
             SELECT chain.tag_id, t.parent_id, chain.depth + 1 \
             FROM chain JOIN tags t ON t.id = chain.ancestor_id \
         ) \
         SELECT tag_id, MAX(depth) AS depth FROM chain \
         GROUP BY tag_id ORDER BY depth DESC, tag_id",
    )
    .bind(id_str)
    .fetch_all(&mut *conn)
    .await?;

    let mut out = Reversed {
        removed: 0,
        kept: 0,
    };
    for (tag_id, _depth) in recorded {
        if is_named(&mut *conn, &tag_id).await? {
            out.kept = out.kept.saturating_add(1);
            continue;
        }
        sqlx::query("DELETE FROM tags WHERE id = ?")
            .bind(&tag_id)
            .execute(&mut *conn)
            .await?;
        out.removed = out.removed.saturating_add(1);
    }
    sqlx::query("DELETE FROM import_batch_tags WHERE import_batch_id = ?")
        .bind(id_str)
        .execute(conn)
        .await?;
    Ok(out)
}

/// Reports whether anything still names a tag.
///
/// # Arguments
///
/// * `conn` - An open SQLite connection or transaction.
/// * `tag_id` - The tag's ID as stored.
///
/// # Returns
///
/// `true` if a membership, a budget filter or a child tag names it.
///
/// # Errors
///
/// Returns [`BcError::Database`] on query failure.
async fn is_named(conn: &mut sqlx::SqliteConnection, tag_id: &str) -> BcResult<bool> {
    let named: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM posting_tags WHERE tag_id = ?1) \
              + EXISTS(SELECT 1 FROM transaction_tags WHERE tag_id = ?1) \
              + EXISTS(SELECT 1 FROM account_tags WHERE tag_id = ?1) \
              + EXISTS(SELECT 1 FROM budget_revisions WHERE tag_filter = ?1) \
              + EXISTS(SELECT 1 FROM tags WHERE parent_id = ?1)",
    )
    .bind(tag_id)
    .fetch_one(conn)
    .await?;
    Ok(named > 0)
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
        other_batch_references_tombstoned: to_u64(outcome.other_batch_references_tombstoned)?,
        edited_postings: to_u64(outcome.edited_postings)?,
        reconciled_postings: to_u64(outcome.reconciled_postings)?,
        flagged_postings: to_u64(outcome.flagged_postings)?,
        removed_tags: to_u64(outcome.removed_tags)?,
        kept_tags: to_u64(outcome.kept_tags)?,
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
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
    use bc_models::TagId;
    use bc_models::TagPath;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use super::Outcome;
    use crate::BcError;
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
            .add_postings_in_tx(&mut db_tx, transaction_id, date(2026, 1, 15), &[posting])
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
        // The run whose own leg holds the transaction up is opened first: a
        // discard is refused when a *later* run owns a leg on its transaction.
        let second = batches.open(None, "csv").await.expect("open second");
        let first = batches.open(None, "csv").await.expect("open first");
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
        assert_eq!(
            outcome.other_batch_references_tombstoned, 1,
            "another batch's reference lost its posting; the report has to say so, \
             since the transaction surviving means nothing else reveals it"
        );
        assert_eq!(
            outcome.other_batch_references_removed, 0,
            "nothing was swept, so nothing was removed with a transaction"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_swept_transactions_other_batch_reference_is_not_double_counted(pool: SqlitePool) {
        // The same collateral reference, but on a transaction that empties.
        // It goes with the transaction, so it belongs to `..._removed` and
        // must not also appear in `..._tombstoned`.
        let batches = ImportBatchService::new(pool.clone());
        let first = batches.open(None, "csv").await.expect("open first");
        let second = batches.open(None, "csv").await.expect("open second");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &first, &tx, &posting, &acct, true).await;
        attach_at(&pool, &second, &tx, &posting, &acct, false, 1).await;

        let outcome = batches.discard(&first).await.expect("discard");

        assert_eq!(outcome.removed_transactions, 1, "nothing held it up");
        assert_eq!(outcome.other_batch_references_removed, 1);
        assert_eq!(
            outcome.other_batch_references_tombstoned, 0,
            "a reference counted as removed must not be counted as tombstoned too"
        );
    }

    /// Hands out occurrence slots, so every reference a test attaches to one
    /// account can share a fingerprint without colliding.
    struct Slots(u32);

    impl Slots {
        /// Returns the next unused slot.
        fn next(&mut self) -> u32 {
            let slot = self.0;
            self.0 = self.0.checked_add(1).expect("small test");
            slot
        }
    }

    /// Builds a transaction holding `count` postings on `acct`, each attached
    /// to `batch` as a posting that batch created.
    async fn owned_transaction(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        acct: &AccountId,
        count: usize,
        slots: &mut Slots,
    ) -> (TransactionId, Vec<PostingId>) {
        let (tx, first) = transaction_with_posting(pool, acct).await;
        let mut postings = vec![first];
        for _ in 1..count {
            postings.push(add_posting(pool, &tx, acct).await);
        }
        for posting in &postings {
            attach_at(pool, batch, &tx, posting, acct, true, slots.next()).await;
        }
        (tx, postings)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_discard_event_carries_every_count(pool: SqlitePool) {
        // Asserts the payload, not just that an event exists. `event_for` maps
        // nine same-typed counts by hand, and a transposed pair would report
        // the wrong numbers forever in the only durable record of an
        // irreversible operation. Every count is made pairwise distinct, so no
        // swap can hide behind two fields that happen to agree.
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let other = batches.open(None, "ofx").await.expect("open other");
        let acct = account(&pool, "Checking").await;
        let elsewhere = account(&pool, "Savings").await;
        let mut slots = Slots(0);

        // Two owned postings, reconciled: reconciled_postings = 2.
        let (reconciled_tx, _) = owned_transaction(&pool, &batch, &acct, 2, &mut slots).await;
        sqlx::query("UPDATE transactions SET reconciliation = 'reconciled' WHERE id = ?")
            .bind(reconciled_tx.to_string())
            .execute(&pool)
            .await
            .expect("reconcile");

        // Four owned postings, flagged: flagged_postings = 4.
        let (flagged_tx, _) = owned_transaction(&pool, &batch, &acct, 4, &mut slots).await;
        sqlx::query("UPDATE transactions SET reconciliation = 'flagged' WHERE id = ?")
            .bind(flagged_tx.to_string())
            .execute(&pool)
            .await
            .expect("flag");

        // One owned posting carrying five of the other batch's adoptions. The
        // transaction empties, so all five go with it:
        // other_batch_references_removed = 5.
        let (swept_tx, swept) = owned_transaction(&pool, &batch, &acct, 1, &mut slots).await;
        let swept_posting = swept.first().expect("one posting");
        for _ in 0..5_u32 {
            attach_at(
                &pool,
                &other,
                &swept_tx,
                swept_posting,
                &acct,
                false,
                slots.next(),
            )
            .await;
        }

        // Seven owned postings, each adopted by the other batch, on a
        // transaction an eighth unreferenced posting keeps alive:
        // other_batch_references_tombstoned = 7.
        let (surviving_tx, surviving) =
            owned_transaction(&pool, &batch, &acct, 7, &mut slots).await;
        for posting in &surviving {
            attach_at(
                &pool,
                &other,
                &surviving_tx,
                posting,
                &acct,
                false,
                slots.next(),
            )
            .await;
        }
        add_posting(&pool, &surviving_tx, &acct).await;

        // One reference on a posting the batch did not create: detached_adopted = 1.
        let (adopted_tx, adopted_posting) = transaction_with_posting(&pool, &acct).await;
        attach_at(
            &pool,
            &batch,
            &adopted_tx,
            &adopted_posting,
            &acct,
            false,
            slots.next(),
        )
        .await;

        // Six of the batch's own references, already orphaned by the user
        // deleting those legs: freed_tombstones = 6. The postings stay, so the
        // transaction survives and contributes nothing else.
        let (tombstoned_tx, tombstoned) =
            owned_transaction(&pool, &batch, &acct, 6, &mut slots).await;
        sqlx::query("UPDATE transaction_sources SET posting_id = NULL WHERE transaction_id = ?")
            .bind(tombstoned_tx.to_string())
            .execute(&pool)
            .await
            .expect("tombstone");
        assert_eq!(tombstoned.len(), 6, "six legs were orphaned");

        // The eight postings the user recategorised — the swept one and the
        // seven surviving-collateral ones: edited_postings = 8.
        for posting in core::iter::once(swept_posting).chain(surviving.iter()) {
            sqlx::query("UPDATE postings SET account_id = ? WHERE id = ?")
                .bind(elsewhere.to_string())
                .bind(posting.to_string())
                .execute(&pool)
                .await
                .expect("recategorise");
        }

        // Two tags the run minted, one of them since applied by hand:
        // removed_tags = 1, kept_tags = 1.
        let minted = tags_for(&pool, &batch, &["gifts", "travel"]).await;
        tag_account(&pool, &acct, minted.ids.get("travel").expect("travel")).await;

        let outcome = batches.discard(&batch).await.expect("discard");

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM events WHERE kind = 'ImportBatchDiscarded'")
                .fetch_one(&pool)
                .await
                .expect("query event payload");
        let event: crate::Event = serde_json::from_str(&payload).expect("decode payload");

        let crate::Event::ImportBatchDiscarded {
            batch_id,
            removed_postings,
            removed_transactions,
            detached_adopted,
            freed_tombstones,
            other_batch_references_removed,
            other_batch_references_tombstoned,
            edited_postings,
            reconciled_postings,
            flagged_postings,
            removed_tags,
            kept_tags,
        } = event
        else {
            panic!("the discard appended an event of the wrong kind");
        };

        assert_eq!(batch_id, batch);
        assert_eq!(detached_adopted, 1, "one adopted reference");
        assert_eq!(reconciled_postings, 2, "two postings in the reconciled run");
        assert_eq!(
            removed_transactions, 3,
            "the reconciled, flagged and swept transactions all emptied"
        );
        assert_eq!(flagged_postings, 4, "four postings in the flagged run");
        assert_eq!(
            other_batch_references_removed, 5,
            "five adoptions rode the swept transaction down"
        );
        assert_eq!(freed_tombstones, 6, "six references were already orphaned");
        assert_eq!(
            other_batch_references_tombstoned, 7,
            "seven adoptions outlived their postings on a surviving transaction"
        );
        assert_eq!(edited_postings, 8, "eight postings were recategorised");
        assert_eq!(
            removed_postings, 14,
            "2 reconciled + 4 flagged + 1 swept + 7 surviving-collateral"
        );
        assert_eq!(removed_tags, 1, "the unused tag went with the run");
        assert_eq!(kept_tags, 1, "the tag applied by hand stayed");

        // The same nine values via the outcome, so a mapping that is
        // self-consistently wrong in both surfaces still cannot pass.
        assert_eq!(
            usize::try_from(removed_postings).expect("fits"),
            outcome.removed_postings
        );
        assert_eq!(
            usize::try_from(removed_transactions).expect("fits"),
            outcome.removed_transactions
        );
        assert_eq!(
            usize::try_from(detached_adopted).expect("fits"),
            outcome.detached_adopted
        );
        assert_eq!(
            usize::try_from(freed_tombstones).expect("fits"),
            outcome.freed_tombstones
        );
        assert_eq!(
            usize::try_from(other_batch_references_removed).expect("fits"),
            outcome.other_batch_references_removed
        );
        assert_eq!(
            usize::try_from(other_batch_references_tombstoned).expect("fits"),
            outcome.other_batch_references_tombstoned
        );
        assert_eq!(
            usize::try_from(edited_postings).expect("fits"),
            outcome.edited_postings
        );
        assert_eq!(
            usize::try_from(reconciled_postings).expect("fits"),
            outcome.reconciled_postings
        );
        assert_eq!(
            usize::try_from(flagged_postings).expect("fits"),
            outcome.flagged_postings
        );
        assert_eq!(
            usize::try_from(removed_tags).expect("fits"),
            outcome.removed_tags
        );
        assert_eq!(usize::try_from(kept_tags).expect("fits"), outcome.kept_tags);
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
    async fn a_posting_whose_amount_changed_is_counted_as_edited(pool: SqlitePool) {
        // The edit predicate is a three-way disjunction and only its
        // account_id arm was exercised. Correcting an amount is the more
        // common edit to an imported row than recategorising it.
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;
        sqlx::query("UPDATE postings SET amount = '55' WHERE id = ?")
            .bind(posting.to_string())
            .execute(&pool)
            .await
            .expect("correct the amount");

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(
            outcome.edited_postings, 1,
            "a corrected amount is an edit the user is about to lose"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_materialised_residual_is_not_mistaken_for_an_edit(pool: SqlitePool) {
        // An elided leg records no amount on its reference and is given the
        // document's residual on its posting, so the two legitimately differ
        // from the moment of import. A null-safe comparison alone reports
        // every such leg as edited by a user who never touched it.
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        let reference = attach(&pool, &batch, &tx, &posting, &acct, true).await;
        // As `import_exec` writes an elided leg: the posting carries the
        // residual, the reference fingerprints the empty amount.
        sqlx::query("UPDATE transaction_sources SET amount = NULL, commodity = NULL WHERE id = ?")
            .bind(reference.to_string())
            .execute(&pool)
            .await
            .expect("elide the reference amount");

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(
            outcome.edited_postings, 0,
            "the import itself wrote this difference; the user changed nothing"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tagged_posting_and_its_tagged_transaction_are_removed(pool: SqlitePool) {
        // posting_tags, transaction_tags and the two metadata tables do not cascade
        // from the rows discard deletes, so each needs its own DELETE. Without
        // them a discard of anything the user had tagged aborts on a foreign
        // key violation instead of running.
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &batch, &tx, &posting, &acct, true).await;

        let tag = crate::TagService::new(pool.clone())
            .create_path(&bc_models::TagPath::new(["groceries"]).expect("valid tag path"))
            .await
            .expect("create tag");
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
            .bind(posting.to_string())
            .bind(tag.to_string())
            .execute(&pool)
            .await
            .expect("tag the posting");
        sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
            .bind(tx.to_string())
            .bind(tag.to_string())
            .execute(&pool)
            .await
            .expect("tag the transaction");
        sqlx::query("INSERT INTO metadata_keys (key, value_type, created_at) VALUES (?, ?, ?)")
            .bind("settled")
            .bind("date")
            .bind("2026-01-15T00:00:00Z")
            .execute(&pool)
            .await
            .expect("register the key");
        sqlx::query(
            "INSERT INTO transaction_metadata (transaction_id, key, position, value_text) \
             VALUES (?, ?, 0, ?)",
        )
        .bind(tx.to_string())
        .bind("settled")
        .bind("2026-01-16")
        .execute(&pool)
        .await
        .expect("annotate the transaction");

        let outcome = batches
            .discard(&batch)
            .await
            .expect("a tagged batch discards");

        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.removed_transactions, 1);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM posting_tags").await, 0);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_tags").await,
            0
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM transaction_metadata").await,
            0
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM tags").await,
            1,
            "the tag itself is not the batch's to delete"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_surviving_transactions_positions_are_left_contiguous(pool: SqlitePool) {
        // Every other writer keeps positions contiguous from zero, and
        // `TransferService::merge` places its posting at the survivor's
        // posting *count* — so a gap left here would collide with an existing
        // position and scramble the leg order on the next merge.
        let batches = ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;
        let (tx, first) = transaction_with_posting(&pool, &acct).await;
        let second = add_posting(&pool, &tx, &acct).await;
        let third = add_posting(&pool, &tx, &acct).await;
        // Discard the middle leg only, which is the one that leaves a gap.
        attach(&pool, &batch, &tx, &second, &acct, true).await;

        let outcome = batches.discard(&batch).await.expect("discard");

        assert_eq!(outcome.removed_transactions, 0, "two legs still stand");
        let positions: Vec<i64> = sqlx::query_scalar(
            "SELECT position FROM postings WHERE transaction_id = ? ORDER BY position",
        )
        .bind(tx.to_string())
        .fetch_all(&pool)
        .await
        .expect("read positions");
        assert_eq!(
            positions,
            vec![0, 1],
            "the gap the removed leg left must be closed, not preserved"
        );
        let surviving: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM postings WHERE transaction_id = ? ORDER BY position",
        )
        .bind(tx.to_string())
        .fetch_all(&pool)
        .await
        .expect("read survivors");
        assert_eq!(
            surviving,
            vec![first.to_string(), third.to_string()],
            "renumbering must preserve the surviving legs' relative order"
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
    /// Attaches a reference with no batch at all, as the public
    /// `SourceService::attach` does for a hand-recorded provenance.
    async fn attach_unbatched(
        pool: &SqlitePool,
        transaction_id: &TransactionId,
        posting_id: &PostingId,
        account_id: &AccountId,
    ) {
        let source = SourceRef::builder()
            .id(SourceRefId::new())
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
            .occurrence(0)
            .import_batch_id(None)
            .owns_posting(true)
            .created_at(Timestamp::now())
            .build();
        crate::SourceService::new(pool.clone())
            .attach(&source)
            .await
            .expect("attach");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_batch_a_later_batch_built_on_cannot_be_discarded(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let older = batches.open(None, "csv").await.expect("open older");
        let newer = batches.open(None, "ledger").await.expect("open newer");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &older, &tx, &posting, &acct, true).await;
        let counter = add_posting(&pool, &tx, &other).await;
        attach(&pool, &newer, &tx, &counter, &other, true).await;

        let err = batches.discard(&older).await.expect_err("blocked");

        let BcError::DiscardBlocked { batch, dependants } = err else {
            panic!("expected DiscardBlocked, got {err:?}");
        };
        assert_eq!(batch, older);
        let [dependant] = dependants.as_slice() else {
            panic!("expected one dependant, got {dependants:?}");
        };
        assert_eq!(dependant.batch_id, newer);
        assert_eq!(dependant.importer, "ledger");
        assert_eq!(dependant.postings, 1);
        assert_eq!(dependant.transactions, 1);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM postings").await,
            2,
            "a refused discard touches nothing"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM import_batches WHERE discarded_at IS NOT NULL"
            )
            .await,
            0
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn dependants_are_listed_newest_first_with_their_counts(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let oldest = batches.open(None, "csv").await.expect("open oldest");
        let middle = batches.open(None, "csv").await.expect("open middle");
        let newest = batches.open(None, "ledger").await.expect("open newest");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let mut slots = Slots(0);
        // Two transactions the oldest run owns, each with two of its legs, so
        // the self-join has a duplicate to collapse.
        let (tx_a, _) = owned_transaction(&pool, &oldest, &acct, 2, &mut slots).await;
        let (tx_b, _) = owned_transaction(&pool, &oldest, &acct, 2, &mut slots).await;
        // The middle run adds one leg to each; the newest adds one to the first.
        let middle_a = add_posting(&pool, &tx_a, &other).await;
        attach_at(&pool, &middle, &tx_a, &middle_a, &other, true, 0).await;
        let middle_b = add_posting(&pool, &tx_b, &other).await;
        attach_at(&pool, &middle, &tx_b, &middle_b, &other, true, 1).await;
        let newest_a = add_posting(&pool, &tx_a, &other).await;
        attach_at(&pool, &newest, &tx_a, &newest_a, &other, true, 2).await;

        let err = batches.discard(&oldest).await.expect_err("blocked");

        let BcError::DiscardBlocked { dependants, .. } = err else {
            panic!("expected DiscardBlocked, got {err:?}");
        };
        let ids: Vec<&ImportBatchId> = dependants.iter().map(|d| &d.batch_id).collect();
        assert_eq!(
            ids,
            vec![&newest, &middle],
            "newest first: the order to discard in"
        );
        let counts: Vec<(usize, usize)> = dependants
            .iter()
            .map(|d| (d.postings, d.transactions))
            .collect();
        assert_eq!(
            counts,
            vec![(1, 1), (2, 2)],
            "each of the oldest run's two legs per transaction must not double the count"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_service_short_circuit_reports_the_same_block(pool: SqlitePool) {
        let batches = ImportBatchService::new(pool.clone());
        let older = batches.open(None, "csv").await.expect("open older");
        let newer = batches.open(None, "csv").await.expect("open newer");
        let acct = account(&pool, "Checking").await;
        let other = account(&pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(&pool, &acct).await;
        attach(&pool, &older, &tx, &posting, &acct, true).await;
        let counter = add_posting(&pool, &tx, &other).await;
        attach(&pool, &newer, &tx, &counter, &other, true).await;

        let err = batches
            .ensure_discardable(&older)
            .await
            .expect_err("blocked");

        assert!(
            matches!(err, BcError::DiscardBlocked { .. }),
            "the pre-snapshot check must refuse exactly what discard refuses, got {err:?}"
        );
    }

    /// A shape in which another reference sits on this batch's transaction
    /// without depending on it, so the discard must go ahead.
    #[derive(Clone, Copy, Debug)]
    enum Harmless {
        /// The other batch is older: this batch's legs were the counter-legs.
        OtherIsOlder,
        /// The other batch adopted this batch's leg rather than owning one.
        OtherAdopted,
        /// The other batch's leg was deleted by the user, leaving a tombstone.
        OtherTombstoned,
        /// The other reference was attached by hand, with no batch.
        OtherHasNoBatch,
        /// The other batch was already discarded.
        OtherDiscarded,
        /// This batch only adopted its leg, so the discard removes no posting.
        ThisOnlyAdopted,
    }

    /// Builds `shape` around one transaction and discards the batch under test.
    async fn discard_harmless_shape(pool: &SqlitePool, shape: Harmless) -> Outcome {
        let batches = ImportBatchService::new(pool.clone());
        let (this, other_batch) = if matches!(shape, Harmless::OtherIsOlder) {
            let other = batches.open(None, "csv").await.expect("open other");
            let this = batches.open(None, "csv").await.expect("open this");
            (this, other)
        } else {
            let this = batches.open(None, "csv").await.expect("open this");
            let other = batches.open(None, "csv").await.expect("open other");
            (this, other)
        };
        let acct = account(pool, "Checking").await;
        let other_acct = account(pool, "Groceries").await;
        let (tx, posting) = transaction_with_posting(pool, &acct).await;
        let this_owns = !matches!(shape, Harmless::ThisOnlyAdopted);
        attach(pool, &this, &tx, &posting, &acct, this_owns).await;

        match shape {
            Harmless::OtherIsOlder | Harmless::OtherDiscarded | Harmless::ThisOnlyAdopted => {
                let counter = add_posting(pool, &tx, &other_acct).await;
                attach(pool, &other_batch, &tx, &counter, &other_acct, true).await;
            }
            Harmless::OtherAdopted => {
                attach_at(pool, &other_batch, &tx, &posting, &acct, false, 1).await;
            }
            Harmless::OtherTombstoned => {
                let counter = add_posting(pool, &tx, &other_acct).await;
                let reference = attach(pool, &other_batch, &tx, &counter, &other_acct, true).await;
                sqlx::query("UPDATE transaction_sources SET posting_id = NULL WHERE id = ?")
                    .bind(reference.to_string())
                    .execute(pool)
                    .await
                    .expect("tombstone");
                sqlx::query("DELETE FROM postings WHERE id = ?")
                    .bind(counter.to_string())
                    .execute(pool)
                    .await
                    .expect("delete posting");
            }
            Harmless::OtherHasNoBatch => {
                let counter = add_posting(pool, &tx, &other_acct).await;
                attach_unbatched(pool, &tx, &counter, &other_acct).await;
            }
        }
        if matches!(shape, Harmless::OtherDiscarded) {
            batches.discard(&other_batch).await.expect("discard other");
        }

        batches.discard(&this).await.expect("not blocked")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_older_batch_on_the_transaction_does_not_block(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::OtherIsOlder).await;
        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.removed_transactions, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_later_batch_that_only_adopted_does_not_block(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::OtherAdopted).await;
        assert_eq!(outcome.other_batch_references_removed, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_later_batchs_tombstone_does_not_block(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::OtherTombstoned).await;
        assert_eq!(outcome.removed_transactions, 1);
        assert_eq!(outcome.other_batch_references_removed, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_hand_attached_reference_does_not_block(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::OtherHasNoBatch).await;
        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.removed_transactions, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_already_discarded_later_batch_does_not_block(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::OtherDiscarded).await;
        assert_eq!(outcome.removed_postings, 1);
        assert_eq!(outcome.removed_transactions, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_batch_that_only_adopted_is_not_blocked(pool: SqlitePool) {
        let outcome = discard_harmless_shape(&pool, Harmless::ThisOnlyAdopted).await;
        assert_eq!(outcome.detached_adopted, 1);
        assert_eq!(outcome.removed_postings, 0);
    }

    /// Creates `paths` through the tag service and records every minted row
    /// against `batch`, the way `Commit::ensure_tags` does.
    async fn tags_for(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        paths: &[&str],
    ) -> crate::CreatedTags {
        let parsed: Vec<TagPath> = paths.iter().map(|p| p.parse().expect("path")).collect();
        let created = crate::TagService::new(pool.clone())
            .create_paths(&parsed)
            .await
            .expect("create tags");
        ImportBatchService::new(pool.clone())
            .record_tags(batch, &created.minted)
            .await
            .expect("record tags");
        created
    }

    /// Applies `tag` to `account` by hand.
    async fn tag_account(pool: &SqlitePool, account: &AccountId, tag: &TagId) {
        sqlx::query("INSERT INTO account_tags (account_id, tag_id) VALUES (?, ?)")
            .bind(account.to_string())
            .bind(tag.to_string())
            .execute(pool)
            .await
            .expect("tag account");
    }

    /// Opens a batch owning one leg on one transaction, so a discard has
    /// something to remove besides its tags.
    async fn batch_with_a_leg(pool: &SqlitePool) -> (ImportBatchId, AccountId, TransactionId) {
        let batch = ImportBatchService::new(pool.clone())
            .open(None, "csv")
            .await
            .expect("open");
        let acct = account(pool, "Checking").await;
        let (tx, posting) = transaction_with_posting(pool, &acct).await;
        attach(pool, &batch, &tx, &posting, &acct, true).await;
        (batch, acct, tx)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unused_tag_and_its_minted_ancestor_go_with_the_batch(pool: SqlitePool) {
        let (batch, _, _) = batch_with_a_leg(&pool).await;
        tags_for(&pool, &batch, &["food:groceries"]).await;

        let outcome = ImportBatchService::new(pool.clone())
            .discard(&batch)
            .await
            .expect("discard");

        assert_eq!(outcome.removed_tags, 2, "food and food:groceries");
        assert_eq!(outcome.kept_tags, 0);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM tags").await, 0);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM import_batch_tags").await,
            0,
            "the batch's record is dropped with the discard"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_ancestor_with_a_surviving_child_is_kept(pool: SqlitePool) {
        let (batch, _, _) = batch_with_a_leg(&pool).await;
        tags_for(&pool, &batch, &["food:groceries"]).await;
        // A sibling created by hand afterwards, under the run's ancestor.
        crate::TagService::new(pool.clone())
            .create_path(&"food:dining".parse().expect("path"))
            .await
            .expect("create by hand");

        let outcome = ImportBatchService::new(pool.clone())
            .discard(&batch)
            .await
            .expect("discard");

        assert_eq!(outcome.removed_tags, 1, "food:groceries");
        assert_eq!(outcome.kept_tags, 1, "food still has a child");
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM tags").await, 2);
    }

    /// Where a tag the run created has since been applied by hand.
    #[derive(Clone, Copy, Debug)]
    enum Applied {
        Posting,
        Transaction,
        Account,
        BudgetFilter,
    }

    /// Mints `food:groceries` on a batch, applies the leaf as `applied`
    /// outside the batch, and discards.
    async fn discard_with_leaf_applied(pool: &SqlitePool, applied: Applied) -> Outcome {
        let (batch, acct, _) = batch_with_a_leg(pool).await;
        let minted = tags_for(pool, &batch, &["food:groceries"]).await;
        let leaf = minted.ids.get("food:groceries").expect("leaf").to_string();
        match applied {
            Applied::Posting => {
                // A posting outside the batch, so the discard does not take
                // the membership with it.
                let elsewhere = account(pool, "Savings").await;
                let (_, posting) = transaction_with_posting(pool, &elsewhere).await;
                sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
                    .bind(posting.to_string())
                    .bind(&leaf)
                    .execute(pool)
                    .await
                    .expect("tag posting");
            }
            Applied::Transaction => {
                let elsewhere = account(pool, "Savings").await;
                let (tx, _) = transaction_with_posting(pool, &elsewhere).await;
                sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
                    .bind(tx.to_string())
                    .bind(&leaf)
                    .execute(pool)
                    .await
                    .expect("tag transaction");
            }
            Applied::Account => {
                tag_account(pool, &acct, minted.ids.get("food:groceries").expect("leaf")).await;
            }
            Applied::BudgetFilter => {
                sqlx::query("INSERT INTO budgets (id, account_id, created_at) VALUES (?, ?, ?)")
                    .bind("budget_1")
                    .bind(acct.to_string())
                    .bind(Timestamp::now().to_string())
                    .execute(pool)
                    .await
                    .expect("budget");
                sqlx::query(
                    "INSERT INTO budget_revisions \
                     (id, budget_id, effective_from, period, rollover, tag_filter, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind("budget_revision_1")
                .bind("budget_1")
                .bind("2026-01-01")
                .bind("\"monthly\"")
                .bind("reset_to_zero")
                .bind(&leaf)
                .bind(Timestamp::now().to_string())
                .execute(pool)
                .await
                .expect("revision");
            }
        }

        ImportBatchService::new(pool.clone())
            .discard(&batch)
            .await
            .expect("discard")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_applied_to_a_posting_by_hand_is_kept(pool: SqlitePool) {
        let outcome = discard_with_leaf_applied(&pool, Applied::Posting).await;
        assert_eq!(outcome.kept_tags, 2, "the leaf, and the ancestor it keeps");
        assert_eq!(outcome.removed_tags, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_applied_to_a_transaction_by_hand_is_kept(pool: SqlitePool) {
        let outcome = discard_with_leaf_applied(&pool, Applied::Transaction).await;
        assert_eq!(outcome.kept_tags, 2);
        assert_eq!(outcome.removed_tags, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_applied_to_an_account_by_hand_is_kept(pool: SqlitePool) {
        let outcome = discard_with_leaf_applied(&pool, Applied::Account).await;
        assert_eq!(outcome.kept_tags, 2);
        assert_eq!(outcome.removed_tags, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_used_as_a_budget_filter_is_kept(pool: SqlitePool) {
        let outcome = discard_with_leaf_applied(&pool, Applied::BudgetFilter).await;
        assert_eq!(outcome.kept_tags, 2);
        assert_eq!(outcome.removed_tags, 0);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM budget_revisions").await,
            1
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_a_later_batch_reused_is_kept(pool: SqlitePool) {
        let (older, _, _) = batch_with_a_leg(&pool).await;
        let minted = tags_for(&pool, &older, &["food:groceries"]).await;
        // The later run names the same path, mints nothing, and tags its own
        // leg — on a transaction of its own, so it is no dependant of the
        // older run.
        let batches = ImportBatchService::new(pool.clone());
        let newer = batches.open(None, "csv").await.expect("open newer");
        let reused = tags_for(&pool, &newer, &["food:groceries"]).await;
        assert!(reused.minted.is_empty(), "nothing new to mint");
        let elsewhere = account(&pool, "Savings").await;
        let (tx, posting) = transaction_with_posting(&pool, &elsewhere).await;
        attach_at(&pool, &newer, &tx, &posting, &elsewhere, true, 1).await;
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
            .bind(posting.to_string())
            .bind(minted.ids.get("food:groceries").expect("leaf").to_string())
            .execute(&pool)
            .await
            .expect("tag posting");

        let outcome = batches.discard(&older).await.expect("discard");

        assert_eq!(outcome.kept_tags, 2);
        assert_eq!(outcome.removed_tags, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_deleted_by_hand_before_the_discard_is_a_no_op(pool: SqlitePool) {
        let (batch, _, _) = batch_with_a_leg(&pool).await;
        let minted = tags_for(&pool, &batch, &["food:groceries"]).await;
        let root = minted.minted.first().expect("root").clone();
        crate::TagService::new(pool.clone())
            .delete(&root)
            .await
            .expect("delete by hand");

        let outcome = ImportBatchService::new(pool.clone())
            .discard(&batch)
            .await
            .expect("discard");

        assert_eq!(outcome.removed_tags, 0);
        assert_eq!(outcome.kept_tags, 0);
    }

    async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(pool)
            .await
            .expect("count")
    }
}
