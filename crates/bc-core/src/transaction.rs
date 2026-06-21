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
    /// Decimal string for the posting amount.
    amount: String,
    /// Commodity code string.
    commodity: String,
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

/// Validates that the postings in a transaction sum to zero per commodity.
fn validate_balance(postings: &[Posting]) -> BcResult<()> {
    if postings.is_empty() {
        return Err(BcError::UnbalancedTransaction);
    }

    let mut sums: std::collections::BTreeMap<&str, Decimal> = std::collections::BTreeMap::new();
    for p in postings {
        let entry: &mut Decimal = sums.entry(p.amount().commodity().as_str()).or_default();
        *entry = entry
            .checked_add(p.amount().value())
            .ok_or(BcError::BadData("posting sum overflow".into()))?;
    }
    for (commodity, sum) in &sums {
        if !sum.is_zero() {
            tracing::warn!(%commodity, %sum, "transaction postings do not balance");
            return Err(BcError::UnbalancedTransaction);
        }
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
    /// Decimal string for the posting amount.
    amount: String,
    /// Commodity code string.
    commodity: String,
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
    /// Returns [`BcError::UnbalancedTransaction`] if postings do not sum to zero.
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(&self, tx: Transaction) -> BcResult<TransactionId> {
        validate_balance(tx.postings())?;

        let tx_id = tx.id().clone();
        let event = Event::TransactionCreated { id: tx_id.clone() };

        let date_str = tx.date().to_string();
        let created_at_str = tx.created_at().to_string();

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO transactions (id, date, payee, description, reconciliation, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(tx_id.to_string())
        .bind(&date_str)
        .bind(tx.payee())
        .bind(tx.description())
        .bind(to_db_str(tx.reconciliation())?)
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        for tag_id in tx.tag_ids() {
            sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
                .bind(tx_id.to_string())
                .bind(tag_id.to_string())
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
            .bind(posting.amount().value().to_string()) //  4. amount
            .bind(posting.amount().commodity().as_str()) //  5. commodity
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

            for tag_id in posting.tag_ids() {
                sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
                    .bind(posting.id().to_string())
                    .bind(tag_id.to_string())
                    .execute(&mut *db_tx)
                    .await?;
            }
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
        let tx_row = sqlx::query_as::<_, (String, String, Option<String>, String, String, String)>(
            "SELECT id, date, payee, description, reconciliation, created_at \
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

        let reconciliation = from_db_str::<Reconciliation>(&tx_row.4)?;

        let created_at = tx_row
            .5
            .parse::<Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at '{}': {e}", tx_row.5)))?;

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
                let value = row.amount.parse::<Decimal>().map_err(|e| {
                    BcError::BadData(format!("invalid amount '{}': {e}", row.amount))
                })?;
                let amount = Amount::new(value, CommodityCode::new(row.commodity));
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
                    .amount(amount)
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
            .postings(postings)
            .reconciliation(reconciliation)
            .tag_ids(tag_ids)
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
            let negated_value = posting
                .amount()
                .value()
                .checked_mul(Decimal::NEGATIVE_ONE)
                .ok_or_else(|| BcError::BadData("posting amount negation overflow".into()))?;
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
            .bind(negated_value.to_string())
            .bind(posting.amount().commodity().as_str())
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
        let tx_rows: Vec<(String, String, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT id, date, payee, description, reconciliation, created_at \
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
            let value = row
                .amount
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{}': {e}", row.amount)))?;
            let amount = Amount::new(value, CommodityCode::new(row.commodity));
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
                .amount(amount)
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
                |(id_str, date_str, payee, description, reconciliation_str, created_at_str)| {
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
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
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

        let tx_rows: Vec<(String, String, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT t.id, t.date, t.payee, t.description, t.reconciliation, t.created_at \
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
            let value = row
                .amount
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{}': {e}", row.amount)))?;
            let amount = Amount::new(value, CommodityCode::new(row.commodity));
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
                .amount(amount)
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
                |(id_str, date_str, payee, description, reconciliation_str, created_at_str)| {
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
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
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

        let tx_rows: Vec<(String, String, Option<String>, String, String, String)> =
            match (&date_from_str, &date_until_str) {
                (Some(from), Some(until)) => sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) \
                     SELECT t.id, t.date, t.payee, t.description, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.reconciliation, t.created_at \
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
                     SELECT t.id, t.date, t.payee, t.description, t.reconciliation, t.created_at \
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
            let value = row
                .amount
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{}': {e}", row.amount)))?;
            let amount = Amount::new(value, CommodityCode::new(row.commodity));
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
                .amount(amount)
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
                |(id_str, date_str, payee, description, reconciliation_str, created_at_str)| {
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
                    let postings = postings_by_tx.remove(&id_str).unwrap_or_default();
                    Ok(Transaction::builder()
                        .id(tx_id)
                        .date(date)
                        .maybe_payee(payee)
                        .description(description)
                        .postings(postings)
                        .reconciliation(reconciliation)
                        .tag_ids(tag_ids)
                        .created_at(created_at)
                        .build())
                },
            )
            .collect::<BcResult<Vec<_>>>()
            .map(IntoIterator::into_iter)
    }

    /// Amends an existing transaction, replacing its projection row and all postings atomically.
    ///
    /// The event append, projection UPDATE, posting DELETE/INSERT, and tag DELETE/INSERT
    /// are all wrapped in a single SQLite transaction so they succeed or fail atomically.
    /// `posting_tags` rows are deleted before `postings` rows to satisfy the FK constraint
    /// `posting_tags.posting_id REFERENCES postings(id)` enforced by `PRAGMA foreign_keys = ON`.
    ///
    /// # Arguments
    ///
    /// * `updated` - The new transaction state. Must carry the same [`TransactionId`]
    ///   as the existing transaction. All postings are replaced.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::UnbalancedTransaction`] if postings do not sum to zero.
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn amend(&self, updated: Transaction) -> BcResult<()> {
        validate_balance(updated.postings())?;

        let tx_id = updated.id().clone();
        let tx_id_str = tx_id.to_string();
        let event = Event::TransactionAmended {
            id: tx_id.clone(),
            date: updated.date(),
            description: updated.description().to_owned(),
            payee: updated.payee().map(str::to_owned),
        };
        let date_str = updated.date().to_string();

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        let result = sqlx::query(
            "UPDATE transactions SET date = ?, payee = ?, description = ? WHERE id = ?",
        )
        .bind(&date_str)
        .bind(updated.payee())
        .bind(updated.description())
        .bind(&tx_id_str)
        .execute(&mut *db_tx)
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
        .execute(&mut *db_tx)
        .await?;

        sqlx::query("DELETE FROM postings WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut *db_tx)
            .await?;

        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(&tx_id_str)
            .execute(&mut *db_tx)
            .await?;

        for tag_id in updated.tag_ids() {
            sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
                .bind(&tx_id_str)
                .bind(tag_id.to_string())
                .execute(&mut *db_tx)
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
            .bind(posting.amount().value().to_string()) //  4. amount
            .bind(posting.amount().commodity().as_str()) //  5. commodity
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

            for tag_id in posting.tag_ids() {
                sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
                    .bind(posting.id().to_string())
                    .bind(tag_id.to_string())
                    .execute(&mut *db_tx)
                    .await?;
            }
        }

        db_tx.commit().await?;
        tracing::info!(transaction_id = %tx_id, "transaction amended");
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
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

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
    async fn create_unbalanced_transaction_fails(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;
        let svc = Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Unbalanced")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(AccountId::new())
                    .amount(Amount::new(dec!(50.00), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let result = svc.create(tx).await;
        assert!(matches!(result, Err(BcError::UnbalancedTransaction)));
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
        let orig_sum: rust_decimal::Decimal =
            orig.postings().iter().map(|p| p.amount().value()).sum();
        let rev_sum: rust_decimal::Decimal =
            reversal.postings().iter().map(|p| p.amount().value()).sum();
        pretty_assertions::assert_eq!(orig_sum, rust_decimal::Decimal::ZERO);
        pretty_assertions::assert_eq!(rev_sum, rust_decimal::Decimal::ZERO);
        pretty_assertions::assert_eq!(reversal.postings().len(), orig.postings().len());

        // Amounts are negated.
        for (orig_p, rev_p) in orig.postings().iter().zip(reversal.postings().iter()) {
            let rev_negated = rev_p
                .amount()
                .value()
                .checked_mul(rust_decimal::Decimal::NEGATIVE_ONE)
                .expect("negation should not overflow in test");
            pretty_assertions::assert_eq!(
                orig_p.amount().value(),
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
}
