//! Transaction service with double-entry validation.

use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Cost;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::TagId;
use bc_models::Transaction;
use bc_models::TransactionId;
use bc_models::TransactionStatus;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;

/// Row type returned by the postings bulk-load query in [`Service::list`].
///
/// Includes `transaction_id` (field 1) so postings can be grouped by transaction.
type ListPostingRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
use crate::events::insert_event;

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

/// Column tuple returned from the `postings` table when loading a transaction.
type PostingRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

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
            "INSERT INTO transactions (id, date, payee, description, status, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(tx_id.to_string())
        .bind(&date_str)
        .bind(tx.payee())
        .bind(tx.description())
        .bind(to_db_str(tx.status())?)
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
                 (id, transaction_id, account_id, amount, commodity, memo, position, \
                  cost_total_value, cost_total_commodity, cost_date, cost_label) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(posting.id().to_string())
            .bind(tx_id.to_string())
            .bind(posting.account_id().to_string())
            .bind(posting.amount().value().to_string())
            .bind(posting.amount().commodity().as_str())
            .bind(posting.memo())
            .bind(
                i64::try_from(position)
                    .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?,
            )
            .bind(cost_value)
            .bind(cost_commodity)
            .bind(cost_date)
            .bind(cost_label)
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
        reason = "loading a transaction with postings, cost, and tags inherently requires several queries and field mappings"
    )]
    pub async fn find_by_id(&self, id: &TransactionId) -> BcResult<Transaction> {
        let tx_row = sqlx::query_as::<_, (String, String, Option<String>, String, String, String)>(
            "SELECT id, date, payee, description, status, created_at \
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

        let status = from_db_str::<TransactionStatus>(&tx_row.4)?;

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

        // Load postings with cost columns.
        let posting_rows: Vec<PostingRow> = sqlx::query_as(
            "SELECT id, account_id, amount, commodity, memo, \
                    cost_total_value, cost_total_commodity, cost_date, cost_label \
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
            .map(
                |(
                    pid,
                    account_id,
                    amount_str,
                    commodity,
                    memo,
                    cost_val,
                    cost_com,
                    cost_dt,
                    cost_lbl,
                )| {
                    let posting_id = pid.parse::<PostingId>().map_err(|e| {
                        BcError::BadData(format!("invalid posting id '{pid}': {e}"))
                    })?;
                    let acc_id = account_id.parse::<AccountId>().map_err(|e| {
                        BcError::BadData(format!("invalid account id '{account_id}': {e}"))
                    })?;
                    let value = amount_str.parse::<Decimal>().map_err(|e| {
                        BcError::BadData(format!("invalid amount '{amount_str}': {e}"))
                    })?;
                    let amount = Amount::new(value, CommodityCode::new(commodity));
                    let cost = parse_cost(cost_val, cost_com, cost_dt, cost_lbl)?;
                    let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
                    Ok(Posting::builder()
                        .id(posting_id)
                        .account_id(acc_id)
                        .amount(amount)
                        .maybe_cost(cost)
                        .maybe_memo(memo)
                        .tag_ids(p_tag_ids)
                        .build())
                },
            )
            .collect::<BcResult<Vec<_>>>()?;

        Ok(Transaction::builder()
            .id(tx_id)
            .date(date)
            .maybe_payee(tx_row.2)
            .description(tx_row.3)
            .postings(postings)
            .status(status)
            .tag_ids(tag_ids)
            .created_at(created_at)
            .build())
    }

    /// Voids a transaction by setting its status to `voided`.
    ///
    /// The event append and the projection UPDATE are wrapped in a single SQLite
    /// transaction so they succeed or fail atomically.  `rows_affected()` is used
    /// to detect a missing or already-voided transaction without a separate pre-check,
    /// eliminating a TOCTOU race.  The voided-status string is derived at runtime
    /// via [`to_db_str`] so it stays in sync with the serde representation.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError::AlreadyVoided`] if the transaction exists but is already voided.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn void(&self, id: &TransactionId) -> BcResult<()> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;
        let event = Event::TransactionVoided { id: id.clone() };

        let mut tx = self.pool.begin().await?;

        insert_event(&event, &mut tx).await?;

        let result = sqlx::query("UPDATE transactions SET status = ? WHERE id = ? AND status != ?")
            .bind(&voided_str)
            .bind(id.to_string())
            .bind(&voided_str)
            .execute(&mut *tx)
            .await?;

        if result.rows_affected() == 0 {
            // rows_affected == 0 means the UPDATE found no matching row.
            // Returning here drops `tx` without committing — sqlx rolls it
            // back implicitly, discarding the event insert above.
            //
            // Perform a follow-up SELECT to distinguish "not found" from
            // "already voided" so callers get a semantic error.
            let exists: bool =
                sqlx::query_scalar("SELECT count(*) > 0 FROM transactions WHERE id = ?")
                    .bind(id.to_string())
                    .fetch_one(&self.pool)
                    .await?;

            return if exists {
                Err(BcError::AlreadyVoided(id.clone()))
            } else {
                Err(BcError::NotFound(id.to_string()))
            };
        }

        tx.commit().await?;
        tracing::info!(transaction_id = %id, "transaction voided");
        Ok(())
    }

    /// Lists all non-voided transactions ordered by date descending, including postings.
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
        let voided_str = to_db_str(TransactionStatus::Voided)?;

        let tx_rows: Vec<(String, String, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT id, date, payee, description, status, created_at \
                 FROM transactions WHERE status != ? ORDER BY date DESC",
            )
            .bind(&voided_str)
            .fetch_all(&self.pool)
            .await?;

        if tx_rows.is_empty() {
            return Ok(vec![]);
        }

        // Load all transaction-level tags for non-voided transactions in one query.
        let tx_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT tt.transaction_id, tt.tag_id \
             FROM transaction_tags tt \
             JOIN transactions t ON tt.transaction_id = t.id \
             WHERE t.status != ?",
        )
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        let mut tx_tags_map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (tx_id_str, tag_id_str) in tx_tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            tx_tags_map.entry(tx_id_str).or_default().push(tid);
        }

        // Load all postings for non-voided transactions in one query.
        let posting_rows: Vec<ListPostingRow> = sqlx::query_as(
            "SELECT p.id, p.transaction_id, p.account_id, p.amount, p.commodity, p.memo, \
                    p.cost_total_value, p.cost_total_commodity, p.cost_date, p.cost_label \
             FROM postings p \
             JOIN transactions t ON p.transaction_id = t.id \
             WHERE t.status != ? \
             ORDER BY p.transaction_id, p.position ASC",
        )
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        // Load all posting tags for non-voided transactions in one query.
        let posting_tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT pt.posting_id, pt.tag_id \
             FROM posting_tags pt \
             JOIN postings p ON pt.posting_id = p.id \
             JOIN transactions t ON p.transaction_id = t.id \
             WHERE t.status != ?",
        )
        .bind(&voided_str)
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
        for (
            pid,
            tx_id_str,
            account_id,
            amount_str,
            commodity,
            memo,
            cost_val,
            cost_com,
            cost_dt,
            cost_lbl,
        ) in posting_rows
        {
            let posting_id = pid
                .parse::<PostingId>()
                .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
            let acc_id = account_id
                .parse::<AccountId>()
                .map_err(|e| BcError::BadData(format!("invalid account id '{account_id}': {e}")))?;
            let value = amount_str
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{amount_str}': {e}")))?;
            let amount = Amount::new(value, CommodityCode::new(commodity));
            let cost = parse_cost(cost_val, cost_com, cost_dt, cost_lbl)?;
            let p_tag_ids = posting_tags_map.remove(&pid).unwrap_or_default();
            let posting = Posting::builder()
                .id(posting_id)
                .account_id(acc_id)
                .amount(amount)
                .maybe_cost(cost)
                .maybe_memo(memo)
                .tag_ids(p_tag_ids)
                .build();
            postings_by_tx.entry(tx_id_str).or_default().push(posting);
        }

        tx_rows
            .into_iter()
            .map(
                |(id_str, date_str, payee, description, status_str, created_at_str)| {
                    let tx_id = id_str
                        .parse::<TransactionId>()
                        .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;
                    let date = date_str
                        .parse::<jiff::civil::Date>()
                        .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                    let status = from_db_str::<TransactionStatus>(&status_str)?;
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
                        .status(status)
                        .tag_ids(tag_ids)
                        .created_at(created_at)
                        .build())
                },
            )
            .collect()
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
    /// Returns [`BcError::AlreadyVoided`] if the transaction exists but is already voided.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "complex atomic operation: event + UPDATE + FK-ordered DELETE + INSERT"
    )]
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
        let voided_str = crate::db::to_db_str(bc_models::TransactionStatus::Voided)?;

        let date_str = updated.date().to_string();

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        let result = sqlx::query(
            "UPDATE transactions SET date = ?, payee = ?, description = ? WHERE id = ? AND status != ?",
        )
        .bind(&date_str)
        .bind(updated.payee())
        .bind(updated.description())
        .bind(&tx_id_str)
        .bind(&voided_str)
        .execute(&mut *db_tx)
        .await?;

        if result.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT count(*) > 0 FROM transactions WHERE id = ?")
                    .bind(&tx_id_str)
                    .fetch_one(&mut *db_tx)
                    .await?;
            return if exists {
                Err(BcError::AlreadyVoided(tx_id.clone()))
            } else {
                Err(BcError::NotFound(tx_id_str))
            };
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
                 (id, transaction_id, account_id, amount, commodity, memo, position, \
                  cost_total_value, cost_total_commodity, cost_date, cost_label) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(posting.id().to_string())
            .bind(&tx_id_str)
            .bind(posting.account_id().to_string())
            .bind(posting.amount().value().to_string())
            .bind(posting.amount().commodity().as_str())
            .bind(posting.memo())
            .bind(
                i64::try_from(position)
                    .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?,
            )
            .bind(cost_value)
            .bind(cost_commodity)
            .bind(cost_date)
            .bind(cost_label)
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
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionStatus;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    fn make_balanced_transaction(acc_a: AccountId, acc_b: AccountId) -> Transaction {
        use jiff::Timestamp;
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
            .status(TransactionStatus::Cleared)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_balanced_transaction_succeeds(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "Income",
                AccountType::Income,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create Income account should succeed");
        let acc_b = acct_svc
            .create(
                "Checking",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
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
            .status(TransactionStatus::Cleared)
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
            .create(
                "Brokerage",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create Brokerage account should succeed");
        let acc_b = acct_svc
            .create(
                "Cash",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
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
            .status(TransactionStatus::Cleared)
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
    async fn void_nonexistent_transaction_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::TransactionId::new();
        let result = svc.void(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
        // Verify the failed void did not leave any orphaned events.
        let store = crate::events::SqliteStore::new(pool.clone());
        let events = store
            .replay_for(&fake_id.to_string())
            .await
            .expect("replay should succeed");
        assert!(
            events.is_empty(),
            "failed void must not leave events in the log"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn void_already_voided_returns_already_voided(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "A",
                bc_models::AccountType::Asset,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create(
                "B",
                bc_models::AccountType::Expense,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create B should succeed");

        let svc = Service::new(pool.clone());
        let tx = make_balanced_transaction(acc_a, acc_b);
        let id = tx.id().clone();
        svc.create(tx).await.expect("create should succeed");
        svc.void(&id).await.expect("first void should succeed");
        let result = svc.void(&id).await;
        assert!(matches!(result, Err(BcError::AlreadyVoided(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_excludes_voided_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "A",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create(
                "B",
                AccountType::Expense,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create B should succeed");

        let svc = Service::new(pool.clone());

        // Create two transactions; void one.
        let tx1 = make_balanced_transaction(acc_a.clone(), acc_b.clone());
        let id1 = tx1.id().clone();
        svc.create(tx1).await.expect("create tx1 should succeed");

        let tx2 = make_balanced_transaction(acc_a, acc_b);
        let id2 = tx2.id().clone();
        svc.create(tx2).await.expect("create tx2 should succeed");
        svc.void(&id2).await.expect("void tx2 should succeed");

        let active = svc.list().await.expect("list should succeed");
        assert_eq!(active.len(), 1);
        let first = active.first().expect("one active transaction should exist");
        assert_eq!(first.id(), &id1);
        assert_eq!(first.postings().len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amend_updates_projection(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account_svc = crate::AccountService::new(pool.clone());

        // Create two accounts so FK constraints pass.
        let checking_id = account_svc
            .create(
                "Checking",
                bc_models::AccountType::Asset,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create checking");
        let expenses_id = account_svc
            .create(
                "Expenses",
                bc_models::AccountType::Expense,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create expenses");

        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date("2026-01-01".parse::<jiff::civil::Date>().expect("date"))
            .description("Original description")
            .status(TransactionStatus::Cleared)
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
            .status(TransactionStatus::Cleared)
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
    async fn amend_voided_transaction_returns_already_voided(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let account_svc = crate::AccountService::new(pool.clone());

        let checking_id = account_svc
            .create(
                "Checking",
                bc_models::AccountType::Asset,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create checking");
        let expenses_id = account_svc
            .create(
                "Expenses",
                bc_models::AccountType::Expense,
                bc_models::AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create expenses");

        let original = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date("2026-01-01".parse::<jiff::civil::Date>().expect("date"))
            .description("Original description")
            .status(TransactionStatus::Cleared)
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
        svc.void(&id).await.expect("void should succeed");

        let amended = Transaction::builder()
            .id(id.clone())
            .date("2026-01-15".parse::<jiff::civil::Date>().expect("date"))
            .description("Amended after void")
            .status(TransactionStatus::Cleared)
            .postings(original.postings().to_vec())
            .created_at(*original.created_at())
            .build();

        let result = svc.amend(amended).await;
        assert!(matches!(result, Err(BcError::AlreadyVoided(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_tag_ids_round_trip(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "A",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create A should succeed");
        let acc_b = acct_svc
            .create(
                "B",
                AccountType::Expense,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
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
            .status(TransactionStatus::Cleared)
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
            .create(
                "Income",
                AccountType::Income,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create Income account should succeed");
        let acc_b = acct_svc
            .create(
                "Checking",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
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
            .status(TransactionStatus::Cleared)
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
            .status(TransactionStatus::Cleared)
            .created_at(Timestamp::now())
            .build();

        svc.amend(updated)
            .await
            .expect("amend should succeed despite posting_tags FK");

        let found = svc.find_by_id(&tx_id).await.expect("find should succeed");
        assert_eq!(found.description(), "Amended description");
    }
}
