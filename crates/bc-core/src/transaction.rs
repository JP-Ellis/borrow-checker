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
use bc_models::TransactionLinkId;
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
    /// Optional free-text note.
    note: Option<String>,
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
/// Fields: `(id, date, payee, description, note, reconciliation, created_at)`.
type TxRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    String,
);

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
/// editable fields (payee, date, description, note, `tag_ids`, `extra_dates`,
/// posting account/amount/note/tags/spread) while carrying forward from `current`:
/// - `extra_dates`: taken from `updated` (the DTO is authoritative; Task 2+).
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
                .maybe_note(p.note().map(str::to_owned))
                .tag_ids(p.tag_ids().to_vec())
                .maybe_spread_from(p.spread_from())
                .maybe_spread_until(p.spread_until())
                .build()
        })
        .collect();

    Transaction::builder()
        .id(updated.id().clone())
        .date(updated.date())
        .maybe_payee(updated.payee().map(str::to_owned))
        .description(updated.description().to_owned())
        .maybe_note(updated.note().map(str::to_owned))
        .postings(merged_postings)
        .reconciliation(current.reconciliation())
        .tag_ids(updated.tag_ids().to_vec())
        .extra_dates(updated.extra_dates().to_vec())
        .created_at(*updated.created_at())
        .build()
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
#[expect(
    clippy::too_many_lines,
    reason = "the diff covers all scalar and posting-level fields; extraction would obscure the sequential check logic"
)]
pub(crate) fn diff_transaction(current: &Transaction, updated: &Transaction) -> Vec<Event> {
    let id = updated.id().clone();
    let mut events = Vec::new();

    if current.payee() != updated.payee() {
        events.push(Event::TransactionPayeeChanged {
            id: id.clone(),
            from: current.payee().map(str::to_owned),
            to: updated.payee().map(str::to_owned),
        });
    }
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
    if current.note() != updated.note() {
        events.push(Event::TransactionNoteChanged {
            id: id.clone(),
            from: current.note().map(str::to_owned),
            to: updated.note().map(str::to_owned),
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

    if current.extra_dates() != updated.extra_dates() {
        events.push(Event::TransactionExtraDatesChanged {
            id: id.clone(),
            from: current.extra_dates().to_vec(),
            to: updated.extra_dates().to_vec(),
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
            Some(prev) => {
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
                if prev.note() != posting.note() {
                    events.push(Event::PostingNoteChanged {
                        id: id.clone(),
                        posting_id: posting.id().clone(),
                        from: prev.note().map(str::to_owned),
                        to: posting.note().map(str::to_owned),
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
            }
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
    /// Optional free-text note.
    note: Option<String>,
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

    /// Persists a transaction after validating double-entry balance.
    ///
    /// The event append and all projection inserts are wrapped in a single
    /// SQLite transaction so they succeed or fail atomically.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, contains two
    /// or more elided amounts, or is a single lone elided posting.
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(&self, tx: Transaction) -> BcResult<TransactionId> {
        validate_postings(tx.postings())?;

        let tx_id = tx.id().clone();
        let event = Event::TransactionCreated { id: tx_id.clone() };

        let date_str = tx.date().to_string();
        let created_at_str = tx.created_at().to_string();

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO transactions (id, date, payee, description, note, reconciliation, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(tx_id.to_string())
        .bind(&date_str)
        .bind(tx.payee())
        .bind(tx.description())
        .bind(tx.note())
        .bind(to_db_str(tx.reconciliation())?)
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        crate::tag::insert_transaction_tags(&mut db_tx, &tx_id, tx.tag_ids()).await?;

        for (label, date) in tx.extra_dates() {
            sqlx::query(
                "INSERT INTO transaction_dates (transaction_id, label, date) VALUES (?, ?, ?)",
            )
            .bind(tx_id.to_string())
            .bind(label)
            .bind(date.to_string())
            .execute(&mut *db_tx)
            .await?;
        }

        for (position, posting) in tx.postings().iter().enumerate() {
            let (cost_value, cost_commodity, cost_date, cost_label) =
                if let Some(cost) = posting.cost() {
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
                 (id, transaction_id, account_id, amount, commodity, note, position, \
                  cost_total_value, cost_total_commodity, cost_date, cost_label, \
                  spread_from, spread_until) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(posting.id().to_string()) //  1. id
            .bind(tx_id.to_string()) //  2. transaction_id
            .bind(posting.account_id().to_string()) //  3. account_id
            .bind(posting.amount().map(|a| a.value().to_string())) //  4. amount
            .bind(posting.amount().map(|a| a.commodity().as_str().to_owned())) //  5. commodity
            .bind(posting.note()) //  6. note
            .bind(
                i64::try_from(position)
                    .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?,
            ) //  7. position
            .bind(cost_value) //  8. cost_total_value
            .bind(cost_commodity) //  9. cost_total_commodity
            .bind(cost_date) // 10. cost_date
            .bind(cost_label) // 11. cost_label
            .bind(posting.spread_from().map(|d| d.to_string())) // 12. spread_from
            .bind(posting.spread_until().map(|d| d.to_string())) // 13. spread_until
            .execute(&mut *db_tx)
            .await?;

            crate::tag::insert_posting_tags(&mut db_tx, posting.id(), posting.tag_ids()).await?;
        }

        db_tx.commit().await?;
        tracing::info!(transaction_id = %tx_id, "transaction created");
        Ok(tx_id)
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
            "SELECT id, date, payee, description, note, reconciliation, created_at \
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

        let reconciliation = from_db_str::<Reconciliation>(&tx_row.5)?;

        let created_at = tx_row
            .6
            .parse::<Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at '{}': {e}", tx_row.6)))?;

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

        // Load extra labeled dates.
        let extra_date_rows: Vec<(String, String)> =
            sqlx::query_as("SELECT label, date FROM transaction_dates WHERE transaction_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        let extra_dates: Vec<(String, Date)> = extra_date_rows
            .into_iter()
            .map(|(label, date_str)| {
                date_str
                    .parse::<Date>()
                    .map(|d| (label, d))
                    .map_err(|e| BcError::BadData(format!("invalid extra date '{date_str}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        // Load postings with cost and spread columns.
        let posting_rows: Vec<PostingRow> = sqlx::query_as(
            "SELECT id, account_id, amount, commodity, note, \
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
                Ok(Posting::builder()
                    .id(posting_id)
                    .account_id(acc_id)
                    .maybe_amount(amount)
                    .maybe_cost(cost)
                    .maybe_note(row.note)
                    .maybe_spread_from(spread_from)
                    .maybe_spread_until(spread_until)
                    .tag_ids(p_tag_ids)
                    .build())
            })
            .collect::<BcResult<Vec<_>>>()?;

        Ok(Transaction::builder()
            .id(tx_id)
            .date(date)
            .maybe_payee(tx_row.2)
            .description(tx_row.3)
            .maybe_note(tx_row.4)
            .postings(postings)
            .reconciliation(reconciliation)
            .tag_ids(tag_ids)
            .extra_dates(extra_dates)
            .created_at(created_at)
            .build())
    }

    /// Creates a reversal transaction for the given transaction.
    ///
    /// A reversal inserts a new transaction with the same postings negated, a
    /// description of `"Reversal of {id}"`, and `Reconciliation::Unreconciled`.  One
    /// `transaction_links` row (`link_type = 'reversal'`) and two
    /// `transaction_link_members` rows (original + reversal) are inserted in the
    /// same database transaction to tie them together atomically.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with the given ID exists.
    /// Returns [`BcError`] on database insert failure.
    #[inline]
    pub async fn reverse(&self, id: &TransactionId) -> BcResult<TransactionId> {
        let original = self.find_by_id(id).await?;

        let reversal_id = TransactionId::new();
        let link_id = TransactionLinkId::new();
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
            "INSERT INTO transactions (id, date, payee, description, reconciliation, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(reversal_id.to_string())
        .bind(original.date().to_string())
        .bind(original.payee())
        .bind(&description)
        .bind(&unreconciled_str)
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        // Insert negated postings for the reversal.
        for (position, posting) in original.postings().iter().enumerate() {
            let (negated_amount_str, commodity_str) = if let Some(amount) = posting.amount() {
                let negated = amount
                    .value()
                    .checked_mul(Decimal::NEGATIVE_ONE)
                    .ok_or_else(|| BcError::BadData("posting amount negation overflow".into()))?;
                (
                    Some(negated.to_string()),
                    Some(amount.commodity().as_str().to_owned()),
                )
            } else {
                (None, None)
            };
            let (cost_value, cost_commodity, cost_date, cost_label) =
                if let Some(cost) = posting.cost() {
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
                 (id, transaction_id, account_id, amount, commodity, note, position, \
                  cost_total_value, cost_total_commodity, cost_date, cost_label, \
                  spread_from, spread_until) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(PostingId::new().to_string())
            .bind(reversal_id.to_string())
            .bind(posting.account_id().to_string())
            .bind(negated_amount_str)
            .bind(commodity_str)
            .bind(posting.note())
            .bind(
                i64::try_from(position)
                    .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?,
            )
            .bind(cost_value)
            .bind(cost_commodity)
            .bind(cost_date)
            .bind(cost_label)
            .bind(posting.spread_from().map(|d| d.to_string()))
            .bind(posting.spread_until().map(|d| d.to_string()))
            .execute(&mut *db_tx)
            .await?;
        }

        // Insert the link registry row.
        sqlx::query(
            "INSERT INTO transaction_links (id, link_type, created_at) VALUES (?, 'reversal', ?)",
        )
        .bind(link_id.to_string())
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        // Insert both members: original and reversal.
        sqlx::query(
            "INSERT INTO transaction_link_members (link_id, transaction_id) VALUES (?, ?), (?, ?)",
        )
        .bind(link_id.to_string())
        .bind(id.to_string())
        .bind(link_id.to_string())
        .bind(reversal_id.to_string())
        .execute(&mut *db_tx)
        .await?;

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
            "SELECT id, date, payee, description, note, reconciliation, created_at \
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

        // Load all extra labeled dates in one query.
        let extra_date_rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT td.transaction_id, td.label, td.date \
             FROM transaction_dates td",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut tx_extra_dates_map: HashMap<String, Vec<(String, Date)>> = HashMap::new();
        for (tx_id_str, label, date_str) in extra_date_rows {
            let d = date_str
                .parse::<Date>()
                .map_err(|e| BcError::BadData(format!("invalid extra date '{date_str}': {e}")))?;
            tx_extra_dates_map
                .entry(tx_id_str)
                .or_default()
                .push((label, d));
        }

        // Load all postings in one query.
        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, p.note, \
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
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .maybe_note(row.note)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(
                    id_str,
                    date_str,
                    payee,
                    description,
                    note,
                    reconciliation_str,
                    created_at_str,
                )| {
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
                    let extra_dates = tx_extra_dates_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .maybe_note(note)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .extra_dates(extra_dates)
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
    #[expect(
        clippy::too_many_lines,
        reason = "loading transactions with postings, cost, and tags for a specific account inherently requires several queries and field mappings"
    )]
    pub async fn list_for_account(
        &self,
        account_id: &AccountId,
    ) -> BcResult<impl Iterator<Item = Transaction>> {
        let account_id_str = account_id.to_string();

        let tx_rows: Vec<TxRow> = sqlx::query_as(
            "SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
                 FROM transactions t \
                 WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?) \
                 ORDER BY t.date DESC",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        if tx_rows.is_empty() {
            return Ok(vec![].into_iter());
        }

        let tx_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT tt.transaction_id, tt.tag_id \
             FROM transaction_tags tt \
             JOIN transactions t ON tt.transaction_id = t.id \
             WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?)",
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

        // Load extra labeled dates for the matching transactions.
        let extra_date_rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT td.transaction_id, td.label, td.date \
             FROM transaction_dates td \
             JOIN transactions t ON td.transaction_id = t.id \
             WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?)",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let mut tx_extra_dates_map: HashMap<String, Vec<(String, Date)>> = HashMap::new();
        for (tx_id_str, label, date_str) in extra_date_rows {
            let d = date_str
                .parse::<Date>()
                .map_err(|e| BcError::BadData(format!("invalid extra date '{date_str}': {e}")))?;
            tx_extra_dates_map
                .entry(tx_id_str)
                .or_default()
                .push((label, d));
        }

        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, p.note, \
                    p.cost_total_value, p.cost_total_commodity, p.cost_date, p.cost_label, \
                    p.spread_from, p.spread_until \
             FROM postings p \
             WHERE p.transaction_id IN \
                 (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?) \
             ORDER BY p.transaction_id, p.position ASC",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let posting_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt \
             JOIN postings p ON pt.posting_id = p.id \
             WHERE p.transaction_id IN \
                 (SELECT DISTINCT transaction_id FROM postings WHERE account_id = ?)",
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
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .maybe_note(row.note)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(
                    id_str,
                    date_str,
                    payee,
                    description,
                    note,
                    reconciliation_str,
                    created_at_str,
                )| {
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
                    let extra_dates = tx_extra_dates_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .maybe_note(note)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .extra_dates(extra_dates)
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
                     SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
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

        // Load extra labeled dates for the matching transactions.
        let extra_date_rows: Vec<(String, String, String)> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) \
             SELECT td.transaction_id, td.label, td.date \
             FROM transaction_dates td \
             JOIN transactions t ON td.transaction_id = t.id \
             WHERE t.id IN (SELECT DISTINCT transaction_id FROM postings WHERE account_id IN (SELECT id FROM subtree))",
        )
        .bind(&account_id_str)
        .fetch_all(&self.pool)
        .await?;

        let mut tx_extra_dates_map: HashMap<String, Vec<(String, Date)>> = HashMap::new();
        for (tx_id_str, label, date_str) in extra_date_rows {
            let d = date_str
                .parse::<Date>()
                .map_err(|e| BcError::BadData(format!("invalid extra date '{date_str}': {e}")))?;
            tx_extra_dates_map
                .entry(tx_id_str)
                .or_default()
                .push((label, d));
        }

        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) \
             SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, p.note, \
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
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .maybe_amount(amount)
                .maybe_cost(cost)
                .maybe_note(row.note)
                .maybe_spread_from(spread_from)
                .maybe_spread_until(spread_until)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(
                    id_str,
                    date_str,
                    payee,
                    description,
                    note,
                    reconciliation_str,
                    created_at_str,
                )| {
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
                    let extra_dates = tx_extra_dates_map.remove(&id_str).unwrap_or_default();
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .maybe_note(note)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .extra_dates(extra_dates)
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
    /// postings, posting tags, transaction tags, and extra dates.
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

        let result = sqlx::query(
            "UPDATE transactions SET date = ?, payee = ?, description = ?, note = ? WHERE id = ?",
        )
        .bind(&date_str)
        .bind(updated.payee())
        .bind(updated.description())
        .bind(updated.note())
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

        sqlx::query("DELETE FROM postings WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        sqlx::query("DELETE FROM transaction_dates WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut **db_tx)
            .await?;

        crate::tag::insert_transaction_tags(&mut *db_tx, updated.id(), updated.tag_ids()).await?;

        for (label, date) in updated.extra_dates() {
            sqlx::query(
                "INSERT INTO transaction_dates (transaction_id, label, date) VALUES (?, ?, ?)",
            )
            .bind(&tx_id_str)
            .bind(label)
            .bind(date.to_string())
            .execute(&mut **db_tx)
            .await?;
        }

        for (position, posting) in updated.postings().iter().enumerate() {
            let (cost_value, cost_commodity, cost_date, cost_label) =
                if let Some(cost) = posting.cost() {
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
                 (id, transaction_id, account_id, amount, commodity, note, position, \
                  cost_total_value, cost_total_commodity, cost_date, cost_label, \
                  spread_from, spread_until) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(posting.id().to_string()) //  1. id
            .bind(&tx_id_str) //  2. transaction_id
            .bind(posting.account_id().to_string()) //  3. account_id
            .bind(posting.amount().map(|a| a.value().to_string())) //  4. amount
            .bind(posting.amount().map(|a| a.commodity().as_str().to_owned())) //  5. commodity
            .bind(posting.note()) //  6. note
            .bind(
                i64::try_from(position)
                    .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?,
            ) //  7. position
            .bind(cost_value) //  8. cost_total_value
            .bind(cost_commodity) //  9. cost_total_commodity
            .bind(cost_date) // 10. cost_date
            .bind(cost_label) // 11. cost_label
            .bind(posting.spread_from().map(|d| d.to_string())) // 12. spread_from
            .bind(posting.spread_until().map(|d| d.to_string())) // 13. spread_until
            .execute(&mut **db_tx)
            .await?;

            crate::tag::insert_posting_tags(&mut *db_tx, posting.id(), posting.tag_ids()).await?;
        }

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
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, contains two
    /// or more elided amounts, or is a single lone elided posting.
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn amend(&self, updated: Transaction) -> BcResult<()> {
        validate_postings(updated.postings())?;

        let tx_id = updated.id().clone();
        let event = Event::TransactionAmended {
            id: tx_id.clone(),
            date: updated.date(),
            description: updated.description().to_owned(),
            payee: updated.payee().map(str::to_owned),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;
        self.apply_transaction_projection(&mut db_tx, &updated)
            .await?;
        db_tx.commit().await?;
        tracing::info!(transaction_id = %tx_id, "transaction amended");
        Ok(())
    }

    /// Applies a desired transaction state, recording decomposed semantic events.
    ///
    /// Loads the current state, diffs it against `updated` to produce granular
    /// events (payee/date/note/tags and per-posting recategorise/amount/note/
    /// spread/add/remove), then atomically appends those events and rewrites the
    /// projection. Persistence is permissive: an unbalanced result is allowed.
    ///
    /// # Arguments
    ///
    /// * `updated` - The desired transaction state. Must carry the ID of an
    ///   existing transaction; all postings are replaced.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the posting list is empty, has ≥2 elided
    /// amounts, or is a lone elided posting. Returns [`BcError::NotFound`] if no
    /// transaction with that ID exists. Returns [`BcError`] on DB failure.
    #[inline]
    pub async fn edit(&self, updated: Transaction) -> BcResult<()> {
        validate_postings(updated.postings())?;

        let tx_id = updated.id().clone();
        let current = self.find_by_id(&tx_id).await?;
        let merged = merge_preserving(&current, &updated);
        let events = diff_transaction(&current, &merged);

        let mut db_tx = self.pool.begin().await?;
        for event in &events {
            insert_event(event, &mut db_tx).await?;
        }
        self.apply_transaction_projection(&mut db_tx, &merged)
            .await?;
        db_tx.commit().await?;

        tracing::info!(transaction_id = %tx_id, event_count = events.len(), "transaction edited");
        Ok(())
    }

    /// Lists all transactions with a posting against `account_id`
    /// (optionally filtered to postings tagged with `tag_filter`) in
    /// `[period_start, period_end)`.
    ///
    /// Because the tag filter is now time-varying (it lives on a
    /// [`bc_models::BudgetRevision`]), callers must resolve the governing
    /// revision themselves and pass the filter explicitly.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose posting tree to search.
    /// * `tag_filter` - Optional tag; only postings tagged with this tag (or a
    ///   descendant) are included.  `None` = no filter (all postings match).
    /// * `period_start` - Inclusive start of the date range.
    /// * `period_end` - Exclusive end of the date range.
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
    ) -> BcResult<Vec<Transaction>> {
        let txns: Vec<Transaction> = self
            .list_for_account_tree_in_range(account_id, Some(period_start), Some(period_end))
            .await?
            .collect();

        let result = if let Some(tag) = tag_filter {
            txns.into_iter()
                .filter(|tx| {
                    tx.postings()
                        .iter()
                        .any(|p| p.tag_ids().iter().any(|tid| tid == tag))
                })
                .collect()
        } else {
            txns
        };

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
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
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
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::diff_transaction;
    use super::*;
    use crate::events::Event;

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

        let tx_id = svc.create(tx).await.expect("create tx");
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

        let tx_id = svc.create(tx).await.expect("create tx");

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

        // A reversal link ties them together.
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transaction_links WHERE link_type = 'reversal'",
        )
        .fetch_one(&pool)
        .await
        .expect("count links");
        pretty_assertions::assert_eq!(link_count, 1);

        // Both transactions are members of the link.
        let member_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transaction_link_members")
            .fetch_one(&pool)
            .await
            .expect("count members");
        pretty_assertions::assert_eq!(member_count, 2);

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

        let id = svc.create(original.clone()).await.expect("create");

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
    async fn transaction_note_roundtrips(pool: sqlx::SqlitePool) {
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
            .note("my annotation")
            .postings(base.postings().to_vec())
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        svc.create(tx).await.expect("create");
        let found = svc.find_by_id(&id).await.expect("find");
        assert_eq!(found.note(), Some("my annotation"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn extra_dates_roundtrip(pool: sqlx::SqlitePool) {
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
            .extra_dates(vec![("cleared".to_owned(), date(2026, 1, 17))])
            .postings(base.postings().to_vec())
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        svc.create(tx).await.expect("create");
        let found = svc.find_by_id(&id).await.expect("find");
        assert_eq!(
            found.extra_dates(),
            &[("cleared".to_owned(), date(2026, 1, 17))]
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
    async fn amend_preserves_note_and_extra_dates(pool: sqlx::SqlitePool) {
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
            .description("Original payee")
            .maybe_note(Some("keep this note".to_owned()))
            .extra_dates(vec![("cleared".to_owned(), date(2026, 1, 12))])
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

        let id = svc.create(original.clone()).await.expect("create");

        let updated = Transaction::builder()
            .id(id.clone())
            .date(date(2026, 1, 10))
            .description("Amended payee")
            .maybe_note(original.note().map(str::to_owned))
            .extra_dates(original.extra_dates().to_vec())
            .postings(original.postings().to_vec())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(*original.created_at())
            .build();

        svc.amend(updated).await.expect("amend should succeed");

        let found = svc.find_by_id(&id).await.expect("find after amend");
        assert_eq!(found.description(), "Amended payee");
        assert_eq!(
            found.note(),
            Some("keep this note"),
            "note must survive amend"
        );
        assert_eq!(
            found.extra_dates(),
            &[("cleared".to_owned(), date(2026, 1, 12))],
            "extra_dates must survive amend"
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
            .maybe_payee(Some("Old Payee".to_owned()))
            .description("desc".to_owned())
            .postings(vec![p1, p2])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    trait TxTestExt {
        fn with_payee(self, payee: Option<String>) -> Self;
        fn recategorise_first(self, account: AccountId) -> Self;
        fn push_leg(self) -> Self;
        fn recategorise_posting(self, target: &PostingId, account: AccountId) -> Self;
    }

    impl TxTestExt for Transaction {
        fn with_payee(self, payee: Option<String>) -> Self {
            Transaction::builder()
                .id(self.id().clone())
                .date(self.date())
                .maybe_payee(payee)
                .description(self.description().to_owned())
                .maybe_note(self.note().map(str::to_owned))
                .postings(self.postings().to_vec())
                .reconciliation(self.reconciliation())
                .tag_ids(self.tag_ids().to_vec())
                .created_at(Timestamp::now())
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
                .maybe_payee(self.payee().map(str::to_owned))
                .description(self.description().to_owned())
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
                .maybe_payee(self.payee().map(str::to_owned))
                .description(self.description().to_owned())
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
                            .maybe_note(p.note().map(str::to_owned))
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
                .maybe_payee(self.payee().map(str::to_owned))
                .description(self.description().to_owned())
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
        let tx_id = svc.create(tx).await.expect("seed transaction created");
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
            .with_payee(Some("Edited Payee".to_owned()))
            .recategorise_posting(&posting_id, new_account_id.clone());
        service.edit(updated).await.expect("edit ok");

        let kinds: Vec<String> =
            sqlx::query_scalar("SELECT kind FROM events WHERE aggregate_id = ? ORDER BY rowid ASC")
                .bind(tx_id.to_string())
                .fetch_all(&pool)
                .await
                .expect("query events");
        assert!(kinds.contains(&"TransactionPayeeChanged".to_owned()));
        assert!(kinds.contains(&"PostingRecategorised".to_owned()));

        let reloaded = service.find_by_id(&tx_id).await.expect("reload");
        assert_eq!(reloaded.payee(), Some("Edited Payee"));
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
            .extra_dates(vec![("cleared".to_owned(), date(2026, 3, 3))])
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

        let tx_id = svc.create(original.clone()).await.expect("create");

        // Edit: only change the payee — extra_dates echoed from current, posting cost must survive.
        let current = svc.find_by_id(&tx_id).await.expect("load current");
        let edited = Transaction::builder()
            .id(tx_id.clone())
            .date(current.date())
            .maybe_payee(Some("New Payee".to_owned()))
            .description(current.description().to_owned())
            .maybe_note(current.note().map(str::to_owned))
            .postings(current.postings().to_vec())
            .reconciliation(current.reconciliation())
            .tag_ids(current.tag_ids().to_vec())
            .extra_dates(current.extra_dates().to_vec())
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
    async fn edit_can_change_extra_dates(pool: sqlx::SqlitePool) {
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
            .extra_dates(vec![("cleared".to_owned(), date(2026, 3, 3))])
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

        let tx_id = svc.create(original).await.expect("create");

        let current = svc.find_by_id(&tx_id).await.expect("load");
        let edited = Transaction::builder()
            .id(tx_id.clone())
            .date(current.date())
            .maybe_payee(current.payee().map(str::to_owned))
            .description(current.description().to_owned())
            .maybe_note(current.note().map(str::to_owned))
            .postings(current.postings().to_vec())
            .reconciliation(current.reconciliation())
            .tag_ids(current.tag_ids().to_vec())
            .extra_dates(vec![("effective".to_owned(), date(2026, 3, 10))])
            .created_at(*current.created_at())
            .build();
        svc.edit(edited).await.expect("edit ok");

        let reloaded = svc.find_by_id(&tx_id).await.expect("reload");
        assert_eq!(
            reloaded.extra_dates(),
            &[("effective".to_owned(), date(2026, 3, 10))]
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
            .with_payee(Some("Still Editable".to_owned()));
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

    #[test]
    fn diff_detects_payee_change() {
        let current = sample_tx();
        let updated = current.clone().with_payee(Some("New Payee".to_owned()));
        let events = diff_transaction(&current, &updated);
        assert_eq!(events.len(), 1);
        let first = events.first().expect("one event expected");
        assert!(matches!(
            first,
            Event::TransactionPayeeChanged { to, .. } if to.as_deref() == Some("New Payee")
        ));
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

    #[test]
    fn diff_emits_extra_dates_changed() {
        let current = sample_tx();
        let updated = Transaction::builder()
            .id(current.id().clone())
            .date(current.date())
            .maybe_payee(current.payee().map(str::to_owned))
            .description(current.description().to_owned())
            .maybe_note(current.note().map(str::to_owned))
            .postings(current.postings().to_vec())
            .reconciliation(current.reconciliation())
            .tag_ids(current.tag_ids().to_vec())
            .extra_dates(vec![("cleared".to_owned(), date(2026, 3, 3))])
            .created_at(*current.created_at())
            .build();
        let events = diff_transaction(&current, &updated);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::TransactionExtraDatesChanged { .. })),
            "changing extra_dates must emit TransactionExtraDatesChanged"
        );
    }

    #[test]
    fn diff_no_extra_dates_change_emits_nothing() {
        let tx = sample_tx();
        let events = diff_transaction(&tx, &tx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::TransactionExtraDatesChanged { .. }))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_updates_note(pool: sqlx::SqlitePool) {
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
            .date(date(2026, 2, 1))
            .description("Groceries")
            .maybe_note(Some("old note".to_owned()))
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(-30), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(dec!(30), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let id = svc.create(original.clone()).await.expect("create");

        let updated = Transaction::builder()
            .id(id.clone())
            .date(date(2026, 2, 1))
            .description("Groceries")
            .maybe_note(Some("new note".to_owned()))
            .postings(original.postings().to_vec())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(*original.created_at())
            .build();

        svc.amend(updated).await.expect("amend should succeed");

        let found = svc.find_by_id(&id).await.expect("find after amend");
        assert_eq!(found.note(), Some("new note"), "amended note must persist");
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
        let id = svc.create(tx).await.expect("create balanced tx");

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
}
