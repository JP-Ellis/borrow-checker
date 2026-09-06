//! Transaction service with double-entry validation.

use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Cost;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Named row type for postings loaded in bulk during [`Service::list`].
///
/// Includes `transaction_id` so postings can be grouped by transaction.
#[derive(sqlx::FromRow)]
struct ListPostingRow {
    /// Posting ID.
    id: String,
    /// Parent transaction ID (used for grouping).
    transaction_id: String,
    /// Account ID for this posting.
    account_id: String,
    /// Decimal string for the posting amount; `None` when this leg is elided.
    amount: Option<String>,
    /// Commodity code string; `None` iff `amount` is `None`.
    commodity: Option<String>,
    /// Decimal string for the cost basis total value; NULL if no cost basis.
    cost_total_value: Option<String>,
    /// Commodity code for the cost basis total; NULL when `cost_total_value` is NULL.
    cost_total_commodity: Option<String>,
    /// Optional cost acquisition date in ISO 8601 format.
    cost_date: Option<String>,
    /// Optional cost lot label.
    cost_label: Option<String>,
    /// Start of the accrual spread window in ISO 8601 format; NULL if no spread.
    spread_from: Option<String>,
    /// End of the accrual spread window in ISO 8601 format; NULL if no spread.
    spread_until: Option<String>,
}

/// Row type for a transaction fetched from the `transactions` table.
///
/// Fields: `(id, date, description, reconciliation, created_at)`.
pub(crate) type TxRow = (String, String, String, String, String);

/// Builds a comma-separated list of `n` SQL bind placeholders, e.g. `?,?,?`.
///
/// Used to bind a variable-length set of IDs into an `IN (…)` clause, since
/// SQLite has no native array binding.
pub(crate) fn sql_placeholders(n: usize) -> String {
    let mut out = String::with_capacity(n.saturating_mul(2));
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        out.push('?');
    }
    out
}

/// Returns a posting's spread window as a `(from, until)` pair when both ends are set.
///
/// Returns `None` if either end of the spread window is absent.
pub(crate) fn spread_pair(posting: &Posting) -> Option<(Date, Date)> {
    match (posting.spread_from(), posting.spread_until()) {
        (Some(from), Some(until)) => Some((from, until)),
        _ => None,
    }
}

/// Merges `updated` with fields from `current` that the edit DTO cannot express.
///
/// The `edit` path receives a `Transaction` built from an `EditTransaction` DTO
/// whose `EditPosting` has no `cost` field. Without merging, calling
/// `apply_transaction_projection` with the DTO-derived value would silently wipe
/// cost columns.
///
/// This function returns a new `Transaction` that carries all of `updated`'s
/// editable fields (date, description, `metadata`, `tag_ids`, posting
/// account/amount/metadata/tags/spread) while carrying forward from `current`:
/// - `metadata`: taken from `updated` (the DTO is authoritative).
/// - `reconciliation`: always taken from `current`; the edit path never changes it
///   (reconciliation is owned by `Service::reconcile`, which enforces the balance
///   guard). The DTO's `reconciliation` field is echoed but ignored here.
/// - per-posting `cost`: taken from the matching `current` posting (by ID); new
///   postings (ID not in `current`) keep `None`.
///
/// # Arguments
///
/// * `current` - The current stored transaction state.
/// * `updated` - The desired transaction state built from the edit DTO.
///
/// # Returns
///
/// A merged `Transaction` suitable for both diffing and projection rewrite.
fn merge_preserving(current: &Transaction, updated: &Transaction) -> Transaction {
    let current_postings: std::collections::HashMap<&PostingId, &Posting> =
        current.postings().iter().map(|p| (p.id(), p)).collect();

    let merged_postings: Vec<Posting> = updated
        .postings()
        .iter()
        .map(|p| {
            let carried_cost = current_postings
                .get(p.id())
                .and_then(|cp| cp.cost())
                .cloned();
            Posting::builder()
                .id(p.id().clone())
                .account_id(p.account_id().clone())
                .maybe_amount(p.amount().cloned())
                .maybe_cost(carried_cost)
                .metadata(p.metadata().clone())
                .tag_ids(p.tag_ids().to_vec())
                .maybe_spread_from(p.spread_from())
                .maybe_spread_until(p.spread_until())
                .build()
        })
        .collect();

    Transaction::builder()
        .id(updated.id().clone())
        .date(updated.date())
        .description(updated.description().to_owned())
        .postings(merged_postings)
        .reconciliation(current.reconciliation())
        .tag_ids(updated.tag_ids().to_vec())
        .metadata(updated.metadata().clone())
        .created_at(*updated.created_at())
        .build()
}

/// Computes the events that turn `prev` into `posting`, both being the same
/// leg of transaction `id` before and after an edit.
///
/// # Arguments
///
/// * `id` - The transaction both postings belong to.
/// * `prev` - The stored posting state.
/// * `posting` - The desired posting state, carrying the same [`PostingId`].
///
/// # Returns
///
/// The list of events; empty if the two states are equal.
fn diff_posting(id: &TransactionId, prev: &Posting, posting: &Posting) -> Vec<Event> {
    let mut events = Vec::new();

    if prev.account_id() != posting.account_id() {
        events.push(Event::PostingRecategorised {
            id: id.clone(),
            posting_id: posting.id().clone(),
            from_account: prev.account_id().clone(),
            to_account: posting.account_id().clone(),
        });
    }
    if prev.amount() != posting.amount() {
        events.push(Event::PostingAmountChanged {
            id: id.clone(),
            posting_id: posting.id().clone(),
            from: prev.amount().cloned(),
            to: posting.amount().cloned(),
        });
    }
    let prev_spread = spread_pair(prev);
    let new_spread = spread_pair(posting);
    if prev_spread != new_spread {
        events.push(Event::PostingSpreadChanged {
            id: id.clone(),
            posting_id: posting.id().clone(),
            from: prev_spread,
            to: new_spread,
        });
    }
    if !prev.metadata().eq_ignoring_mismatched(posting.metadata()) {
        events.push(Event::PostingMetadataChanged {
            id: id.clone(),
            posting_id: posting.id().clone(),
            before: prev.metadata().clone(),
            after: posting.metadata().clone(),
        });
    }

    events
}

/// Computes the decomposed semantic events that turn `current` into `updated`.
///
/// Postings are matched by [`PostingId`]. Transaction-scalar changes are emitted
/// first, then per-posting changes in `updated` order, then removals.
///
/// # Arguments
///
/// * `current` - The stored transaction state.
/// * `updated` - The desired transaction state (same [`TransactionId`]).
///
/// # Returns
///
/// The list of events; empty if the two states are equal.
pub(crate) fn diff_transaction(current: &Transaction, updated: &Transaction) -> Vec<Event> {
    let id = updated.id().clone();
    let mut events = Vec::new();

    if current.date() != updated.date() {
        events.push(Event::TransactionDateChanged {
            id: id.clone(),
            from: current.date(),
            to: updated.date(),
        });
    }
    if current.description() != updated.description() {
        events.push(Event::TransactionDescriptionChanged {
            id: id.clone(),
            from: current.description().to_owned(),
            to: updated.description().to_owned(),
        });
    }

    let current_tags: std::collections::HashSet<&TagId> = current.tag_ids().iter().collect();
    let updated_tags: std::collections::HashSet<&TagId> = updated.tag_ids().iter().collect();
    let mut added: Vec<TagId> = updated_tags
        .difference(&current_tags)
        .map(|t| (*t).clone())
        .collect();
    let mut removed: Vec<TagId> = current_tags
        .difference(&updated_tags)
        .map(|t| (*t).clone())
        .collect();
    added.sort_by_key(std::string::ToString::to_string);
    removed.sort_by_key(std::string::ToString::to_string);
    if !added.is_empty() || !removed.is_empty() {
        events.push(Event::TransactionTagsChanged {
            id: id.clone(),
            added,
            removed,
        });
    }

    if !current
        .metadata()
        .eq_ignoring_mismatched(updated.metadata())
    {
        events.push(Event::TransactionMetadataChanged {
            id: id.clone(),
            before: current.metadata().clone(),
            after: updated.metadata().clone(),
        });
    }

    let current_postings: std::collections::HashMap<&PostingId, &Posting> =
        current.postings().iter().map(|p| (p.id(), p)).collect();

    for posting in updated.postings() {
        match current_postings.get(posting.id()) {
            None => events.push(Event::PostingAdded {
                id: id.clone(),
                posting_id: posting.id().clone(),
                account: posting.account_id().clone(),
                amount: posting.amount().cloned(),
            }),
            Some(prev) => events.extend(diff_posting(&id, prev, posting)),
        }
    }

    let updated_ids: std::collections::HashSet<&PostingId> =
        updated.postings().iter().map(Posting::id).collect();
    for prev in current.postings() {
        if !updated_ids.contains(prev.id()) {
            events.push(Event::PostingRemoved {
                id: id.clone(),
                posting_id: prev.id().clone(),
            });
        }
    }

    events
}

/// Validates a transaction's postings before persistence.
///
/// Storing is permissive — an unbalanced (e.g. one-sided, freshly-imported)
/// transaction is valid and persists so the UI can surface it for resolution.
/// Only structurally impossible posting sets are rejected:
/// - an empty posting list;
/// - two or more elided (`None`) amounts, whose residual is ambiguous;
/// - a single posting that is itself elided, which carries no amount at all.
fn validate_postings(postings: &[Posting]) -> BcResult<()> {
    if postings.is_empty() {
        return Err(BcError::BadData("transaction has no postings".into()));
    }
    let elided = postings.iter().filter(|p| p.amount().is_none()).count();
    if elided >= 2 {
        return Err(BcError::BadData("two or more elided postings".into()));
    }
    if elided == 1 && postings.len() == 1 {
        return Err(BcError::BadData(
            "a lone elided posting carries no amount".into(),
        ));
    }
    Ok(())
}

/// Returns `true` when some posting inside `budget_subtree` carries a tag in
/// `tag_subtree`, counting the transaction's own tags as flowing down to each
/// of its postings. Used for the unfiltered drill-down, where only the budget's
/// own tag filter applies.
fn budget_leg_carries_tag(
    tx: &Transaction,
    budget_subtree: &std::collections::HashSet<bc_models::AccountId>,
    tag_subtree: &std::collections::HashSet<TagId>,
) -> bool {
    let tx_carries = tx.tag_ids().iter().any(|t| tag_subtree.contains(t));
    tx.postings().iter().any(|p| {
        budget_subtree.contains(p.account_id())
            && (tx_carries || p.tag_ids().iter().any(|t| tag_subtree.contains(t)))
    })
}

/// Whether `tx` should appear in [`Service::list_for_budget`]'s drill-down for
/// the budget rooted at `budget_subtree`, under the global transaction `query`.
///
/// A transaction is listed iff it satisfies the transaction-level dimensions
/// AND contains **at least one posting `p` in the budget account's subtree**
/// that simultaneously satisfies every active per-posting dimension — the exact
/// counted-posting conjunction [`crate::budget::BudgetStatusEngine`] sums over
/// (see `build_posting_amounts_sql`). Evaluating the per-posting dimensions on
/// the *same* budget-subtree posting is what keeps the list in parity with the
/// tree count: a non-budget leg that matches the filter never pulls in a
/// transaction whose budget leg does not.
///
/// Transaction-level dimensions:
/// * `text` — case-insensitive substring on description. Metadata is
///   deliberately out of reach until the query language lands (issue #429).
/// * `reconciliation` — exact equality.
///
/// Per-posting dimensions, all evaluated on the same budget-subtree posting `p`:
/// * `p.account_id` falls in `budget_subtree`.
/// * `tag_filter` (budget revision) — `p` or its transaction carries that tag
///   or a descendant of it (transaction tags flow down; matched over the subtree).
/// * `query.accounts` — `p.account_id` falls in `global_accounts` (resolved subtree).
/// * `query.amount` — commodity-exact match via [`crate::search::AmountQuery::matches`].
/// * `query.tags` — the transaction carries a filter tag OR `p` carries one.
///
/// `date_from`/`date_until` are intentionally not checked here — the caller
/// already constrains the date range in SQL via `period_start`/`period_end`.
fn transaction_matches_query(
    tx: &Transaction,
    query: &crate::search::TransactionQuery,
    budget_subtree: &std::collections::HashSet<bc_models::AccountId>,
    tag_subtree: Option<&std::collections::HashSet<TagId>>,
    global_accounts: Option<&std::collections::HashSet<bc_models::AccountId>>,
) -> bool {
    if let Some(text) = &query.text {
        let needle = text.to_ascii_lowercase();
        if !tx.description().to_ascii_lowercase().contains(&needle) {
            return false;
        }
    }

    if let Some(rec) = query.reconciliation
        && tx.reconciliation() != rec
    {
        return false;
    }

    let tag_set: std::collections::HashSet<&TagId> = query.tags.iter().collect();
    let tx_carries_global_tag = tx.tag_ids().iter().any(|t| tag_set.contains(t));
    // Transaction tags flow down to every posting, so a transaction tag in the
    // budget's own subtree satisfies the budget-tag dimension for all its legs.
    let tx_carries_own_tag =
        tag_subtree.is_some_and(|s| tx.tag_ids().iter().any(|t| s.contains(t)));

    // The transaction is kept iff some budget-subtree posting satisfies the
    // full per-posting conjunction the tree counts on that same posting.
    tx.postings().iter().any(|p| {
        if !budget_subtree.contains(p.account_id()) {
            return false;
        }
        if let Some(subtree) = tag_subtree
            && !tx_carries_own_tag
            && !p.tag_ids().iter().any(|t| subtree.contains(t))
        {
            return false;
        }
        if let Some(set) = global_accounts
            && !set.contains(p.account_id())
        {
            return false;
        }
        if let Some(aq) = &query.amount
            && !aq.matches(p.amount())
        {
            return false;
        }
        if !query.tags.is_empty()
            && !tx_carries_global_tag
            && !p.tag_ids().iter().any(|t| tag_set.contains(t))
        {
            return false;
        }
        true
    })
}

/// Parses a `Cost` from the four nullable cost columns on a posting row.
///
/// Returns `None` if `total_value` is `None` (no cost basis recorded).
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any stored value cannot be parsed.
#[expect(
    clippy::needless_pass_by_value,
    reason = "all parameters come from owned DB rows; passing by value is ergonomic at call sites"
)]
fn parse_cost(
    total_value: Option<String>,
    total_commodity: Option<String>,
    cost_date: Option<String>,
    cost_label: Option<String>,
) -> BcResult<Option<Cost>> {
    let Some(value_str) = total_value else {
        return Ok(None);
    };
    let commodity_str = total_commodity.ok_or_else(|| {
        BcError::BadData("cost_total_commodity is NULL with non-NULL cost_total_value".into())
    })?;
    let value = value_str
        .parse::<Decimal>()
        .map_err(|e| BcError::BadData(format!("invalid cost_total_value '{value_str}': {e}")))?;
    let total = Amount::new(value, CommodityCode::new(commodity_str));
    let date = cost_date
        .as_deref()
        .map(|s| {
            s.parse::<Date>()
                .map_err(|e| BcError::BadData(format!("invalid cost_date '{s}': {e}")))
        })
        .transpose()?;
    Ok(Some(
        Cost::builder()
            .total(total)
            .maybe_date(date)
            .maybe_label(cost_label)
            .build(),
    ))
}

/// Named row type for postings loaded during [`Service::find_by_id`].
///
/// Does not include `transaction_id` since we already know it from context.
#[derive(sqlx::FromRow)]
struct PostingRow {
    /// Posting ID.
    id: String,
    /// Account ID for this posting.
    account_id: String,
    /// Decimal string for the posting amount; `None` when this leg is elided.
    amount: Option<String>,
    /// Commodity code string; `None` iff `amount` is `None`.
    commodity: Option<String>,
    /// Decimal string for the cost basis total value; NULL if no cost basis.
    cost_total_value: Option<String>,
    /// Commodity code for the cost basis total; NULL when `cost_total_value` is NULL.
    cost_total_commodity: Option<String>,
    /// Optional cost acquisition date in ISO 8601 format.
    cost_date: Option<String>,
    /// Optional cost lot label.
    cost_label: Option<String>,
    /// Start of the accrual spread window in ISO 8601 format; NULL if no spread.
    spread_from: Option<String>,
    /// End of the accrual spread window in ISO 8601 format; NULL if no spread.
    spread_until: Option<String>,
}

/// One `transaction_sources` row's identity and the posting it named, captured
/// before [`Service::apply_transaction_projection`] replaces the posting set.
///
/// The second element is already `None` for a row tombstoned by an earlier edit.
type SourcePostingLink = (String, Option<String>);

/// Re-points snapshotted `transaction_sources` rows at the postings that
/// survived a posting-set replace.
///
/// [`Service::apply_transaction_projection`] fully replaces a transaction's
/// postings. `transaction_sources.posting_id` is `ON DELETE SET NULL`, so that
/// replace clears every reference's posting link without destroying the
/// reference itself; this restores the link for each leg that kept its posting
/// id across the edit.
///
/// A leg the edit genuinely removed has no matching id among `new_postings`, so
/// its reference is left with a `NULL` `posting_id` — a tombstone. That is
/// deliberate: the reference is history of what the source document contained,
/// and it keeps its `(account_id, fingerprint, occurrence)` slot so re-importing
/// the same document does not recreate a leg the user chose to delete.
///
/// A tombstone also keeps its **original** `account_id`, even where the edit
/// recategorised the posting onto a different account. The reference describes
/// the source document, not the edited state, so stored provenance can name an
/// account that [`crate::SourceService::attach_in_tx`] would reject on a fresh
/// write.
///
/// # Arguments
///
/// * `db_tx` - The open SQLite transaction the caller is already writing within.
/// * `new_postings` - The postings just reinserted by the caller.
/// * `snapshot` - The reference-to-posting links captured before the delete.
///
/// # Errors
///
/// Returns [`BcError`] on database write failure.
async fn relink_surviving_sources(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    new_postings: &[Posting],
    snapshot: Vec<SourcePostingLink>,
) -> BcResult<()> {
    let surviving_posting_ids: std::collections::HashSet<String> = new_postings
        .iter()
        .map(|posting| posting.id().to_string())
        .collect();
    for (id, snapshotted) in snapshot {
        let Some(surviving) = snapshotted.filter(|p| surviving_posting_ids.contains(p.as_str()))
        else {
            continue;
        };
        sqlx::query("UPDATE transaction_sources SET posting_id = ? WHERE id = ?")
            .bind(surviving)
            .bind(id)
            .execute(&mut **db_tx)
            .await?;
    }
    Ok(())
}

/// Service for creating and managing transactions.
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns the underlying connection pool.
    ///
    /// Lets sibling services (e.g. import execution) begin a database
    /// transaction that spans a transaction create and its source-ref attach.
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Persists a transaction after validating double-entry balance.
    ///
    /// The event append and all projection inserts are wrapped in a single
    /// SQLite transaction so they succeed or fail atomically.
    ///
    /// # Warnings
    ///
    /// Returns advisory [`crate::Warning`]s alongside the result — a commodity
    /// outside the account's declared list, a date outside its declared life,
    /// or an archived account. None of these blocks the write.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, contains two
    /// or more elided amounts, or is a single lone elided posting.
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(&self, tx: Transaction) -> BcResult<crate::Warned<TransactionId>> {
        let mut db_tx = self.pool.begin().await?;
        let warned = self.create_in_tx(&mut db_tx, tx).await?;
        db_tx.commit().await?;
        tracing::info!(transaction_id = %warned.value, "transaction created");
        Ok(warned)
    }

    /// Persists a transaction within an already-open database transaction.
    ///
    /// Validates double-entry structure, appends the creation event, and writes
    /// every projection row using `db_tx`, letting a caller bundle the write with
    /// adjacent work (e.g. attaching an import source reference) into one atomic
    /// unit. The caller owns `db_tx` and must commit it; nothing is durable until
    /// then.
    ///
    /// # Arguments
    ///
    /// * `db_tx` - An open SQLite transaction to write within.
    /// * `tx` - The transaction to persist.
    ///
    /// # Returns
    ///
    /// The ID of the persisted transaction.
    ///
    /// # Warnings
    ///
    /// Returns advisory [`crate::Warning`]s alongside the result — a commodity
    /// outside the account's declared list, a date outside its declared life,
    /// or an archived account. None of these blocks the write.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, contains two
    /// or more elided amounts, or is a single lone elided posting.
    /// Returns [`BcError`] on event append or database insert failure.
    pub(crate) async fn create_in_tx(
        &self,
        db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        tx: Transaction,
    ) -> BcResult<crate::Warned<TransactionId>> {
        validate_postings(tx.postings())?;

        let warnings = crate::warning::check_postings(db_tx, tx.date(), tx.postings()).await?;

        let tx_id = tx.id().clone();
        let event = Event::TransactionCreated { id: tx_id.clone() };

        let date_str = tx.date().to_string();
        let created_at_str = tx.created_at().to_string();

        insert_event(&event, db_tx).await?;

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(tx_id.to_string())
        .bind(&date_str)
        .bind(tx.description())
        .bind(to_db_str(tx.reconciliation())?)
        .bind(&created_at_str)
        .execute(&mut **db_tx)
        .await?;

        crate::tag::insert_transaction_tags(&mut *db_tx, &tx_id, tx.tag_ids()).await?;

        crate::metadata::insert(
            db_tx,
            crate::metadata::Owner::Transaction,
            &tx_id.to_string(),
            tx.metadata(),
        )
        .await?;

        for (index, posting) in tx.postings().iter().enumerate() {
            let position = i64::try_from(index)
                .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?;
            insert_posting_row(db_tx, &tx_id, posting, position).await?;
            crate::tag::insert_posting_tags(&mut *db_tx, posting.id(), posting.tag_ids()).await?;
        }

        Ok(crate::Warned::new(tx_id, warnings))
    }

    /// Appends postings to an existing transaction within an open database
    /// transaction.
    ///
    /// Import needs this because a transaction's legs may arrive across several
    /// passes: a leg whose account did not exist on the first import is attached
    /// to the *same* transaction once that account is created, rather than
    /// creating a duplicate.
    ///
    /// New postings take positions after the highest existing one, so ordering
    /// reflects arrival and positions never collide. No `Event` is appended:
    /// posting mutations are projection-level, consistent with how
    /// `TransactionAmended` already treats them.
    ///
    /// # Arguments
    ///
    /// * `db_tx` - An open SQLite transaction to write within.
    /// * `transaction_id` - The transaction to append to.
    /// * `postings` - The postings to append. An empty slice is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if `transaction_id` names no transaction.
    /// Returns [`BcError::BadData`] if appending would leave two or more elided
    /// postings, since the residual would then be ambiguous.
    /// Returns [`BcError`] on database insert failure.
    pub(crate) async fn add_postings_in_tx(
        &self,
        db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        transaction_id: &TransactionId,
        postings: &[Posting],
    ) -> BcResult<()> {
        if postings.is_empty() {
            return Ok(());
        }

        let existing: Option<(i64, i64)> = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(MAX(position), -1) FROM postings WHERE transaction_id = ?",
        )
        .bind(transaction_id.to_string())
        .fetch_optional(&mut **db_tx)
        .await?;
        let (existing_count, max_position) =
            existing.ok_or_else(|| BcError::NotFound(transaction_id.to_string()))?;
        if existing_count == 0 {
            return Err(BcError::NotFound(transaction_id.to_string()));
        }

        let existing_elided: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM postings WHERE transaction_id = ? AND amount IS NULL",
        )
        .bind(transaction_id.to_string())
        .fetch_one(&mut **db_tx)
        .await?;
        let added_elided = i64::try_from(postings.iter().filter(|p| p.amount().is_none()).count())
            .map_err(|_err| BcError::BadData("elided posting count overflow".into()))?;
        if existing_elided.saturating_add(added_elided) >= 2 {
            return Err(BcError::BadData("two or more elided postings".into()));
        }

        for (offset, posting) in postings.iter().enumerate() {
            let position = max_position
                .saturating_add(1)
                .saturating_add(i64::try_from(offset).map_err(|_err| {
                    BcError::BadData("posting position exceeds i64::MAX".into())
                })?);
            insert_posting_row(db_tx, transaction_id, posting, position).await?;
            crate::tag::insert_posting_tags(&mut *db_tx, posting.id(), posting.tag_ids()).await?;
        }
        Ok(())
    }

    /// Finds a transaction by ID, including all its postings with cost and tag data.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "loading a transaction with postings, cost, spread, and tags requires several queries and field mappings"
    )]
    pub async fn find_by_id(&self, id: &TransactionId) -> BcResult<Transaction> {
        let tx_row = sqlx::query_as::<_, TxRow>(
            "SELECT id, date, description, reconciliation, created_at \
             FROM transactions WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(id.to_string()))?;

        let tx_id = tx_row
            .0
            .parse::<TransactionId>()
            .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;

        let date = tx_row
            .1
            .parse::<Date>()
            .map_err(|e| BcError::BadData(format!("invalid date '{}': {e}", tx_row.1)))?;

        let reconciliation = from_db_str::<Reconciliation>(&tx_row.3)?;

        let created_at = tx_row
            .4
            .parse::<Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at '{}': {e}", tx_row.4)))?;

        // Load transaction-level tag IDs.
        let tx_tag_rows: Vec<(String,)> =
            sqlx::query_as("SELECT tag_id FROM transaction_tags WHERE transaction_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        let tag_ids: Vec<TagId> = tx_tag_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        let id_str = id.to_string();
        let metadata = crate::metadata::load_for(
            &self.pool,
            crate::metadata::Owner::Transaction,
            &[id_str.as_str()],
        )
        .await?
        .remove(&id_str)
        .unwrap_or_default();

        // Load postings with cost and spread columns.
        let posting_rows: Vec<PostingRow> = sqlx::query_as(
            "SELECT id, account_id, amount, commodity, \
                    cost_total_value, cost_total_commodity, cost_date, cost_label, \
                    spread_from, spread_until \
             FROM postings WHERE transaction_id = ? ORDER BY position ASC",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        // Load all posting tag IDs for this transaction in one query.
        let posting_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt \
             JOIN postings p ON pt.posting_id = p.id \
             WHERE p.transaction_id = ?",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut posting_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (posting_id, tag_id_str) in posting_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            posting_tags_map.entry(posting_id).or_default().push(tid);
        }

        let posting_ids: Vec<&str> = posting_rows.iter().map(|row| row.id.as_str()).collect();
        let mut posting_metadata =
            crate::metadata::load_for(&self.pool, crate::metadata::Owner::Posting, &posting_ids)
                .await?;

        let postings = posting_rows
            .into_iter()
            .map(|row| {
                let pid = row.id;
                let posting_id = pid
                    .parse::<PostingId>()
                    .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
                let acc_id = row.account_id.parse::<AccountId>().map_err(|e| {
                    BcError::BadData(format!("invalid account id '{}': {e}", row.account_id))
                })?;
                let amount = match (row.amount, row.commodity) {
                    (Some(v), Some(c)) => {
                        let value = v
                            .parse::<Decimal>()
                            .map_err(|e| BcError::BadData(format!("invalid amount '{v}': {e}")))?;
                        Some(Amount::new(value, CommodityCode::new(c)))
                    }
                    _ => None,
                };
                let cost = parse_cost(
                    row.cost_total_value,
                    row.cost_total_commodity,
                    row.cost_date,
                    row.cost_label,
                )?;
                let spread_from = row
                    .spread_from
                    .as_deref()
                    .map(|s| {
                        s.parse::<Date>().map_err(|e| {
                            BcError::BadData(format!("invalid spread_from '{s}': {e}"))
                        })
                    })
                    .transpose()?;
                let spread_until = row
                    .spread_until
                    .as_deref()
                    .map(|s| {
                        s.parse::<Date>().map_err(|e| {
                            BcError::BadData(format!("invalid spread_until '{s}': {e}"))
                        })
                    })
                    .transpose()?;
                let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
                let p_metadata = posting_metadata.remove(&pid).unwrap_or_default();
                Ok(Posting::builder()
                    .id(posting_id)
                    .account_id(acc_id)
                    .maybe_amount(amount)
                    .maybe_cost(cost)
                    .metadata(p_metadata)
                    .maybe_spread_from(spread_from)
                    .maybe_spread_until(spread_until)
                    .tag_ids(p_tag_ids)
                    .build())
            })
            .collect::<BcResult<Vec<_>>>()?;

        Ok(Transaction::builder()
            .id(tx_id)
            .date(date)
            .description(tx_row.2)
            .postings(postings)
            .reconciliation(reconciliation)
            .tag_ids(tag_ids)
            .metadata(metadata)
            .created_at(created_at)
            .build())
    }

    /// Creates a reversal transaction for the given transaction.
    ///
    /// A reversal inserts a new transaction with the same postings negated, a
    /// description of `"Reversal of {id}"`, and `Reconciliation::Unreconciled`.
    /// The reversal relationship is recorded solely by the
    /// [`crate::Event::TransactionReversed`] event; no projection table stores it.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with the given ID exists.
    /// Returns [`BcError`] on database insert failure.
    #[inline]
    pub async fn reverse(&self, id: &TransactionId) -> BcResult<TransactionId> {
        let original = self.find_by_id(id).await?;

        let reversal_id = TransactionId::new();
        let created_at_str = Timestamp::now().to_string();
        let unreconciled_str = to_db_str(Reconciliation::Unreconciled)?;
        let description = format!("Reversal of {id}");

        let event = Event::TransactionReversed {
            original_id: id.clone(),
            reversal_id: reversal_id.clone(),
        };

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        // Insert the reversal transaction row.
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(reversal_id.to_string())
        .bind(original.date().to_string())
        .bind(&description)
        .bind(&unreconciled_str)
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        crate::metadata::insert(
            &mut db_tx,
            crate::metadata::Owner::Transaction,
            &reversal_id.to_string(),
            original.metadata(),
        )
        .await?;

        // Insert negated postings for the reversal. Each is a fresh leg with a
        // new id carrying the original's cost, spread and metadata: a reversal
        // describes the same real-world event, so its annotations travel with
        // it.
        for (index, posting) in original.postings().iter().enumerate() {
            let negated = posting
                .amount()
                .map(|amount| {
                    amount
                        .value()
                        .checked_mul(Decimal::NEGATIVE_ONE)
                        .map(|value| Amount::new(value, amount.commodity().clone()))
                        .ok_or_else(|| BcError::BadData("posting amount negation overflow".into()))
                })
                .transpose()?;

            let reversed = Posting::builder()
                .id(PostingId::new())
                .account_id(posting.account_id().clone())
                .maybe_amount(negated)
                .maybe_cost(posting.cost().cloned())
                .metadata(posting.metadata().clone())
                .maybe_spread_from(posting.spread_from())
                .maybe_spread_until(posting.spread_until())
                .build();

            let position = i64::try_from(index)
                .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?;
            insert_posting_row(&mut db_tx, &reversal_id, &reversed, position).await?;
        }

        db_tx.commit().await?;
        tracing::info!(original_id = %id, reversal_id = %reversal_id, "transaction reversed");
        Ok(reversal_id)
    }

    /// Lists all transactions ordered by date descending, including postings.
    ///
    /// Postings, tags, and cost data are loaded via separate queries to avoid N+1
    /// round-trips.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "loading transactions with postings, cost, and tags inherently requires several queries and field mappings"
    )]
    pub async fn list(&self) -> BcResult<Vec<Transaction>> {
        let tx_rows: Vec<TxRow> = sqlx::query_as(
            "SELECT id, date, description, reconciliation, created_at \
                 FROM transactions ORDER BY date DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        if tx_rows.is_empty() {
            return Ok(vec![]);
        }

        // Load all transaction-level tags in one query.
        let tx_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT tt.transaction_id, tt.tag_id \
             FROM transaction_tags tt",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut tx_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (tx_id_str, tag_id_str) in tx_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            tx_tags_map.entry(tx_id_str).or_default().push(tid);
        }

        // Load metadata for the matching transactions in one query.
        let tx_id_strs: Vec<&str> = tx_rows.iter().map(|row| row.0.as_str()).collect();
        let mut tx_metadata_map =
            crate::metadata::load_for(&self.pool, crate::metadata::Owner::Transaction, &tx_id_strs)
                .await?;

        // Load all postings in one query.
        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, \
                    p.cost_total_value, p.cost_total_commodity, p.cost_date, p.cost_label, \
                    p.spread_from, p.spread_until \
             FROM postings p \
             ORDER BY p.transaction_id, p.position ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        // Load all posting tags in one query.
        let posting_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut posting_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (posting_id, tag_id_str) in posting_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            posting_tags_map.entry(posting_id).or_default().push(tid);
        }

        // Group postings by transaction_id.
        let posting_id_strs: Vec<&str> = posting_rows.iter().map(|row| row.id.as_str()).collect();
        let mut posting_metadata_map = crate::metadata::load_for(
            &self.pool,
            crate::metadata::Owner::Posting,
            &posting_id_strs,
        )
        .await?;

        let mut postings_by_tx: HashMap<String, Vec<Posting>> = HashMap::new();
        for row in posting_rows {
            let pid = row.id;
            let tx_id_str = row.transaction_id;
            let posting_id = pid
                .parse::<PostingId>()
                .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
            let acc_id = row.account_id.parse::<AccountId>().map_err(|e| {
                BcError::BadData(format!("invalid account id '{}': {e}", row.account_id))
            })?;
            let amount = match (row.amount, row.commodity) {
                (Some(v), Some(c)) => {
                    let value = v
                        .parse::<Decimal>()
                        .map_err(|e| BcError::BadData(format!("invalid amount '{v}': {e}")))?;
                    Some(Amount::new(value, CommodityCode::new(c)))
                }
                _ => None,
            };
            let cost = parse_cost(
                row.cost_total_value,
                row.cost_total_commodity,
                row.cost_date,
                row.cost_label,
            )?;
            let spread_from = row
                .spread_from
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_from '{s}': {e}")))
                })
                .transpose()?;
            let spread_until = row
                .spread_until
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_until '{s}': {e}")))
                })
                .transpose()?;
            let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
            let p_metadata = posting_metadata_map.remove(&pid).unwrap_or_default();
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .metadata(p_metadata)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(id_str, date_str, description, reconciliation_str, created_at_str)| {
                    let tx_id = id_str
                        .parse::<TransactionId>()
                        .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;
                    let date = date_str
                        .parse::<jiff::civil::Date>()
                        .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                    let reconciliation = from_db_str::<Reconciliation>(&reconciliation_str)?;
                    let created_at = created_at_str.parse::<Timestamp>().map_err(|e| {
                        BcError::BadData(format!("invalid created_at '{created_at_str}': {e}"))
                    })?;
                    let tag_ids = tx_tags_map.remove(&id_str).unwrap_or_default();
                    let metadata = tx_metadata_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .metadata(metadata)
                        .created_at(created_at)
                        .build())
                },
            )
            .collect()
    }

    /// Lists all transactions that have at least one posting for the
    /// given account, ordered by date descending.
    ///
    /// Issues four targeted queries against SQLite (transactions, tx-tags, postings,
    /// posting-tags), each scoped to the given account via a subquery, so only the
    /// relevant rows are fetched rather than the full table.
    ///
    /// # Arguments
    ///
    /// * `account_id` - Only transactions with a posting referencing this account
    ///   are returned.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_for_account(
        &self,
        account_id: &AccountId,
    ) -> BcResult<impl Iterator<Item = Transaction>> {
        let account_id_str = account_id.to_string();

        let tx_rows: Vec<TxRow> = sqlx::query_as(
            "SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                 FROM transactions t \
                 WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?) \
                 ORDER BY t.date DESC",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        self.assemble_transactions(tx_rows).await
    }

    /// Lists transactions involving `account_id` whose canonical date falls in the
    /// half-open interval `[from, until)`, newest first.
    ///
    /// Filters strictly on `t.date` (no accrual-spread overlap).
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `from` - Inclusive lower bound on `t.date`.
    /// * `until` - Exclusive upper bound on `t.date`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data-parse failure.
    pub async fn list_for_account_in_range(
        &self,
        account_id: &AccountId,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
    ) -> BcResult<impl Iterator<Item = Transaction>> {
        let account_id_str = account_id.to_string();
        let from_str = from.to_string();
        let until_str = until.to_string();

        let tx_rows: Vec<TxRow> = sqlx::query_as(
            "SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                 FROM transactions t \
                 WHERE t.date >= ? AND t.date < ? \
                   AND t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?) \
                 ORDER BY t.date DESC",
        )
        .bind(&from_str)
        .bind(&until_str)
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        self.assemble_transactions(tx_rows).await
    }

    /// Loads tx-tags, extra dates, and postings for `tx_rows` and assembles the
    /// full [`Transaction`] values.
    ///
    /// Shared by [`Service::list_for_account`] and
    /// [`Service::list_for_account_in_range`], which differ only in how
    /// `tx_rows` is selected. All sub-queries are scoped to the transaction IDs
    /// already present in `tx_rows` (not the account's full history), so the
    /// work tracks the selected window rather than the account lifetime.
    #[expect(
        clippy::too_many_lines,
        reason = "loading transactions with postings, cost, and tags for a specific account inherently requires several queries and field mappings"
    )]
    pub(crate) async fn assemble_transactions(
        &self,
        tx_rows: Vec<TxRow>,
    ) -> BcResult<impl Iterator<Item = Transaction> + use<>> {
        if tx_rows.is_empty() {
            return Ok(vec![].into_iter());
        }

        // Bind the already-selected transaction IDs into every sub-query so each
        // one touches only these rows, not the whole account history.
        let tx_ids: Vec<&str> = tx_rows.iter().map(|row| row.0.as_str()).collect();
        let placeholders = sql_placeholders(tx_ids.len());

        let tx_tag_query = format!(
            "SELECT tt.transaction_id, tt.tag_id \
             FROM transaction_tags tt \
             WHERE tt.transaction_id IN ({placeholders})"
        );
        let mut tx_tag_stmt = sqlx::query_as(sqlx::AssertSqlSafe(tx_tag_query));
        for id in &tx_ids {
            tx_tag_stmt = tx_tag_stmt.bind(*id);
        }
        let tx_tag_rows: Vec<(String, String)> = tx_tag_stmt.fetch_all(&self.pool).await?;

        let mut tx_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (tx_id_str, tag_id_str) in tx_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            tx_tags_map.entry(tx_id_str).or_default().push(tid);
        }

        // Load metadata for the matching transactions in one query.
        let mut tx_metadata_map =
            crate::metadata::load_for(&self.pool, crate::metadata::Owner::Transaction, &tx_ids)
                .await?;

        let posting_query = format!(
            "SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, \
                    p.cost_total_value, p.cost_total_commodity, p.cost_date, p.cost_label, \
                    p.spread_from, p.spread_until \
             FROM postings p \
             WHERE p.transaction_id IN ({placeholders}) \
             ORDER BY p.transaction_id, p.position ASC"
        );
        let mut posting_stmt = sqlx::query_as(sqlx::AssertSqlSafe(posting_query));
        for id in &tx_ids {
            posting_stmt = posting_stmt.bind(*id);
        }
        let posting_rows: Vec<ListPostingRow> = posting_stmt.fetch_all(&self.pool).await?;

        let posting_tag_query = format!(
            "SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt \
             JOIN postings p ON pt.posting_id = p.id \
             WHERE p.transaction_id IN ({placeholders})"
        );
        let mut posting_tag_stmt = sqlx::query_as(sqlx::AssertSqlSafe(posting_tag_query));
        for id in &tx_ids {
            posting_tag_stmt = posting_tag_stmt.bind(*id);
        }
        let posting_tag_rows: Vec<(String, String)> =
            posting_tag_stmt.fetch_all(&self.pool).await?;

        let mut posting_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (posting_id, tag_id_str) in posting_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            posting_tags_map.entry(posting_id).or_default().push(tid);
        }

        let posting_id_strs: Vec<&str> = posting_rows.iter().map(|row| row.id.as_str()).collect();
        let mut posting_metadata_map = crate::metadata::load_for(
            &self.pool,
            crate::metadata::Owner::Posting,
            &posting_id_strs,
        )
        .await?;

        let mut postings_by_tx: HashMap<String, Vec<Posting>> = HashMap::new();
        for row in posting_rows {
            let pid = row.id;
            let tx_id_str = row.transaction_id;
            let posting_id = pid
                .parse::<PostingId>()
                .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
            let acc_id = row.account_id.parse::<AccountId>().map_err(|e| {
                BcError::BadData(format!("invalid account id '{}': {e}", row.account_id))
            })?;
            let amount = match (row.amount, row.commodity) {
                (Some(v), Some(c)) => {
                    let value = v
                        .parse::<Decimal>()
                        .map_err(|e| BcError::BadData(format!("invalid amount '{v}': {e}")))?;
                    Some(Amount::new(value, CommodityCode::new(c)))
                }
                _ => None,
            };
            let cost = parse_cost(
                row.cost_total_value,
                row.cost_total_commodity,
                row.cost_date,
                row.cost_label,
            )?;
            let spread_from = row
                .spread_from
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_from '{s}': {e}")))
                })
                .transpose()?;
            let spread_until = row
                .spread_until
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_until '{s}': {e}")))
                })
                .transpose()?;
            let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
            let p_metadata = posting_metadata_map.remove(&pid).unwrap_or_default();
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .metadata(p_metadata)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(id_str, date_str, description, reconciliation_str, created_at_str)| {
                    let tx_id = id_str
                        .parse::<TransactionId>()
                        .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;
                    let date = date_str
                        .parse::<jiff::civil::Date>()
                        .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                    let reconciliation = from_db_str::<Reconciliation>(&reconciliation_str)?;
                    let created_at = created_at_str.parse::<Timestamp>().map_err(|e| {
                        BcError::BadData(format!("invalid created_at '{created_at_str}': {e}"))
                    })?;
                    let tag_ids = tx_tags_map.remove(&id_str).unwrap_or_default();
                    let metadata = tx_metadata_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .metadata(metadata)
                        .created_at(created_at)
                        .build())
                },
            )
            .collect::<BcResult<Vec<_>>>()
            .map(IntoIterator::into_iter)
    }

    /// Lists all transactions that involve `account_id` or any of its
    /// descendant accounts in the account hierarchy.
    ///
    /// Uses a `WITH RECURSIVE` CTE to collect the full subtree of account IDs
    /// rooted at `account_id`, then returns all transactions that have at least
    /// one posting in that subtree.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The root of the account subtree to query.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_for_account_tree(
        &self,
        account_id: &AccountId,
    ) -> BcResult<impl Iterator<Item = Transaction>> {
        self.list_for_account_tree_in_range(account_id, None, None)
            .await
    }

    /// Lists all transactions that involve `account_id` or any of its
    /// descendant accounts, optionally restricted to a date half-open interval
    /// `[date_from, date_until)`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The root of the account subtree to query.
    /// * `date_from` - Inclusive lower bound on `t.date`, or `None` for no lower bound.
    /// * `date_until` - Exclusive upper bound on `t.date`, or `None` for no upper bound.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[expect(
        clippy::too_many_lines,
        reason = "loading transactions with postings, cost, and tags for an account subtree inherently requires several queries and field mappings"
    )]
    async fn list_for_account_tree_in_range(
        &self,
        account_id: &AccountId,
        date_from: Option<jiff::civil::Date>,
        date_until: Option<jiff::civil::Date>,
    ) -> BcResult<impl Iterator<Item = Transaction>> {
        let account_id_str = account_id.to_string();
        let date_from_str = date_from.map(|d| d.to_string());
        let date_until_str = date_until.map(|d| d.to_string());

        let tx_rows: Vec<TxRow> =
            match (&date_from_str, &date_until_str) {
                (Some(from), Some(until)) => sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) \
                     SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                     FROM transactions t \
                     WHERE t.date >= ? AND t.date < ? \
                     AND t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree)) \
                     ORDER BY t.date DESC",
                )
                .bind(&account_id_str)
                .bind(from)
                .bind(until)
                .fetch_all(&self.pool)
                .await?,
                (Some(from), None) => sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) \
                     SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                     FROM transactions t \
                     WHERE t.date >= ? \
                     AND t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree)) \
                     ORDER BY t.date DESC",
                )
                .bind(&account_id_str)
                .bind(from)
                .fetch_all(&self.pool)
                .await?,
                (None, Some(until)) => sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) \
                     SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                     FROM transactions t \
                     WHERE t.date < ? \
                     AND t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree)) \
                     ORDER BY t.date DESC",
                )
                .bind(&account_id_str)
                .bind(until)
                .fetch_all(&self.pool)
                .await?,
                (None, None) => sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) \
                     SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
                     FROM transactions t \
                     WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree)) \
                     ORDER BY t.date DESC",
                )
                .bind(&account_id_str)
                .fetch_all(&self.pool)
                .await?,
            };

        if tx_rows.is_empty() {
            return Ok(vec![].into_iter());
        }

        let tx_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) \
             SELECT tt.transaction_id, tt.tag_id \
             FROM transaction_tags tt \
             JOIN transactions t ON tt.transaction_id = t.id \
             WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree))",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let mut tx_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (tx_id_str, tag_id_str) in tx_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            tx_tags_map.entry(tx_id_str).or_default().push(tid);
        }

        // Load metadata for the matching transactions in one query.
        let tx_id_strs: Vec<&str> = tx_rows.iter().map(|row| row.0.as_str()).collect();
        let mut tx_metadata_map =
            crate::metadata::load_for(&self.pool, crate::metadata::Owner::Transaction, &tx_id_strs)
                .await?;

        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) \
             SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, \
                    p.cost_total_value, p.cost_total_commodity, p.cost_date, p.cost_label, \
                    p.spread_from, p.spread_until \
             FROM postings p \
             WHERE p.transaction_id IN \
                 (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree)) \
             ORDER BY p.transaction_id, p.position ASC",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let posting_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) \
             SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt \
             JOIN postings p ON pt.posting_id = p.id \
             WHERE p.transaction_id IN \
                 (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree))",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let mut posting_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (posting_id, tag_id_str) in posting_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            posting_tags_map.entry(posting_id).or_default().push(tid);
        }

        let posting_id_strs: Vec<&str> = posting_rows.iter().map(|row| row.id.as_str()).collect();
        let mut posting_metadata_map = crate::metadata::load_for(
            &self.pool,
            crate::metadata::Owner::Posting,
            &posting_id_strs,
        )
        .await?;

        let mut postings_by_tx: HashMap<String, Vec<Posting>> = HashMap::new();
        for row in posting_rows {
            let pid = row.id;
            let tx_id_str = row.transaction_id;
            let posting_id = pid
                .parse::<PostingId>()
                .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
            let acc_id = row.account_id.parse::<AccountId>().map_err(|e| {
                BcError::BadData(format!("invalid account id '{}': {e}", row.account_id))
            })?;
            let amount = match (row.amount, row.commodity) {
                (Some(v), Some(c)) => {
                    let value = v
                        .parse::<Decimal>()
                        .map_err(|e| BcError::BadData(format!("invalid amount '{v}': {e}")))?;
                    Some(Amount::new(value, CommodityCode::new(c)))
                }
                _ => None,
            };
            let cost = parse_cost(
                row.cost_total_value,
                row.cost_total_commodity,
                row.cost_date,
                row.cost_label,
            )?;
            let spread_from = row
                .spread_from
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_from '{s}': {e}")))
                })
                .transpose()?;
            let spread_until = row
                .spread_until
                .as_deref()
                .map(|s| {
                    s.parse::<Date>()
                        .map_err(|e| BcError::BadData(format!("invalid spread_until '{s}': {e}")))
                })
                .transpose()?;
            let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
            let p_metadata = posting_metadata_map.remove(&pid).unwrap_or_default();
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .metadata(p_metadata)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(id_str, date_str, description, reconciliation_str, created_at_str)| {
                    let tx_id = id_str
                        .parse::<TransactionId>()
                        .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;
                    let date = date_str
                        .parse::<jiff::civil::Date>()
                        .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                    let reconciliation = from_db_str::<Reconciliation>(&reconciliation_str)?;
                    let created_at = created_at_str.parse::<Timestamp>().map_err(|e| {
                        BcError::BadData(format!("invalid created_at '{created_at_str}': {e}"))
                    })?;
                    let tag_ids = tx_tags_map.remove(&id_str).unwrap_or_default();
                    let metadata = tx_metadata_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .metadata(metadata)
                        .created_at(created_at)
                        .build())
                },
            )
            .collect::<BcResult<Vec<_>>>()
            .map(IntoIterator::into_iter)
    }

    /// Rewrites the projection tables for `updated` within an open DB transaction.
    ///
    /// Updates the `transactions` row and fully replaces the transaction's
    /// postings, posting tags, transaction tags, and metadata.
    ///
    /// # Arguments
    ///
    /// * `db_tx` - An open SQLite transaction to write within.
    /// * `updated` - The desired transaction state.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no `transactions` row matches the ID.
    /// Returns [`BcError`] on any database write failure.
    async fn apply_transaction_projection(
        &self,
        db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        updated: &Transaction,
    ) -> BcResult<()> {
        let tx_id_str = updated.id().to_string();
        let date_str = updated.date().to_string();

        let result = sqlx::query("UPDATE transactions SET date = ?, description = ? WHERE id = ?")
            .bind(&date_str)
            .bind(updated.description())
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(tx_id_str));
        }

        // Delete posting_tags first to satisfy the FK constraint
        // `posting_tags.posting_id REFERENCES postings(id)` enforced by
        // `PRAGMA foreign_keys = ON`.
        sqlx::query(
            "DELETE FROM posting_tags WHERE posting_id IN \
             (SELECT id FROM postings WHERE transaction_id = ?)",
        )
        .bind(&tx_id_str)
        .execute(&mut **db_tx)
        .await?;

        // `transaction_sources.posting_id` is `ON DELETE SET NULL`, so the DELETE
        // below clears every reference's posting link. Snapshot the links first so
        // the legs that survive the replace can be re-pointed at their postings.
        let source_snapshot: Vec<SourcePostingLink> = sqlx::query_as(
            "SELECT id, posting_id FROM transaction_sources WHERE transaction_id = ?",
        )
        .bind(&tx_id_str)
        .fetch_all(&mut **db_tx)
        .await?;

        // `posting_metadata.posting_id` is a plain foreign key, so the entries
        // go before the postings that own them.
        crate::metadata::delete_for_transaction_postings(db_tx, &tx_id_str).await?;

        sqlx::query("DELETE FROM postings WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        crate::metadata::delete_for(db_tx, crate::metadata::Owner::Transaction, &tx_id_str).await?;

        crate::tag::insert_transaction_tags(&mut *db_tx, updated.id(), updated.tag_ids()).await?;

        crate::metadata::insert(
            db_tx,
            crate::metadata::Owner::Transaction,
            &tx_id_str,
            updated.metadata(),
        )
        .await?;

        for (index, posting) in updated.postings().iter().enumerate() {
            let position = i64::try_from(index)
                .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?;
            insert_posting_row(db_tx, updated.id(), posting, position).await?;
            crate::tag::insert_posting_tags(&mut *db_tx, posting.id(), posting.tag_ids()).await?;
        }

        relink_surviving_sources(db_tx, updated.postings(), source_snapshot).await?;

        Ok(())
    }

    /// Amends an existing transaction, replacing its projection row and all postings atomically.
    ///
    /// The event append, projection UPDATE, posting DELETE/INSERT, and tag DELETE/INSERT
    /// are all wrapped in a single SQLite transaction so they succeed or fail atomically.
    /// `posting_tags` rows are deleted before `postings` rows to satisfy the FK constraint
    /// `posting_tags.posting_id REFERENCES postings(id)` enforced by `PRAGMA foreign_keys = ON`.
    ///
    /// `reconciliation` is intentionally **not** updated here — it may only advance
    /// through [`Service::reconcile`], which enforces the `balanced()` invariant before
    /// allowing a transition to [`Reconciliation::Reconciled`].
    ///
    /// # Arguments
    ///
    /// * `updated` - The new transaction state. Must carry the same [`TransactionId`]
    ///   as the existing transaction. All postings are replaced.
    ///
    /// # Events
    ///
    /// Appends [`Event::TransactionAmended`] for the scalar fields, and
    /// [`Event::TransactionMetadataChanged`] when the metadata list differs
    /// from the stored one — this call replaces that list, so leaving the
    /// change unrecorded would put a stored edit outside the log. Postings and
    /// tags are replaced without an event of their own;
    /// [`Service::edit`] is the path that decomposes those.
    ///
    /// # Warnings
    ///
    /// Returns advisory [`crate::Warning`]s alongside the result — a commodity
    /// outside the account's declared list, a date outside its declared life,
    /// or an archived account. None of these blocks the write.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, contains two
    /// or more elided amounts, or is a single lone elided posting.
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn amend(&self, updated: Transaction) -> BcResult<crate::Warned<()>> {
        validate_postings(updated.postings())?;

        let tx_id = updated.id().clone();
        let current = self.find_by_id(&tx_id).await?;
        let mut events = vec![Event::TransactionAmended {
            id: tx_id.clone(),
            date: updated.date(),
            description: updated.description().to_owned(),
        }];
        if !current
            .metadata()
            .eq_ignoring_mismatched(updated.metadata())
        {
            events.push(Event::TransactionMetadataChanged {
                id: tx_id.clone(),
                before: current.metadata().clone(),
                after: updated.metadata().clone(),
            });
        }

        let mut db_tx = self.pool.begin().await?;
        let warnings =
            crate::warning::check_postings(&mut db_tx, updated.date(), updated.postings()).await?;
        for event in &events {
            insert_event(event, &mut db_tx).await?;
        }
        self.apply_transaction_projection(&mut db_tx, &updated)
            .await?;
        db_tx.commit().await?;
        tracing::info!(transaction_id = %tx_id, "transaction amended");
        Ok(crate::Warned::new((), warnings))
    }

    /// Applies a desired transaction state, recording decomposed semantic events.
    ///
    /// Loads the current state, diffs it against `updated` to produce granular
    /// events (date, description, tags and metadata, and per-posting
    /// recategorise / amount / spread / metadata / add / remove), then
    /// atomically appends those events and rewrites the projection.
    /// Persistence is permissive: an unbalanced result is allowed.
    ///
    /// # Arguments
    ///
    /// * `updated` - The desired transaction state. Must carry the ID of an
    ///   existing transaction; all postings are replaced.
    ///
    /// # Warnings
    ///
    /// Returns advisory [`crate::Warning`]s alongside the result — a commodity
    /// outside the account's declared list, a date outside its declared life,
    /// or an archived account. None of these blocks the write.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, has ≥2 elided
    /// amounts, or is a lone elided posting. Returns [`BcError::NotFound`] if no
    /// transaction with that ID exists. Returns [`BcError`] on DB failure.
    #[inline]
    pub async fn edit(&self, updated: Transaction) -> BcResult<crate::Warned<()>> {
        validate_postings(updated.postings())?;

        let tx_id = updated.id().clone();
        let current = self.find_by_id(&tx_id).await?;
        let merged = merge_preserving(&current, &updated);
        let events = diff_transaction(&current, &merged);

        let mut db_tx = self.pool.begin().await?;
        let warnings =
            crate::warning::check_postings(&mut db_tx, merged.date(), merged.postings()).await?;
        for event in &events {
            insert_event(event, &mut db_tx).await?;
        }
        self.apply_transaction_projection(&mut db_tx, &merged)
            .await?;
        db_tx.commit().await?;

        tracing::info!(transaction_id = %tx_id, event_count = events.len(), "transaction edited");
        Ok(crate::Warned::new((), warnings))
    }

    /// Lists all transactions with a posting against `account_id`
    /// (optionally filtered to postings tagged with `tag_filter`) in
    /// `[period_start, period_end)`, additionally narrowed by the global
    /// transaction filter `query` so the drill-down list matches what the
    /// budget tree counted for the same filter.
    ///
    /// Because the tag filter is now time-varying (it lives on a
    /// [`bc_models::BudgetRevision`]), callers must resolve the governing
    /// revision themselves and pass the filter explicitly.
    ///
    /// # Filtering approach
    ///
    /// The candidate transactions (account tree ∩ date range) are fetched in
    /// SQL via [`Self::list_for_account_tree_in_range`]; the budget's own tag
    /// filter and the global `query`'s dimensions are then applied in Rust over
    /// the assembled (small — one budget, one period) list. A transaction is
    /// kept iff it contains at least one posting in the budget account's subtree
    /// that satisfies the *same* counted-posting conjunction the budget tree
    /// sums over, so the drill-down list and the tree count never disagree —
    /// see [`transaction_matches_query`] for the exact predicate. Transaction
    /// tags flow down to every posting, matching the tree's SQL.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose posting tree to search.
    /// * `tag_filter` - Optional tag; a transaction is kept when a budget-subtree
    ///   posting carries this tag or one of its descendants, counting the
    ///   transaction's own tags as flowing down to every posting. `None` = no
    ///   tag filter.
    /// * `period_start` - Inclusive start of the date range.
    /// * `period_end` - Exclusive end of the date range.
    /// * `query` - Optional global transaction filter narrowing the result to
    ///   what the budget tree counted. `None` = no additional filtering.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_for_budget(
        &self,
        account_id: &bc_models::AccountId,
        tag_filter: Option<&bc_models::TagId>,
        period_start: jiff::civil::Date,
        period_end: jiff::civil::Date,
        query: Option<&crate::search::TransactionQuery>,
    ) -> BcResult<Vec<Transaction>> {
        let fetched: Vec<Transaction> = self
            .list_for_account_tree_in_range(account_id, Some(period_start), Some(period_end))
            .await?
            .collect();

        // Resolve the budget account's own subtree (the tree's `acct_tree`), so
        // every parity check counts only a posting *inside the budget*, matching
        // what the budget tree sums.
        let Some(budget_subtree) =
            crate::search::resolve_account_subtrees(&self.pool, core::slice::from_ref(account_id))
                .await?
        else {
            return Ok(Vec::new());
        };

        // Expand the budget's own tag filter to its inclusive subtree once, so a
        // descendant tag counts the same as the parent — mirroring the tree's
        // `tag_subtree` CTE.
        let tag_subtree = match tag_filter {
            Some(tag) => Some(crate::search::resolve_tag_subtree(&self.pool, tag).await?),
            None => None,
        };

        let Some(q) = query else {
            // No global filter: keep the budget's own tag filter, requiring a
            // budget-subtree posting to carry the tag (its own or its
            // transaction's), so the list agrees with the tree count.
            let out = match &tag_subtree {
                Some(subtree) => fetched
                    .into_iter()
                    .filter(|tx| budget_leg_carries_tag(tx, &budget_subtree, subtree))
                    .collect(),
                None => fetched,
            };
            return Ok(out);
        };

        // The global filter's account subtree, so the per-posting predicate can
        // require the *same* budget-subtree posting to satisfy every active
        // dimension — matching what the budget tree counts.
        let global_accounts =
            crate::search::resolve_account_subtrees(&self.pool, &q.accounts).await?;

        let result = fetched
            .into_iter()
            .filter(|tx| {
                transaction_matches_query(
                    tx,
                    q,
                    &budget_subtree,
                    tag_subtree.as_ref(),
                    global_accounts.as_ref(),
                )
            })
            .collect();

        Ok(result)
    }

    /// Sets the accrual spread date range on a posting.
    ///
    /// # Arguments
    ///
    /// * `id` - The posting to update.
    /// * `spread_from` - First day of the accrual window (inclusive).
    /// * `spread_until` - Last day of the accrual window (inclusive).
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no posting with `id` exists.
    /// Returns [`crate::BcError`] on database failure.
    #[inline]
    pub async fn set_posting_spread(
        &self,
        id: &PostingId,
        spread_from: Date,
        spread_until: Date,
    ) -> BcResult<()> {
        if spread_from > spread_until {
            return Err(BcError::InvalidInput(
                "spread_from must not be after spread_until".into(),
            ));
        }

        let result =
            sqlx::query("UPDATE postings SET spread_from = ?, spread_until = ? WHERE id = ?")
                .bind(spread_from.to_string())
                .bind(spread_until.to_string())
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(id.to_string()));
        }
        tracing::info!(posting_id = %id, %spread_from, %spread_until, "posting spread set");
        Ok(())
    }

    /// Clears the accrual spread from a posting.
    ///
    /// # Arguments
    ///
    /// * `id` - The posting to update.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no posting with `id` exists.
    /// Returns [`crate::BcError`] on database failure.
    #[inline]
    pub async fn clear_posting_spread(&self, id: &PostingId) -> BcResult<()> {
        let result =
            sqlx::query("UPDATE postings SET spread_from = NULL, spread_until = NULL WHERE id = ?")
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(id.to_string()));
        }
        tracing::info!(posting_id = %id, "posting spread cleared");
        Ok(())
    }

    /// Sets the reconciliation state of a transaction.
    ///
    /// Setting [`Reconciliation::Reconciled`] requires the transaction to
    /// balance — an unbalanced (one-sided or multi-commodity) transaction cannot
    /// be reconciled until the missing leg is supplied.
    ///
    /// # Arguments
    ///
    /// * `id` - The transaction to update.
    /// * `state` - The new reconciliation state.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if `state` is `Reconciled` and the
    /// transaction does not balance.
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on database update failure.
    pub async fn reconcile(&self, id: &TransactionId, state: Reconciliation) -> BcResult<()> {
        let tx = self.find_by_id(id).await?;
        if state == Reconciliation::Reconciled && !tx.balanced() {
            return Err(BcError::BadData(
                "cannot reconcile an unbalanced transaction".into(),
            ));
        }

        let from = tx.reconciliation();
        if from == state {
            return Ok(());
        }

        let event = Event::TransactionReconciled {
            id: id.clone(),
            from,
            to: state,
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;
        sqlx::query("UPDATE transactions SET reconciliation = ? WHERE id = ?")
            .bind(to_db_str(state)?)
            .bind(id.to_string())
            .execute(&mut *db_tx)
            .await?;
        db_tx.commit().await?;

        tracing::info!(transaction_id = %id, ?state, "transaction reconciliation set");
        Ok(())
    }

    /// Returns the event trail for a transaction, oldest first.
    ///
    /// # Arguments
    ///
    /// * `id` - The transaction whose events to load.
    ///
    /// # Returns
    ///
    /// `(created_at, event)` pairs ordered by insertion.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a stored timestamp or payload cannot be
    /// parsed. Returns [`BcError`] on DB failure.
    #[inline]
    pub async fn audit_trail(
        &self,
        id: &TransactionId,
    ) -> BcResult<Vec<(jiff::Timestamp, crate::events::Event)>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT payload, created_at FROM events WHERE aggregate_id = ? ORDER BY rowid ASC",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(payload, created_at)| {
                let event: crate::events::Event = serde_json::from_str(&payload)
                    .map_err(|e| BcError::BadData(format!("invalid event payload: {e}")))?;
                let ts = created_at.parse::<jiff::Timestamp>().map_err(|e| {
                    BcError::BadData(format!("invalid created_at '{created_at}': {e}"))
                })?;
                Ok((ts, event))
            })
            .collect()
    }

    /// Returns the most recent canonical transaction date for `account_id`, or
    /// `None` if the account has no transactions.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or date-parse failure.
    pub async fn latest_activity_date(
        &self,
        account_id: &AccountId,
    ) -> BcResult<Option<jiff::civil::Date>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT MAX(t.date) FROM transactions t \
             WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?)",
        )
        .bind(account_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row.and_then(|(d,)| d) {
            None => Ok(None),
            Some(s) => s
                .parse::<jiff::civil::Date>()
                .map(Some)
                .map_err(|e| BcError::BadData(format!("invalid date '{s}': {e}"))),
        }
    }
}

/// Inserts one posting projection row at an explicit position.
///
/// Shared by [`Service::create_in_tx`] and [`Service::add_postings_in_tx`] so
/// the column list cannot drift between the create and append paths.
///
/// # Arguments
///
/// * `db_tx` - An open SQLite transaction to write within.
/// * `transaction_id` - The owning transaction.
/// * `posting` - The posting to insert.
/// * `position` - The ordinal position within the transaction.
///
/// # Errors
///
/// Returns [`BcError`] on insert failure.
async fn insert_posting_row(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    transaction_id: &TransactionId,
    posting: &Posting,
    position: i64,
) -> BcResult<()> {
    let (cost_value, cost_commodity, cost_date, cost_label) = if let Some(cost) = posting.cost() {
        (
            Some(cost.total().value().to_string()),
            Some(cost.total().commodity().as_str().to_owned()),
            cost.date().map(|d| d.to_string()),
            cost.label().map(str::to_owned),
        )
    } else {
        (None, None, None, None)
    };

    sqlx::query(
        "INSERT INTO postings \
         (id, transaction_id, account_id, amount, commodity, position, \
          cost_total_value, cost_total_commodity, cost_date, cost_label, \
          spread_from, spread_until) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(posting.id().to_string()) //  1. id
    .bind(transaction_id.to_string()) //  2. transaction_id
    .bind(posting.account_id().to_string()) //  3. account_id
    .bind(posting.amount().map(|a| a.value().to_string())) //  4. amount
    .bind(posting.amount().map(|a| a.commodity().as_str().to_owned())) //  5. commodity
    .bind(position) //  6. position
    .bind(cost_value) //  7. cost_total_value
    .bind(cost_commodity) //  8. cost_total_commodity
    .bind(cost_date) //  9. cost_date
    .bind(cost_label) // 10. cost_label
    .bind(posting.spread_from().map(|d| d.to_string())) // 11. spread_from
    .bind(posting.spread_until().map(|d| d.to_string())) // 12. spread_until
    .execute(&mut **db_tx)
    .await?;

    crate::metadata::insert(
        db_tx,
        crate::metadata::Owner::Posting,
        &posting.id().to_string(),
        posting.metadata(),
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Cost;
    use bc_models::MetaEntry;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::diff_transaction;
    use super::*;
    use crate::events::Event;

    /// Builds a metadata key, for tests that know their literal is valid.
    fn key(name: &str) -> bc_models::MetaKey {
        bc_models::MetaKey::new(name).expect("key should be valid")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_spread_persists_and_loads(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let expense = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("expense");
        let asset = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("asset");

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 9, 1))
            .description("Gym membership")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(expense)
                    .amount(Amount::new(dec!(600.00), CommodityCode::new("AUD")))
                    .spread_from(date(2026, 9, 1))
                    .spread_until(date(2027, 3, 12))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(asset)
                    .amount(Amount::new(dec!(-600.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .build();

        let tx_id = svc.create(tx).await.expect("create tx").into_inner();
        let loaded = svc.find_by_id(&tx_id).await.expect("get tx");

        let gym_posting = loaded
            .postings()
            .iter()
            .find(|p| p.spread_from().is_some())
            .expect("gym posting");
        assert_eq!(gym_posting.spread_from(), Some(date(2026, 9, 1)));
        assert_eq!(gym_posting.spread_until(), Some(date(2027, 3, 12)));

        let asset_posting = loaded
            .postings()
            .iter()
            .find(|p| p.spread_from().is_none())
            .expect("asset posting");
        assert!(asset_posting.spread_until().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_and_clear_posting_spread(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let expense = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("expense");
        let asset = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("asset");

        let posting_id = PostingId::new();
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 9, 1))
            .description("Gym membership")
            .postings(vec![
                Posting::builder()
                    .id(posting_id.clone())
                    .account_id(expense)
                    .amount(Amount::new(dec!(600.00), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(asset)
                    .amount(Amount::new(dec!(-600.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .build();

        let tx_id = svc.create(tx).await.expect("create tx").into_inner();

        svc.set_posting_spread(&posting_id, date(2026, 9, 1), date(2027, 3, 12))
            .await
            .expect("set spread");

        let loaded = svc.find_by_id(&tx_id).await.expect("get after set");
        let posting = loaded
            .postings()
            .iter()
            .find(|row| row.id() == &posting_id)
            .expect("posting");
        assert_eq!(posting.spread_from(), Some(date(2026, 9, 1)));
        assert_eq!(posting.spread_until(), Some(date(2027, 3, 12)));

        svc.clear_posting_spread(&posting_id)
            .await
            .expect("clear spread");
        let loaded2 = svc.find_by_id(&tx_id).await.expect("get after clear");
        let posting2 = loaded2
            .postings()
            .iter()
            .find(|row| row.id() == &posting_id)
            .expect("posting");
        assert!(posting2.spread_from().is_none());
        assert!(posting2.spread_until().is_none());
    }

    fn make_balanced_transaction(acc_a: AccountId, acc_b: AccountId) -> Transaction {
        Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Test")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(100.00), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-100.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_balanced_transaction_succeeds(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income account should succeed");
        let acc_b = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Checking account should succeed");

        let svc = Service::new(pool.clone());
        let tx = make_balanced_transaction(acc_a, acc_b);
        let id = tx.id().clone();
        svc.create(tx)
            .await
            .expect("balanced transaction should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.postings().len(), 2);
        assert!(found.tag_ids().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reconcile_unbalanced_returns_bad_data(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let svc = Service::new(pool.clone());
        let tx_id = bc_models::TransactionId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(date(2026, 1, 15))
            .description("Unbalanced")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc)
                    .amount(Amount::new(dec!(50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(tx).await.expect("create");

        let err = svc
            .reconcile(&tx_id, Reconciliation::Reconciled)
            .await
            .expect_err("reconciling an unbalanced transaction must fail");
        assert!(
            matches!(err, BcError::BadData(_)),
            "expected BadData, got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reconcile_records_audit_event(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let acc_b = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = Service::new(pool.clone());
        let mut tx = make_balanced_transaction(acc_a, acc_b);
        tx = Transaction::builder()
            .id(tx.id().clone())
            .date(tx.date())
            .description(tx.description().to_owned())
            .postings(tx.postings().to_vec())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let tx_id = tx.id().clone();
        svc.create(tx).await.expect("create");

        svc.reconcile(&tx_id, Reconciliation::Reconciled)
            .await
            .expect("reconcile balanced transaction");

        let trail = svc.audit_trail(&tx_id).await.expect("audit trail");
        assert!(
            trail
                .iter()
                .any(|(_, e)| matches!(e, Event::TransactionReconciled { .. })),
            "audit trail must include a TransactionReconciled event"
        );

        let found = svc.find_by_id(&tx_id).await.expect("find");
        assert_eq!(found.reconciliation(), Reconciliation::Reconciled);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_unbalanced_transaction_succeeds(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let svc = Service::new(pool.clone());
        let tx_id = bc_models::TransactionId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(date(2026, 1, 15))
            .description("Unbalanced")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc)
                    .amount(Amount::new(dec!(50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(tx)
            .await
            .expect("one-sided transaction should now succeed");
        let found = svc.find_by_id(&tx_id).await.expect("find should succeed");
        assert!(
            !found.balanced(),
            "one-sided transaction should not be balanced"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn create_warns_but_persists_a_posting_outside_the_account_life(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let acc = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .opened_on(date(2020, 1, 1))
            .call()
            .await
            .expect("create account");

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2019, 5, 1))
            .description("Dated before the account opened")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc)
                    .amount(Amount::new(dec!(50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let warned = svc.create(tx).await.expect("create must succeed");

        assert_eq!(warned.warnings.len(), 1, "{:?}", warned.warnings);
        assert!(
            matches!(
                warned.warnings[0],
                crate::Warning::PostingBeforeAccountOpened { .. }
            ),
            "{:?}",
            warned.warnings
        );

        svc.find_by_id(&warned.value)
            .await
            .expect("the transaction must still have been written");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_inside_the_account_life_warns_nothing(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let acc = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .opened_on(date(2020, 1, 1))
            .call()
            .await
            .expect("create account");

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2022, 3, 3))
            .description("Dated inside the account life")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc)
                    .amount(Amount::new(dec!(50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let warned = svc.create(tx).await.expect("create must succeed");
        assert!(warned.warnings.is_empty(), "{:?}", warned.warnings);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_cost_round_trips(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Brokerage")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Brokerage account should succeed");
        let acc_b = acct_svc
            .create()
            .name("Cash")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Cash account should succeed");

        let cost = Cost::builder()
            .total(Amount::new(dec!(1500.00), CommodityCode::new("AUD")))
            .label("lot-1")
            .build();

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Buy shares")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(10), CommodityCode::new("AAPL")))
                    .cost(cost)
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-10), CommodityCode::new("AAPL")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();

        let id = tx.id().clone();
        svc.create(tx).await.expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        let first_posting = found
            .postings()
            .first()
            .expect("first posting should exist");
        let loaded_cost = first_posting.cost().expect("cost should be present");
        assert_eq!(loaded_cost.total().value(), dec!(1500.00));
        assert_eq!(loaded_cost.total().commodity().as_str(), "AUD");
        assert_eq!(loaded_cost.label(), Some("lot-1"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reverse_creates_linked_negated_transaction(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Income")
            .account_type(bc_models::AccountType::Income)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc a");
        let acc_b = acct_svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc b");

        let svc = Service::new(pool.clone());
        let tx = make_balanced_transaction(acc_a, acc_b);
        let original_id = tx.id().clone();
        svc.create(tx).await.expect("create");

        let reversal_id = svc.reverse(&original_id).await.expect("reverse");
        let reversal = svc.find_by_id(&reversal_id).await.expect("find reversal");

        let orig = svc.find_by_id(&original_id).await.expect("find original");
        let orig_sum: rust_decimal::Decimal = orig
            .postings()
            .iter()
            .map(|p| p.amount().expect("amount set in test").value())
            .sum();
        let rev_sum: rust_decimal::Decimal = reversal
            .postings()
            .iter()
            .map(|p| p.amount().expect("amount set in test").value())
            .sum();
        pretty_assertions::assert_eq!(orig_sum, rust_decimal::Decimal::ZERO);
        pretty_assertions::assert_eq!(rev_sum, rust_decimal::Decimal::ZERO);
        pretty_assertions::assert_eq!(reversal.postings().len(), orig.postings().len());

        // Amounts are negated.
        for (orig_p, rev_p) in orig.postings().iter().zip(reversal.postings().iter()) {
            let rev_negated = rev_p
                .amount()
                .expect("reversal amount set in test")
                .value()
                .checked_mul(rust_decimal::Decimal::NEGATIVE_ONE)
                .expect("negation should not overflow in test");
            pretty_assertions::assert_eq!(
                orig_p
                    .amount()
                    .expect("original amount set in test")
                    .value(),
                rev_negated,
                "reversal posting should negate original"
            );
        }

        // The reversal relationship lives in the event log, not a projection table.
        let reversed_events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE kind = 'TransactionReversed'")
                .fetch_one(&pool)
                .await
                .expect("count reversed events");
        pretty_assertions::assert_eq!(reversed_events, 1, "one reversal event recorded");

        // Description follows the expected pattern.
        pretty_assertions::assert_eq!(
            reversal.description(),
            format!("Reversal of {original_id}").as_str()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reverse_nonexistent_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::TransactionId::new();
        let result = svc.reverse(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_returns_all_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create B should succeed");

        let svc = Service::new(pool.clone());

        let tx1 = make_balanced_transaction(acc_a.clone(), acc_b.clone());
        let id1 = tx1.id().clone();
        svc.create(tx1).await.expect("create tx1 should succeed");

        let tx2 = make_balanced_transaction(acc_a, acc_b);
        let id2 = tx2.id().clone();
        svc.create(tx2).await.expect("create tx2 should succeed");

        let txns = svc.list().await.expect("list should succeed");
        assert_eq!(txns.len(), 2);
        let ids: Vec<_> = txns.iter().map(Transaction::id).collect();
        assert!(ids.contains(&&id1));
        assert!(ids.contains(&&id2));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_for_account_in_range_is_half_open(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create B should succeed");

        let svc = Service::new(pool.clone());

        for d in [
            date(2026, 5, 31),
            date(2026, 6, 1),
            date(2026, 6, 30),
            date(2026, 7, 1),
        ] {
            let tx = Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(d)
                .description("Test")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(acc_a.clone())
                        .amount(Amount::new(dec!(100.00), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(acc_b.clone())
                        .amount(Amount::new(dec!(-100.00), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build();
            svc.create(tx).await.expect("create tx should succeed");
        }

        let from = date(2026, 6, 1);
        let until = date(2026, 7, 1);
        let dates: Vec<Date> = svc
            .list_for_account_in_range(&acc_a, from, until)
            .await
            .expect("range query")
            .map(|t| t.date())
            .collect();

        assert_eq!(dates, vec![date(2026, 6, 30), date(2026, 6, 1)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn latest_activity_date_returns_max_and_none(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create B should succeed");

        let svc = Service::new(pool.clone());

        assert_eq!(svc.latest_activity_date(&acc_a).await.expect("query"), None);

        for d in [date(2026, 1, 5), date(2026, 6, 30), date(2026, 3, 1)] {
            let tx = Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(d)
                .description("Test")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(acc_a.clone())
                        .amount(Amount::new(dec!(100.00), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(acc_b.clone())
                        .amount(Amount::new(dec!(-100.00), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build();
            svc.create(tx).await.expect("create tx should succeed");
        }

        assert_eq!(
            svc.latest_activity_date(&acc_a).await.expect("query"),
            Some(date(2026, 6, 30))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_updates_projection(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account_svc = crate::AccountService::new(pool.clone());

        // Create two accounts so FK constraints pass.
        let checking_id = account_svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create checking");
        let expenses_id = account_svc
            .create()
            .name("Expenses")
            .account_type(bc_models::AccountType::Expense)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create expenses");

        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date("2026-01-01".parse::<jiff::civil::Date>().expect("date"))
            .description("Original description")
            .reconciliation(Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(checking_id.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(expenses_id.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .build();

        let id = svc
            .create(original.clone())
            .await
            .expect("create")
            .into_inner();

        let amended = Transaction::builder()
            .id(id.clone())
            .date("2026-01-15".parse::<jiff::civil::Date>().expect("date"))
            .description("Amended description")
            .reconciliation(Reconciliation::Reconciled)
            .postings(original.postings().to_vec())
            .created_at(*original.created_at())
            .build();

        svc.amend(amended).await.expect("amend should succeed");

        let loaded = svc.find_by_id(&id).await.expect("should still exist");
        assert_eq!(loaded.description(), "Amended description");
        assert_eq!(
            loaded.date(),
            "2026-01-15".parse::<jiff::civil::Date>().expect("date")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_nonexistent_transaction_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::TransactionId::new();

        let amended = Transaction::builder()
            .id(fake_id)
            .date("2026-01-15".parse::<jiff::civil::Date>().expect("date"))
            .description("Amended non-existent")
            .reconciliation(Reconciliation::Reconciled)
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .created_at(jiff::Timestamp::now())
            .build();

        let result = svc.amend(amended).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_tag_ids_round_trip(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create B should succeed");

        // Insert a tag directly (bypassing tag service for simplicity).
        let tag_id = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, 'groceries', ?)")
            .bind(tag_id.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert tag should succeed");

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Groceries")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(50), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-50), CommodityCode::new("AUD")))
                    .build(),
            ])
            .tag_ids(vec![tag_id.clone()])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();

        let id = tx.id().clone();
        svc.create(tx).await.expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.tag_ids(), &[tag_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_metadata_roundtrips(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let a = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("a");
        let b = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("b");
        let svc = Service::new(pool.clone());
        let base = make_balanced_transaction(a, b);
        let tx = Transaction::builder()
            .id(base.id().clone())
            .date(base.date())
            .description("raw")
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("note"),
                MetaValue::Text("my annotation".to_owned()),
            )]))
            .postings(base.postings().to_vec())
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        svc.create(tx).await.expect("create");
        let found = svc.find_by_id(&id).await.expect("find");
        assert_eq!(
            found.metadata().get_first_text(&key("note")),
            Some("my annotation")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn date_valued_metadata_roundtrips(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let a = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("a");
        let b = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("b");
        let svc = Service::new(pool.clone());
        let base = make_balanced_transaction(a, b);
        let tx = Transaction::builder()
            .id(base.id().clone())
            .date(base.date())
            .description("d")
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("cleared"),
                MetaValue::Date(date(2026, 1, 17)),
            )]))
            .postings(base.postings().to_vec())
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        svc.create(tx).await.expect("create");
        let found = svc.find_by_id(&id).await.expect("find");
        assert_eq!(
            found.metadata().get_first(&key("cleared")),
            Some(&MetaValue::Date(date(2026, 1, 17))),
            "a date-typed entry comes back typed, not as text"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_transaction_with_posting_tags(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income account should succeed");
        let acc_b = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Checking account should succeed");

        let svc = Service::new(pool.clone());

        let posting_a = Posting::builder()
            .id(PostingId::new())
            .account_id(acc_a.clone())
            .amount(Amount::new(dec!(75.00), CommodityCode::new("AUD")))
            .build();
        let posting_b = Posting::builder()
            .id(PostingId::new())
            .account_id(acc_b.clone())
            .amount(Amount::new(dec!(-75.00), CommodityCode::new("AUD")))
            .build();
        let posting_a_id = posting_a.id().clone();

        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Original description")
            .postings(vec![posting_a, posting_b])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();

        let tx_id = tx.id().clone();
        svc.create(tx).await.expect("create should succeed");

        // Manually insert a tag and a posting_tag row to exercise the FK constraint path.
        let tag_id = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind(tag_id.to_string())
            .bind("expenses:food")
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert tag should succeed");
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
            .bind(posting_a_id.to_string())
            .bind(tag_id.to_string())
            .execute(&pool)
            .await
            .expect("insert posting_tag should succeed");

        // Amend: FK violation would occur here if posting_tags is not deleted first.
        let updated = Transaction::builder()
            .id(tx_id.clone())
            .date(date(2026, 3, 1))
            .description("Amended description")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(75.00), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-75.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();

        svc.amend(updated)
            .await
            .expect("amend should succeed despite posting_tags FK");

        let found = svc.find_by_id(&tx_id).await.expect("find should succeed");
        assert_eq!(found.description(), "Amended description");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_empty_postings(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Empty")
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let result = svc.create(tx).await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "empty posting list should be rejected"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_two_elided_postings(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Two elided")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(AccountId::new())
                    .maybe_amount(None)
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(AccountId::new())
                    .maybe_amount(None)
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let result = svc.create(tx).await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "two elided postings should be rejected"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_lone_elided_posting(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Lone elided")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(AccountId::new())
                    .maybe_amount(None)
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let result = svc.create(tx).await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "lone elided posting should be rejected"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_accepts_concrete_with_elided(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");
        let svc = Service::new(pool.clone());
        let tx_id = bc_models::TransactionId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(date(2026, 1, 15))
            .description("Concrete plus elided")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(-50.00), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .maybe_amount(None)
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(tx)
            .await
            .expect("concrete + elided should be accepted");
        let found = svc.find_by_id(&tx_id).await.expect("find should succeed");
        assert_eq!(found.postings().len(), 2);
        assert!(found.balanced(), "concrete + elided should balance");
    }

    /// Creates a bare deposit account with the given name, for tests that only
    /// need an account to exist and don't care about its type.
    async fn test_account(pool: &sqlx::SqlitePool, name: &str) -> AccountId {
        crate::account::Service::new(pool.clone())
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_postings_in_tx_appends_legs_after_existing_ones(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let bank = test_account(&pool, "Bank").await;
        let food = test_account(&pool, "Food").await;

        let tx_id = TransactionId::new();
        let first = Posting::builder()
            .id(PostingId::new())
            .account_id(bank.clone())
            .amount(Amount::new(
                Decimal::from(-50_i64),
                CommodityCode::new("AUD"),
            ))
            .build();
        svc.create(
            Transaction::builder()
                .id(tx_id.clone())
                .date(date(2025, 6, 27))
                .description("SPLIT")
                .postings(vec![first])
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("create");

        let added = Posting::builder()
            .id(PostingId::new())
            .account_id(food.clone())
            .amount(Amount::new(
                Decimal::from(50_i64),
                CommodityCode::new("AUD"),
            ))
            .build();

        let mut db_tx = pool.begin().await.expect("begin");
        svc.add_postings_in_tx(&mut db_tx, &tx_id, &[added])
            .await
            .expect("add postings");
        db_tx.commit().await.expect("commit");

        let loaded = svc.find_by_id(&tx_id).await.expect("find");
        assert_eq!(loaded.postings().len(), 2, "the leg was appended");
        assert!(
            loaded.balanced(),
            "supplying the counter-leg balances the transaction"
        );

        let positions: Vec<i64> = sqlx::query_scalar(
            "SELECT position FROM postings WHERE transaction_id = ? ORDER BY position",
        )
        .bind(tx_id.to_string())
        .fetch_all(&pool)
        .await
        .expect("positions");
        assert_eq!(
            positions,
            vec![0, 1],
            "the appended leg takes the next position, never colliding"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_postings_in_tx_rejects_an_unknown_transaction(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let bank = test_account(&pool, "Bank").await;
        let orphan = Posting::builder()
            .id(PostingId::new())
            .account_id(bank)
            .amount(Amount::new(Decimal::from(1_i64), CommodityCode::new("AUD")))
            .build();

        let mut db_tx = pool.begin().await.expect("begin");
        let result = svc
            .add_postings_in_tx(&mut db_tx, &TransactionId::new(), &[orphan])
            .await;
        assert!(
            result.is_err(),
            "attaching to a nonexistent transaction must fail loudly"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reconcile_rejects_unbalanced(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let svc = Service::new(pool.clone());
        let tx_id = bc_models::TransactionId::new();
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(date(2026, 1, 15))
            .description("One sided")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc)
                    .amount(Amount::new(dec!(-50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(tx).await.expect("create should succeed");
        let result = svc.reconcile(&tx_id, Reconciliation::Reconciled).await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "reconcile on unbalanced tx should fail"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reconcile_accepts_balanced(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");
        let svc = Service::new(pool.clone());
        let tx = make_balanced_transaction(acc_a, acc_b);
        let id = tx.id().clone();
        svc.create(tx).await.expect("create balanced tx");
        svc.reconcile(&id, Reconciliation::Reconciled)
            .await
            .expect("reconcile balanced tx should succeed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_preserves_metadata(pool: sqlx::SqlitePool) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");
        let svc = Service::new(pool.clone());

        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 10))
            .description("Original description")
            .metadata(Metadata::new(vec![
                MetaEntry::new(key("note"), MetaValue::Text("keep this note".to_owned())),
                MetaEntry::new(key("cleared"), MetaValue::Date(date(2026, 1, 12))),
            ]))
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(-50), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(50), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let id = svc
            .create(original.clone())
            .await
            .expect("create")
            .into_inner();

        let updated = Transaction::builder()
            .id(id.clone())
            .date(date(2026, 1, 10))
            .description("Amended description")
            .metadata(original.metadata().clone())
            .postings(original.postings().to_vec())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(*original.created_at())
            .build();

        svc.amend(updated).await.expect("amend should succeed");

        let found = svc.find_by_id(&id).await.expect("find after amend");
        assert_eq!(found.description(), "Amended description");
        assert_eq!(
            found.metadata().get_first_text(&key("note")),
            Some("keep this note"),
            "metadata must survive amend"
        );
        assert_eq!(
            found.metadata().get_first(&key("cleared")),
            Some(&MetaValue::Date(date(2026, 1, 12))),
            "and keep its types"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_preserves_import_provenance_of_surviving_legs(pool: sqlx::SqlitePool) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");
        let svc = Service::new(pool.clone());

        let posting_a = PostingId::new();
        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 10))
            .description("COFFEE")
            .postings(vec![
                Posting::builder()
                    .id(posting_a.clone())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(-5), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(5), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let id = svc
            .create(original.clone())
            .await
            .expect("create")
            .into_inner();

        let source_svc = crate::SourceService::new(pool.clone());
        let batch_svc = crate::ImportBatchService::new(pool.clone());
        let batch_id = batch_svc.open(None, "csv").await.expect("open batch");
        let source = bc_models::SourceRef::builder()
            .id(bc_models::SourceRefId::new())
            .transaction_id(id.clone())
            .posting_id(Some(posting_a.clone()))
            .account_id(acc_a.clone())
            .date(date(2026, 1, 10))
            .narration("COFFEE")
            .amount(Some(Amount::new(dec!(-5), CommodityCode::new("AUD"))))
            .occurrence(0)
            .import_batch_id(Some(batch_id.clone()))
            .owns_posting(true)
            .created_at(Timestamp::now())
            .build();
        source_svc.attach(&source).await.expect("attach source");

        // Edit the transaction's description only; both legs (and their posting
        // ids) survive the edit unchanged.
        let updated = Transaction::builder()
            .id(original.id().clone())
            .date(original.date())
            .description("COFFEE (edited)")
            .postings(original.postings().to_vec())
            .reconciliation(original.reconciliation())
            .created_at(*original.created_at())
            .build();
        svc.edit(updated).await.expect("edit should succeed");

        let listed = source_svc
            .list_for_transaction(&id)
            .await
            .expect("list sources after edit");
        assert_eq!(
            listed.len(),
            1,
            "editing a transaction must not destroy its import provenance"
        );
        let restored = listed.first().expect("one source ref");
        assert_eq!(restored.posting_id(), Some(&posting_a));
        assert_eq!(
            restored.import_batch_id(),
            Some(&batch_id),
            "the snapshot/restore path must carry import_batch_id, not just the \
             other columns"
        );
    }

    /// An imported-looking two-leg transaction with provenance on its first leg.
    ///
    /// Returns the stored transaction, the posting id of the leg carrying
    /// provenance, the two accounts, and the batch that "imported" it.
    async fn imported_two_legger(
        pool: &sqlx::SqlitePool,
    ) -> (
        Transaction,
        PostingId,
        AccountId,
        AccountId,
        bc_models::ImportBatchId,
    ) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");

        let posting_a = PostingId::new();
        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 10))
            .description("COFFEE")
            .postings(vec![
                Posting::builder()
                    .id(posting_a.clone())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(-5), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(5), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        Service::new(pool.clone())
            .create(original.clone())
            .await
            .expect("create");

        let batch_id = crate::ImportBatchService::new(pool.clone())
            .open(None, "csv")
            .await
            .expect("open batch");
        let source = bc_models::SourceRef::builder()
            .id(bc_models::SourceRefId::new())
            .transaction_id(original.id().clone())
            .posting_id(Some(posting_a.clone()))
            .account_id(acc_a.clone())
            .date(date(2026, 1, 10))
            .narration("COFFEE")
            .amount(Some(Amount::new(dec!(-5), CommodityCode::new("AUD"))))
            .occurrence(0)
            .import_batch_id(Some(batch_id.clone()))
            .owns_posting(true)
            .created_at(Timestamp::now())
            .build();
        crate::SourceService::new(pool.clone())
            .attach(&source)
            .await
            .expect("attach source");

        (original, posting_a, acc_a, acc_b, batch_id)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_a_leg_leaves_a_tombstoned_source_ref(pool: sqlx::SqlitePool) {
        let (original, _posting_a, acc_a, acc_b, batch_id) = imported_two_legger(&pool).await;

        // Drop the leg that carries provenance, keeping only its counter-leg.
        let kept: Vec<Posting> = original
            .postings()
            .iter()
            .filter(|posting| *posting.account_id() == acc_b)
            .cloned()
            .collect();
        let updated = Transaction::builder()
            .id(original.id().clone())
            .date(original.date())
            .description(original.description())
            .postings(kept)
            .reconciliation(original.reconciliation())
            .created_at(*original.created_at())
            .build();
        Service::new(pool.clone())
            .edit(updated)
            .await
            .expect("edit should succeed");

        let row: (
            Option<String>,
            String,
            String,
            Option<String>,
            i64,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT posting_id, account_id, narration, amount, occurrence, import_batch_id \
                 FROM transaction_sources WHERE transaction_id = ?",
        )
        .bind(original.id().to_string())
        .fetch_one(&pool)
        .await
        .expect("the reference survives the deletion of its leg");

        assert_eq!(
            row.0, None,
            "the reference is tombstoned, not deleted: it records a leg the \
             document contained and the user removed"
        );
        assert_eq!(row.1, acc_a.to_string(), "every other column is intact");
        assert_eq!(row.2, "COFFEE");
        assert_eq!(row.3, Some("-5".to_owned()));
        assert_eq!(row.4, 0_i64);
        assert_eq!(row.5, Some(batch_id.to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn modifying_a_leg_keeps_its_source_ref_linked(pool: sqlx::SqlitePool) {
        let (original, posting_a, acc_a, _acc_b, _batch_id) = imported_two_legger(&pool).await;

        // Change the leg's amount while keeping its posting id — a modification,
        // not a deletion.
        let postings: Vec<Posting> = original
            .postings()
            .iter()
            .map(|posting| {
                if posting.id() == &posting_a {
                    Posting::builder()
                        .id(posting_a.clone())
                        .account_id(acc_a.clone())
                        .amount(Amount::new(dec!(-7), CommodityCode::new("AUD")))
                        .build()
                } else {
                    posting.clone()
                }
            })
            .collect();
        let updated = Transaction::builder()
            .id(original.id().clone())
            .date(original.date())
            .description(original.description())
            .postings(postings)
            .reconciliation(original.reconciliation())
            .created_at(*original.created_at())
            .build();
        Service::new(pool.clone())
            .edit(updated)
            .await
            .expect("edit should succeed");

        let listed = crate::SourceService::new(pool.clone())
            .list_for_transaction(original.id())
            .await
            .expect("list sources after edit");
        let restored = listed.first().expect("one source ref");
        assert_eq!(
            restored.posting_id(),
            Some(&posting_a),
            "modifying a leg leaves its reference pointing at it"
        );
        assert_eq!(
            restored.amount(),
            Some(&Amount::new(dec!(-5), CommodityCode::new("AUD"))),
            "the reference records what the source document said, not the edit"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_recategorised_leg_keeps_its_original_account_on_its_ref(pool: sqlx::SqlitePool) {
        let (original, posting_a, acc_a, acc_b, _batch_id) = imported_two_legger(&pool).await;

        // Move the leg onto the other account, keeping its posting id.
        let postings: Vec<Posting> = original
            .postings()
            .iter()
            .map(|posting| {
                if posting.id() == &posting_a {
                    Posting::builder()
                        .id(posting_a.clone())
                        .account_id(acc_b.clone())
                        .amount(Amount::new(dec!(-5), CommodityCode::new("AUD")))
                        .build()
                } else {
                    posting.clone()
                }
            })
            .collect();
        let updated = Transaction::builder()
            .id(original.id().clone())
            .date(original.date())
            .description(original.description())
            .postings(postings)
            .reconciliation(original.reconciliation())
            .created_at(*original.created_at())
            .build();
        Service::new(pool.clone())
            .edit(updated)
            .await
            .expect("edit should succeed");

        let listed = crate::SourceService::new(pool.clone())
            .list_for_transaction(original.id())
            .await
            .expect("list sources after edit");
        let restored = listed.first().expect("one source ref");
        assert_eq!(
            restored.posting_id(),
            Some(&posting_a),
            "the leg survives, so its reference stays linked"
        );
        assert_eq!(
            restored.account_id(),
            &acc_a,
            "the reference names the account whose statement produced the row, \
             so recategorising the posting does not rewrite it"
        );
    }

    // MARK: diff_transaction tests

    fn sample_tx() -> Transaction {
        let p1 = Posting::builder()
            .id(PostingId::new())
            .account_id(AccountId::new())
            .maybe_amount(Some(Amount::new(
                rust_decimal::Decimal::new(-1000, 2),
                CommodityCode::new("AUD"),
            )))
            .tag_ids(Vec::new())
            .build();
        let p2 = Posting::builder()
            .id(PostingId::new())
            .account_id(AccountId::new())
            .maybe_amount(Some(Amount::new(
                rust_decimal::Decimal::new(1000, 2),
                CommodityCode::new("AUD"),
            )))
            .tag_ids(Vec::new())
            .build();
        Transaction::builder()
            .id(TransactionId::new())
            .date("2026-04-30".parse::<Date>().expect("valid date"))
            .description("desc".to_owned())
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]))
            .postings(vec![p1, p2])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    trait TxTestExt {
        fn with_metadata(self, metadata: Metadata) -> Self;
        fn with_first_posting_metadata(self, metadata: Metadata) -> Self;
        fn recategorise_first(self, account: AccountId) -> Self;
        fn push_leg(self) -> Self;
        fn recategorise_posting(self, target: &PostingId, account: AccountId) -> Self;
    }

    impl TxTestExt for Transaction {
        fn with_metadata(self, metadata: Metadata) -> Self {
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .description(self.description().to_owned())
                .metadata(metadata)
                .postings(self.postings().to_vec())
                .reconciliation(self.reconciliation())
                .tag_ids(self.tag_ids().to_vec())
                .created_at(Timestamp::now())
                .build()
        }

        fn with_first_posting_metadata(self, metadata: Metadata) -> Self {
            let mut carried = Some(metadata);
            let postings = self
                .postings()
                .iter()
                .map(|p| {
                    Posting::builder()
                        .id(p.id().clone())
                        .account_id(p.account_id().clone())
                        .maybe_amount(p.amount().cloned())
                        .metadata(carried.take().unwrap_or_else(|| p.metadata().clone()))
                        .tag_ids(p.tag_ids().to_vec())
                        .build()
                })
                .collect();
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .description(self.description().to_owned())
                .metadata(self.metadata().clone())
                .postings(postings)
                .reconciliation(self.reconciliation())
                .tag_ids(self.tag_ids().to_vec())
                .created_at(*self.created_at())
                .build()
        }

        #[expect(
            clippy::indexing_slicing,
            reason = "test helper: sample_tx always has at least two postings"
        )]
        fn recategorise_first(self, account: AccountId) -> Self {
            let mut postings = self.postings().to_vec();
            let first = &postings[0];
            postings[0] = Posting::builder()
                .id(first.id().clone())
                .account_id(account)
                .maybe_amount(first.amount().cloned())
                .tag_ids(first.tag_ids().to_vec())
                .build();
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .description(self.description().to_owned())
                .metadata(self.metadata().clone())
                .postings(postings)
                .reconciliation(self.reconciliation())
                .created_at(Timestamp::now())
                .build()
        }

        fn push_leg(self) -> Self {
            let mut postings = self.postings().to_vec();
            postings.push(
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(AccountId::new())
                    .maybe_amount(Some(Amount::new(
                        rust_decimal::Decimal::new(500, 2),
                        CommodityCode::new("AUD"),
                    )))
                    .tag_ids(Vec::new())
                    .build(),
            );
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .description(self.description().to_owned())
                .metadata(self.metadata().clone())
                .postings(postings)
                .reconciliation(self.reconciliation())
                .created_at(Timestamp::now())
                .build()
        }

        fn recategorise_posting(self, target: &PostingId, account: AccountId) -> Self {
            let postings = self
                .postings()
                .iter()
                .map(|p| {
                    if p.id() == target {
                        Posting::builder()
                            .id(p.id().clone())
                            .account_id(account.clone())
                            .maybe_amount(p.amount().cloned())
                            .metadata(p.metadata().clone())
                            .tag_ids(p.tag_ids().to_vec())
                            .build()
                    } else {
                        p.clone()
                    }
                })
                .collect::<Vec<_>>();
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .description(self.description().to_owned())
                .metadata(self.metadata().clone())
                .postings(postings)
                .reconciliation(self.reconciliation())
                .created_at(Timestamp::now())
                .build()
        }
    }

    /// Serialises a [`Reconciliation`] value to the canonical DB string.
    ///
    /// Thin wrapper around [`crate::db::to_db_str`] for test helpers that need
    /// a plain `String` without propagating `BcResult`.
    fn to_db_str_test(val: Reconciliation) -> String {
        crate::db::to_db_str(val).expect("Reconciliation serialises cleanly")
    }

    /// Seeds two accounts and a balanced two-posting transaction.
    ///
    /// Returns `(tx_id, first_posting_id, second_account_id)` so callers can
    /// recategorise the first posting into the second account.
    async fn seed_editable_tx(pool: &sqlx::SqlitePool) -> (TransactionId, PostingId, AccountId) {
        let account_svc = crate::AccountService::new(pool.clone());
        let checking_id = account_svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create checking account");
        let expenses_id = account_svc
            .create()
            .name("Expenses")
            .account_type(bc_models::AccountType::Expense)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create expenses account");

        let posting_id = PostingId::new();
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date("2026-05-01".parse::<Date>().expect("valid date"))
            .description("Seed transaction")
            .postings(vec![
                Posting::builder()
                    .id(posting_id.clone())
                    .account_id(checking_id.clone())
                    .amount(Amount::new(
                        rust_decimal::Decimal::new(-10000, 2),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(expenses_id.clone())
                    .amount(Amount::new(
                        rust_decimal::Decimal::new(10000, 2),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let tx_id = svc
            .create(tx)
            .await
            .expect("seed transaction created")
            .into_inner();
        (tx_id, posting_id, expenses_id)
    }

    // MARK: Service::edit tests

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_emits_decomposed_events(pool: sqlx::SqlitePool) {
        let service = Service::new(pool.clone());
        let (tx_id, posting_id, new_account_id) = seed_editable_tx(&pool).await;

        let current = service.find_by_id(&tx_id).await.expect("load tx");
        let updated = current
            .clone()
            .with_metadata(Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Other Grocer".to_owned()),
            )]))
            .recategorise_posting(&posting_id, new_account_id.clone());
        service.edit(updated).await.expect("edit ok");

        let kinds: Vec<String> =
            sqlx::query_scalar("SELECT kind FROM events WHERE aggregate_id = ? ORDER BY rowid ASC")
                .bind(tx_id.to_string())
                .fetch_all(&pool)
                .await
                .expect("query events");
        assert!(kinds.contains(&"PostingRecategorised".to_owned()));

        let reloaded = service.find_by_id(&tx_id).await.expect("reload");
        assert_eq!(
            reloaded.metadata().get_first_text(&key("payee")),
            Some("Other Grocer")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_preserves_cost(pool: sqlx::SqlitePool) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Brokerage")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create brokerage");
        let acc_b = acct_svc
            .create()
            .name("Cash")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create cash");

        let svc = Service::new(pool.clone());

        let cost = Cost::builder()
            .total(Amount::new(dec!(1500.00), CommodityCode::new("AUD")))
            .label("lot-1")
            .build();
        let posting_with_cost_id = PostingId::new();
        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Buy shares")
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("cleared"),
                MetaValue::Date(date(2026, 3, 3)),
            )]))
            .postings(vec![
                Posting::builder()
                    .id(posting_with_cost_id.clone())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(10), CommodityCode::new("AAPL")))
                    .cost(cost)
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(-10), CommodityCode::new("AAPL")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let tx_id = svc
            .create(original.clone())
            .await
            .expect("create")
            .into_inner();

        // Edit: only change the description — metadata echoed from current,
        // posting cost must survive.
        let current = svc.find_by_id(&tx_id).await.expect("load current");
        let edited = Transaction::builder()
            .id(tx_id.clone())
            .date(current.date())
            .description("Buy more shares")
            .metadata(current.metadata().clone())
            .postings(current.postings().to_vec())
            .reconciliation(current.reconciliation())
            .tag_ids(current.tag_ids().to_vec())
            .created_at(*current.created_at())
            .build();

        svc.edit(edited).await.expect("edit ok");

        let reloaded = svc.find_by_id(&tx_id).await.expect("reload");
        let cost_posting = reloaded
            .postings()
            .iter()
            .find(|p| p.id() == &posting_with_cost_id)
            .expect("posting with cost must still exist");
        let saved_cost = cost_posting.cost().expect("cost must survive edit");
        assert_eq!(saved_cost.total().value(), dec!(1500.00));
        assert_eq!(saved_cost.label(), Some("lot-1"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_can_change_metadata(pool: sqlx::SqlitePool) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Brokerage")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create brokerage");
        let acc_b = acct_svc
            .create()
            .name("Cash")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create cash");

        let svc = Service::new(pool.clone());

        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Buy shares")
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("cleared"),
                MetaValue::Date(date(2026, 3, 3)),
            )]))
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(10), CommodityCode::new("AAPL")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(-10), CommodityCode::new("AAPL")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let tx_id = svc.create(original).await.expect("create").into_inner();

        let current = svc.find_by_id(&tx_id).await.expect("load");
        let edited = Transaction::builder()
            .id(tx_id.clone())
            .date(current.date())
            .description(current.description().to_owned())
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("effective"),
                MetaValue::Date(date(2026, 3, 10)),
            )]))
            .postings(current.postings().to_vec())
            .reconciliation(current.reconciliation())
            .tag_ids(current.tag_ids().to_vec())
            .created_at(*current.created_at())
            .build();
        svc.edit(edited).await.expect("edit ok");

        let reloaded = svc.find_by_id(&tx_id).await.expect("reload");
        assert_eq!(
            reloaded.metadata().len(),
            1,
            "the edit replaced, not appended"
        );
        assert_eq!(
            reloaded.metadata().get_first(&key("effective")),
            Some(&MetaValue::Date(date(2026, 3, 10)))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_allows_reconciled_transactions(pool: sqlx::SqlitePool) {
        let service = Service::new(pool.clone());
        let (tx_id, _posting_id, _acct) = seed_editable_tx(&pool).await;
        sqlx::query("UPDATE transactions SET reconciliation = ? WHERE id = ?")
            .bind(to_db_str_test(Reconciliation::Reconciled))
            .bind(tx_id.to_string())
            .execute(&pool)
            .await
            .expect("mark reconciled");

        let current = service.find_by_id(&tx_id).await.expect("load");
        let updated = current
            .clone()
            .with_metadata(Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Other Grocer".to_owned()),
            )]));
        service.edit(updated).await.expect("reconciled edit ok");
    }

    #[test]
    fn diff_no_changes_is_empty() {
        let tx = sample_tx();
        assert!(
            diff_transaction(&tx, &tx).is_empty(),
            "identical inputs must produce no events"
        );
    }

    /// Builds a metadata list from `(key, text)` pairs.
    fn text_meta(pairs: &[(&str, &str)]) -> Metadata {
        Metadata::new(
            pairs
                .iter()
                .map(|&(name, value)| MetaEntry::new(key(name), MetaValue::Text(value.to_owned())))
                .collect(),
        )
    }

    #[test]
    fn diff_emits_one_event_for_the_whole_metadata_list() {
        let current = sample_tx().with_metadata(text_meta(&[
            ("payee", "Generic Grocer"),
            ("note", "weekly shop"),
        ]));
        let updated = current.clone().with_metadata(text_meta(&[
            ("payee", "Other Grocer"),
            ("note", "weekly shop"),
        ]));

        assert_eq!(
            diff_transaction(&current, &updated),
            vec![Event::TransactionMetadataChanged {
                id: current.id().clone(),
                before: text_meta(&[("payee", "Generic Grocer"), ("note", "weekly shop")]),
                after: text_meta(&[("payee", "Other Grocer"), ("note", "weekly shop")]),
            }],
            "one event carries the owner's whole list, touched keys and untouched alike"
        );
    }

    #[test]
    fn reordering_two_different_keys_is_recorded() {
        let current = sample_tx().with_metadata(text_meta(&[
            ("payee", "Generic Grocer"),
            ("note", "weekly shop"),
        ]));
        let updated = current.clone().with_metadata(text_meta(&[
            ("note", "weekly shop"),
            ("payee", "Generic Grocer"),
        ]));

        assert_eq!(
            diff_transaction(&current, &updated),
            vec![Event::TransactionMetadataChanged {
                id: current.id().clone(),
                before: text_meta(&[("payee", "Generic Grocer"), ("note", "weekly shop")]),
                after: text_meta(&[("note", "weekly shop"), ("payee", "Generic Grocer")]),
            }],
            "position is the display order and the editor lets a user set it, \
             so a reorder across keys is an edit the log has to hold, and the \
             payload has to carry both orderings for a replay to reproduce it"
        );
    }

    #[test]
    fn diff_emits_a_posting_metadata_event() {
        let current =
            sample_tx().with_first_posting_metadata(text_meta(&[("note", "new medication")]));
        let updated = current
            .clone()
            .with_first_posting_metadata(text_meta(&[("note", "repeat script")]));

        assert_eq!(
            diff_transaction(&current, &updated)
                .iter()
                .filter(|e| matches!(**e, Event::PostingMetadataChanged { .. }))
                .count(),
            1,
            "one leg changed, so one posting event"
        );
    }

    #[test]
    fn diff_emits_nothing_when_metadata_is_unchanged() {
        let tx = sample_tx().with_metadata(text_meta(&[("payee", "Generic Grocer")]));
        assert_eq!(diff_transaction(&tx, &tx), vec![]);
    }

    #[test]
    fn the_mismatched_flag_alone_is_not_an_edit() {
        let flagged = sample_tx().with_metadata(Metadata::new(vec![MetaEntry::mismatch(
            key("invoice"),
            "not a number",
        )]));
        let rebuilt = flagged
            .clone()
            .with_metadata(text_meta(&[("invoice", "not a number")]));

        assert_eq!(
            diff_transaction(&flagged, &rebuilt),
            vec![],
            "the write path derives `mismatched` and ignores what the entry \
             claims, so a caller that rebuilds an entry without the flag has \
             edited nothing and must not land an event"
        );
    }

    #[test]
    fn diff_detects_recategorise_and_added_leg() {
        let current = sample_tx();
        let new_account = AccountId::new();
        let updated = current
            .clone()
            .recategorise_first(new_account.clone())
            .push_leg();
        let events = diff_transaction(&current, &updated);
        assert!(events.iter().any(|e| matches!(
            e, Event::PostingRecategorised { to_account, .. } if *to_account == new_account
        )));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PostingAdded { .. }))
        );
    }

    /// Persists a balanced two-leg transaction carrying `metadata`, and returns
    /// the service and the stored ID.
    async fn seeded_transaction(
        pool: &sqlx::SqlitePool,
        metadata: Metadata,
    ) -> (Service, TransactionId) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account A");
        let acc_b = acct_svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account B");
        let svc = Service::new(pool.clone());

        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 2, 1))
            .description("Groceries")
            .metadata(metadata)
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a)
                    .amount(Amount::new(dec!(-30), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(30), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let id = svc.create(original).await.expect("create").into_inner();
        (svc, id)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_updates_metadata(pool: sqlx::SqlitePool) {
        let (svc, id) = seeded_transaction(&pool, text_meta(&[("note", "old note")])).await;
        let original = svc.find_by_id(&id).await.expect("load");

        let updated = Transaction::builder()
            .id(id.clone())
            .date(date(2026, 2, 1))
            .description("Groceries")
            .metadata(text_meta(&[("note", "new note")]))
            .postings(original.postings().to_vec())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(*original.created_at())
            .build();

        svc.amend(updated).await.expect("amend should succeed");

        let found = svc.find_by_id(&id).await.expect("find after amend");
        assert_eq!(
            found.metadata().get_first_text(&key("note")),
            Some("new note"),
            "amended metadata must persist"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_records_a_transaction_metadata_event(pool: sqlx::SqlitePool) {
        let (svc, id) = seeded_transaction(&pool, text_meta(&[("note", "old note")])).await;
        let original = svc.find_by_id(&id).await.expect("load");

        svc.amend(original.with_metadata(text_meta(&[("note", "new note")])))
            .await
            .expect("amend should succeed");

        assert!(
            event_kinds(&pool, &id)
                .await
                .contains(&"TransactionMetadataChanged".to_owned()),
            "amend persists metadata through the projection, so it has to record it too"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_that_leaves_metadata_alone_records_no_metadata_event(pool: sqlx::SqlitePool) {
        let (svc, id) = seeded_transaction(&pool, text_meta(&[("note", "old note")])).await;
        let original = svc.find_by_id(&id).await.expect("load");

        let updated = Transaction::builder()
            .id(id.clone())
            .date(original.date())
            .description("Groceries, amended")
            .metadata(original.metadata().clone())
            .postings(original.postings().to_vec())
            .reconciliation(original.reconciliation())
            .created_at(*original.created_at())
            .build();
        svc.amend(updated).await.expect("amend should succeed");

        let kinds = event_kinds(&pool, &id).await;
        assert!(
            kinds.contains(&"TransactionAmended".to_owned()),
            "the amend itself is still recorded, got {kinds:?}"
        );
        assert!(
            !kinds.contains(&"TransactionMetadataChanged".to_owned()),
            "an amend that carries the stored list back unchanged changed nothing, got {kinds:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn edit_records_a_transaction_metadata_event(pool: sqlx::SqlitePool) {
        let (svc, id) = seeded_transaction(&pool, text_meta(&[("note", "old note")])).await;

        let current = svc.find_by_id(&id).await.expect("load");
        svc.edit(current.with_metadata(text_meta(&[("note", "new note")])))
            .await
            .expect("edit");

        let kinds = event_kinds(&pool, &id).await;
        assert!(
            kinds.contains(&"TransactionMetadataChanged".to_owned()),
            "editing metadata through the service reaches the log, got {kinds:?}"
        );
    }

    /// Reads the kinds of every event logged against `id`, oldest first.
    async fn event_kinds(pool: &sqlx::SqlitePool, id: &TransactionId) -> Vec<String> {
        sqlx::query_scalar("SELECT kind FROM events WHERE aggregate_id = ? ORDER BY rowid ASC")
            .bind(id.to_string())
            .fetch_all(pool)
            .await
            .expect("kinds")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_posting_metadata_change_reaches_the_audit_trail(pool: sqlx::SqlitePool) {
        let (svc, id) = seeded_transaction(&pool, Metadata::default()).await;

        let current = svc.find_by_id(&id).await.expect("load");
        svc.edit(current.with_first_posting_metadata(text_meta(&[("note", "new medication")])))
            .await
            .expect("edit");

        let trail = svc.audit_trail(&id).await.expect("audit trail");
        assert!(
            trail
                .iter()
                .any(|(_ts, e)| matches!(*e, Event::PostingMetadataChanged { .. })),
            "audit_trail queries by transaction aggregate id, so a posting \
             metadata event must carry the transaction it belongs to or it \
             disappears from the trail entirely"
        );
    }

    // MARK: metadata round-trip tests

    /// Builds a one-leg transaction carrying `metadata`, with a second bare leg
    /// so the posting set is structurally valid.
    fn tx_with_metadata(
        id: &TransactionId,
        posting_id: &PostingId,
        account: &AccountId,
        metadata: Metadata,
        posting_metadata: Metadata,
    ) -> Transaction {
        Transaction::builder()
            .id(id.clone())
            .date(date(2026, 1, 15))
            .description("Groceries")
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .metadata(metadata)
            .postings(vec![
                Posting::builder()
                    .id(posting_id.clone())
                    .account_id(account.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .metadata(posting_metadata)
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(account.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_metadata_round_trips(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account = test_account(&pool, "Checking").await;
        let tx_id = TransactionId::new();
        let posting_id = PostingId::new();
        svc.create(tx_with_metadata(
            &tx_id,
            &posting_id,
            &account,
            Metadata::default(),
            Metadata::new(vec![MetaEntry::new(
                key("note"),
                MetaValue::Text("doctor's appointment".to_owned()),
            )]),
        ))
        .await
        .expect("create");

        let found = svc.find_by_id(&tx_id).await.expect("find");
        let leg = found
            .postings()
            .iter()
            .find(|p| p.id() == &posting_id)
            .expect("the annotated leg");
        assert_eq!(
            leg.metadata().get_first_text(&key("note")),
            Some("doctor's appointment")
        );
        assert!(
            found
                .postings()
                .iter()
                .any(|p| p.id() != &posting_id && p.metadata().is_empty()),
            "the other leg carries no metadata"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repeated_metadata_keys_survive_a_round_trip(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account = test_account(&pool, "Checking").await;
        let tx_id = TransactionId::new();
        svc.create(tx_with_metadata(
            &tx_id,
            &PostingId::new(),
            &account,
            Metadata::new(vec![
                MetaEntry::new(key("note"), MetaValue::Text("first".to_owned())),
                MetaEntry::new(key("payee"), MetaValue::Text("Generic Grocer".to_owned())),
                MetaEntry::new(key("note"), MetaValue::Text("second".to_owned())),
            ]),
            Metadata::default(),
        ))
        .await
        .expect("create");

        let found = svc.find_by_id(&tx_id).await.expect("find");
        let keys: Vec<&str> = found.metadata().iter().map(|e| e.key().as_str()).collect();
        assert_eq!(
            keys,
            vec!["note", "payee", "note"],
            "repeats and their position survive the round trip"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn every_load_path_reads_metadata(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account = test_account(&pool, "Checking").await;
        let tx_id = TransactionId::new();
        svc.create(tx_with_metadata(
            &tx_id,
            &PostingId::new(),
            &account,
            Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
            Metadata::new(vec![MetaEntry::new(
                key("channel"),
                MetaValue::Text("card".to_owned()),
            )]),
        ))
        .await
        .expect("create");

        // Each loader assembles postings on its own path, so one missed call
        // costs exactly one of them and nothing else.
        let expect_loaded = |tx: &Transaction, via: &str| {
            assert_eq!(
                tx.metadata().get_first_text(&key("payee")),
                Some("Generic Grocer"),
                "{via} must load transaction metadata"
            );
            assert!(
                tx.postings()
                    .iter()
                    .any(|p| p.metadata().get_first_text(&key("channel")) == Some("card")),
                "{via} must load posting metadata"
            );
        };

        expect_loaded(&svc.find_by_id(&tx_id).await.expect("find"), "find_by_id");
        expect_loaded(
            svc.list().await.expect("list").first().expect("one"),
            "list",
        );
        expect_loaded(
            &svc.list_for_account(&account)
                .await
                .expect("list_for_account")
                .next()
                .expect("one"),
            "list_for_account",
        );
        expect_loaded(
            &svc.list_for_account_tree(&account)
                .await
                .expect("list_for_account_tree")
                .next()
                .expect("one"),
            "list_for_account_tree",
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_reversal_copies_the_original_metadata(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account = test_account(&pool, "Checking").await;
        let tx_id = TransactionId::new();
        svc.create(tx_with_metadata(
            &tx_id,
            &PostingId::new(),
            &account,
            Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
            Metadata::new(vec![MetaEntry::new(
                key("channel"),
                MetaValue::Text("card".to_owned()),
            )]),
        ))
        .await
        .expect("create");

        let reversal_id = svc.reverse(&tx_id).await.expect("reverse");
        let reversal = svc.find_by_id(&reversal_id).await.expect("find reversal");

        assert_eq!(
            reversal.metadata().get_first_text(&key("payee")),
            Some("Generic Grocer")
        );
        assert!(
            reversal
                .postings()
                .iter()
                .any(|p| p.metadata().get_first_text(&key("channel")) == Some("card")),
            "the reversed leg carries the original leg's metadata"
        );
        assert_eq!(
            svc.find_by_id(&tx_id)
                .await
                .expect("original still loads")
                .metadata()
                .get_first_text(&key("payee")),
            Some("Generic Grocer"),
            "the original keeps its own entries"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_edit_replaces_metadata_rather_than_appending(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account = test_account(&pool, "Checking").await;
        let tx_id = TransactionId::new();
        let posting_id = PostingId::new();

        svc.create(tx_with_metadata(
            &tx_id,
            &posting_id,
            &account,
            Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Generic Grocer".to_owned()),
            )]),
            Metadata::default(),
        ))
        .await
        .expect("create");

        let current = svc.find_by_id(&tx_id).await.expect("load");
        let edited = Transaction::builder()
            .id(tx_id.clone())
            .date(current.date())
            .description(current.description().to_owned())
            .reconciliation(current.reconciliation())
            .created_at(*current.created_at())
            .metadata(Metadata::new(vec![MetaEntry::new(
                key("payee"),
                MetaValue::Text("Other Grocer".to_owned()),
            )]))
            .postings(current.postings().to_vec())
            .build();
        svc.edit(edited).await.expect("edit");

        let found = svc.find_by_id(&tx_id).await.expect("reload");
        assert_eq!(found.metadata().len(), 1, "the edit replaced, not appended");
        assert_eq!(
            found.metadata().get_first_text(&key("payee")),
            Some("Other Grocer")
        );
    }

    // MARK: Service::reconcile tests

    #[sqlx::test(migrations = "./migrations")]
    async fn reconcile_sets_state_when_balanced(pool: sqlx::SqlitePool) {
        let acct_svc = crate::AccountService::new(pool.clone());
        let checking_id = acct_svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create checking account");
        let expense_id = acct_svc
            .create()
            .name("Expenses")
            .account_type(bc_models::AccountType::Expense)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create expense account");

        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date("2026-06-01".parse::<Date>().expect("valid date"))
            .description("Balanced")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(checking_id)
                    .amount(Amount::new(
                        rust_decimal::Decimal::new(-5000, 2),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(expense_id)
                    .amount(Amount::new(
                        rust_decimal::Decimal::new(5000, 2),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let id = svc
            .create(tx)
            .await
            .expect("create balanced tx")
            .into_inner();

        svc.reconcile(&id, Reconciliation::Reconciled)
            .await
            .expect("reconcile balanced tx should succeed");
        let loaded = svc.find_by_id(&id).await.expect("find after reconcile");
        assert_eq!(loaded.reconciliation(), Reconciliation::Reconciled);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn audit_trail_returns_events_in_order(pool: sqlx::SqlitePool) {
        let service = Service::new(pool.clone());
        let (tx_id, posting_id, new_account_id) = seed_editable_tx(&pool).await;
        let current = service.find_by_id(&tx_id).await.expect("load");
        let updated = current
            .clone()
            .recategorise_posting(&posting_id, new_account_id);
        service.edit(updated).await.expect("edit");

        let trail = service.audit_trail(&tx_id).await.expect("trail");
        assert!(matches!(
            trail.first().map(|(_, e)| e),
            Some(Event::TransactionCreated { .. })
        ));
        assert!(
            trail
                .iter()
                .any(|(_, e)| matches!(e, Event::PostingRecategorised { .. }))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_for_budget_respects_query_filter(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let gym = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("gym");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let svc = Service::new(pool.clone());
        for (desc, amt) in [("Membership", dec!(30)), ("Locker fee", dec!(5))] {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "negation of a bounded test amount"
            )]
            let neg_amt = -amt;
            svc.create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(date(2026, 6, 3))
                    .description(desc)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(gym.clone())
                            .amount(Amount::new(amt, CommodityCode::new("AUD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(neg_amt, CommodityCode::new("AUD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        }

        let query = crate::search::TransactionQuery {
            text: Some("membership".to_owned()),
            ..Default::default()
        };
        let txns = svc
            .list_for_budget(&gym, None, date(2026, 6, 1), date(2026, 7, 1), Some(&query))
            .await
            .expect("list");

        assert_eq!(txns.len(), 1);
        assert_eq!(txns.first().expect("one tx").description(), "Membership");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_for_budget_amount_filter_uses_budget_leg(pool: sqlx::SqlitePool) {
        // Parity regression: the budget tree counts actuals over postings on
        // the budget account's subtree that themselves satisfy the global
        // amount filter. For a split transaction whose *budget* leg does not
        // match but a *non-budget* leg does, the tree counts zero, so the
        // drill-down list must exclude it too (no "click a number, drill in,
        // see a different set").
        let accounts = crate::account::Service::new(pool.clone());
        let gym = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("gym");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let svc = Service::new(pool.clone());

        // Split transaction: budget leg is USD 10 (below the filter), a
        // non-budget leg is USD 500 (above it).
        let split_id = bc_models::TransactionId::new();
        svc.create(
            Transaction::builder()
                .id(split_id.clone())
                .date(date(2026, 6, 3))
                .description("Split")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(gym.clone())
                        .amount(Amount::new(dec!(10), CommodityCode::new("USD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(500), CommodityCode::new("USD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("split tx");

        // Positive control: budget leg itself matches the filter.
        let matching_id = bc_models::TransactionId::new();
        svc.create(
            Transaction::builder()
                .id(matching_id.clone())
                .date(date(2026, 6, 4))
                .description("Big")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(gym.clone())
                        .amount(Amount::new(dec!(200), CommodityCode::new("USD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-200), CommodityCode::new("USD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("matching tx");

        let query = crate::search::TransactionQuery {
            amount: Some(crate::search::AmountQuery {
                min: Some(dec!(100)),
                commodity: Some(CommodityCode::new("USD")),
                ..Default::default()
            }),
            ..Default::default()
        };
        let txns = svc
            .list_for_budget(&gym, None, date(2026, 6, 1), date(2026, 7, 1), Some(&query))
            .await
            .expect("list");

        let ids: Vec<_> = txns.iter().map(|t| t.id().clone()).collect();
        assert!(
            !ids.contains(&split_id),
            "split tx whose budget leg is below the amount filter must be excluded"
        );
        assert!(
            ids.contains(&matching_id),
            "tx whose budget leg matches the amount filter must be included"
        );
        assert_eq!(txns.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_for_budget_tag_filter_flows_down_and_scopes_to_budget_leg(
        pool: sqlx::SqlitePool,
    ) {
        // The unfiltered drill-down must agree with the budget tree's counted
        // postings for the budget's own tag filter: a transaction tag flows down
        // to the budget leg (included), a descendant tag matches via the subtree
        // (included), but a tag on a *non-budget* leg does not leak the
        // transaction in (excluded).
        let accounts = crate::account::Service::new(pool.clone());
        let health = accounts
            .create()
            .name("Health")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("health");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let wellness = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, 'wellness', ?)")
            .bind(wellness.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert wellness");
        let gym_tag = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, parent_id, created_at) VALUES (?, 'gym', ?, ?)")
            .bind(gym_tag.to_string())
            .bind(wellness.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert gym");

        let svc = Service::new(pool.clone());

        // (a) tag on the TRANSACTION, health leg untagged -> included.
        let tx_level_id = bc_models::TransactionId::new();
        svc.create(
            Transaction::builder()
                .id(tx_level_id.clone())
                .date(date(2026, 6, 11))
                .description("Checkup")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(40), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-40), CommodityCode::new("AUD")))
                        .build(),
                ])
                .tag_ids(vec![wellness.clone()])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("tx a");

        // (b) descendant tag on the health POSTING -> included via subtree.
        let subtree_id = bc_models::TransactionId::new();
        svc.create(
            Transaction::builder()
                .id(subtree_id.clone())
                .date(date(2026, 6, 12))
                .description("Gym")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(25), CommodityCode::new("AUD")))
                        .tag_ids(vec![gym_tag.clone()])
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-25), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("tx b");

        // (c) tag only on the NON-budget (checking) leg -> excluded.
        let sibling_id = bc_models::TransactionId::new();
        svc.create(
            Transaction::builder()
                .id(sibling_id.clone())
                .date(date(2026, 6, 13))
                .description("Sibling")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(15), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-15), CommodityCode::new("AUD")))
                        .tag_ids(vec![wellness.clone()])
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("tx c");

        let txns = svc
            .list_for_budget(
                &health,
                Some(&wellness),
                date(2026, 6, 1),
                date(2026, 7, 1),
                None,
            )
            .await
            .expect("list");
        let ids: Vec<_> = txns.iter().map(|t| t.id().clone()).collect();

        assert!(
            ids.contains(&tx_level_id),
            "transaction-level tag must flow down to the budget leg"
        );
        assert!(
            ids.contains(&subtree_id),
            "descendant tag on the budget leg must match via the subtree"
        );
        assert!(
            !ids.contains(&sibling_id),
            "a tag on a non-budget leg must not pull the transaction in"
        );
        assert_eq!(txns.len(), 2);
    }
}
