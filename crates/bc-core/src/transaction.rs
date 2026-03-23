//! Transaction service with double-entry validation.

use bc_models::{
    EventId, Posting, Transaction, TransactionStatus,
    ids::{AccountId, PostingId, TransactionId},
    money::{Amount, CommodityCode},
};
use jiff::Timestamp;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::{
    error::{BcError, BcResult},
    events::Event,
};

/// Converts a [`TransactionStatus`] to its canonical database string.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `s` is an unrecognised variant (future-proofing for
/// `#[non_exhaustive]` additions).
fn status_to_str(s: TransactionStatus) -> BcResult<&'static str> {
    match s {
        TransactionStatus::Pending => Ok("pending"),
        TransactionStatus::Cleared => Ok("cleared"),
        TransactionStatus::Voided => Ok("voided"),
        _ => Err(BcError::BadData(format!(
            "unknown transaction status: {s:?}"
        ))),
    }
}

/// Parses a [`TransactionStatus`] from its canonical database string.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `s` is not a recognised status string.
fn status_from_str(s: &str) -> BcResult<TransactionStatus> {
    match s {
        "pending" => Ok(TransactionStatus::Pending),
        "cleared" => Ok(TransactionStatus::Cleared),
        "voided" => Ok(TransactionStatus::Voided),
        other => Err(BcError::BadData(format!("unknown status: {other}"))),
    }
}

/// Validates that the postings in a transaction sum to zero per commodity.
#[expect(
    clippy::std_instead_of_alloc,
    reason = "this crate is std-based; alloc is not separately available"
)]
fn validate_balance(postings: &[Posting]) -> BcResult<()> {
    if postings.is_empty() {
        return Err(BcError::UnbalancedTransaction);
    }

    let mut sums: std::collections::BTreeMap<&str, Decimal> = std::collections::BTreeMap::new();
    for p in postings {
        let entry: &mut Decimal = sums.entry(p.amount.commodity.as_str()).or_default();
        *entry = entry.saturating_add(p.amount.value);
    }
    for (commodity, sum) in &sums {
        if !sum.is_zero() {
            tracing::warn!(%commodity, %sum, "transaction postings do not balance");
            return Err(BcError::UnbalancedTransaction);
        }
    }
    Ok(())
}

/// Service for creating and managing transactions.
#[expect(
    clippy::module_name_repetitions,
    reason = "TransactionService is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone)]
pub struct TransactionService {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl TransactionService {
    /// Creates a new [`TransactionService`] with the given connection pool.
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
        validate_balance(&tx.postings)?;

        let tx_id = tx.id.clone();
        let event_id = EventId::new().to_string();
        let event = Event::TransactionCreated { id: tx_id.clone() };
        let payload = serde_json::to_string(&event)?;
        let now = Timestamp::now();

        let tags_json = serde_json::to_string(&tx.tags)?;
        let date_str = tx.date.to_string();
        let created_at_str = tx.created_at.to_string();

        let mut db_tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(event.kind())
        .bind(tx_id.to_string())
        .bind(&payload)
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        sqlx::query(
            "INSERT INTO transactions (id, date, payee, description, status, tags, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(tx_id.to_string())
        .bind(&date_str)
        .bind(&tx.payee)
        .bind(&tx.description)
        .bind(status_to_str(tx.status)?)
        .bind(&tags_json)
        .bind(&created_at_str)
        .execute(&mut *db_tx)
        .await?;

        for (position, posting) in tx.postings.iter().enumerate() {
            sqlx::query(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, memo, position) VALUES (?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(posting.id.to_string())
            .bind(tx_id.to_string())
            .bind(posting.account_id.to_string())
            .bind(posting.amount.value.to_string())
            .bind(posting.amount.commodity.as_str())
            .bind(posting.memo.as_deref())
            .bind(i64::try_from(position).unwrap_or(i64::MAX))
            .execute(&mut *db_tx)
            .await?;
        }

        db_tx.commit().await?;
        tracing::info!(transaction_id = %tx_id, "transaction created");
        Ok(tx_id)
    }

    /// Finds a transaction by ID, including all its postings.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn find_by_id(&self, id: &TransactionId) -> BcResult<Transaction> {
        let maybe_tx_row = sqlx::query_as::<_, (String, String, Option<String>, String, String, String, String)>(
            "SELECT id, date, payee, description, status, tags, created_at FROM transactions WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let tx_row = maybe_tx_row.ok_or_else(|| BcError::NotFound(id.to_string()))?;

        let tx_id = tx_row
            .0
            .parse::<TransactionId>()
            .map_err(|e| BcError::BadData(format!("invalid transaction id: {e}")))?;

        let date = tx_row
            .1
            .parse::<jiff::civil::Date>()
            .map_err(|e| BcError::BadData(format!("invalid date '{}': {e}", tx_row.1)))?;

        let status = status_from_str(&tx_row.4)?;

        let tags: Vec<String> = serde_json::from_str(&tx_row.5)
            .map_err(|e| BcError::BadData(format!("invalid tags '{}': {e}", tx_row.5)))?;

        let created_at = tx_row
            .6
            .parse::<Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at '{}': {e}", tx_row.6)))?;

        // Fetch postings
        let posting_rows = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT id, account_id, amount, commodity, memo FROM postings WHERE transaction_id = ? ORDER BY position ASC"
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let postings = posting_rows
            .into_iter()
            .map(|(pid, account_id, amount_str, commodity, memo)| {
                let posting_id = pid
                    .parse::<PostingId>()
                    .map_err(|e| BcError::BadData(format!("invalid posting id '{pid}': {e}")))?;
                let acc_id = account_id.parse::<AccountId>().map_err(|e| {
                    BcError::BadData(format!("invalid account id '{account_id}': {e}"))
                })?;
                let value = amount_str
                    .parse::<Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid amount '{amount_str}': {e}")))?;
                let amount = Amount::new(value, CommodityCode::new(commodity));
                Ok(Posting::new(posting_id, acc_id, amount, memo))
            })
            .collect::<BcResult<Vec<_>>>()?;

        Ok(Transaction::new(
            tx_id, date, tx_row.2, tx_row.3, postings, status, tags, created_at,
        ))
    }

    /// Voids a transaction by setting its status to `voided`.
    ///
    /// The existence check happens before any write.  The event append and the
    /// projection update are wrapped in a single SQLite transaction so they
    /// succeed or fail atomically.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no transaction with that ID exists or is already voided.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn void(&self, id: &TransactionId) -> BcResult<()> {
        let now = Timestamp::now();

        // Check existence first (before writing any event).
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM transactions WHERE id = ? AND status != 'voided'")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        if exists.is_none() {
            return Err(BcError::NotFound(id.to_string()));
        }

        let event_id = EventId::new().to_string();
        let event = Event::TransactionVoided { id: id.clone() };
        let payload = serde_json::to_string(&event)?;

        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(event.kind())
        .bind(id.to_string())
        .bind(&payload)
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE transactions SET status = 'voided' WHERE id = ? AND status != 'voided'",
        )
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        tracing::info!(transaction_id = %id, "transaction voided");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bc_models::{
        AccountType, Posting, Transaction, TransactionStatus,
        ids::{AccountId, PostingId},
        money::{Amount, CommodityCode},
    };
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    fn make_balanced_transaction(acc_a: AccountId, acc_b: AccountId) -> Transaction {
        use jiff::Timestamp;
        Transaction::new(
            bc_models::TransactionId::new(),
            date(2026, 1, 15),
            None,
            "Test".to_owned(),
            vec![
                Posting::new(
                    PostingId::new(),
                    acc_a,
                    Amount::new(dec!(100.00), CommodityCode::new("AUD")),
                    None,
                ),
                Posting::new(
                    PostingId::new(),
                    acc_b,
                    Amount::new(dec!(-100.00), CommodityCode::new("AUD")),
                    None,
                ),
            ],
            TransactionStatus::Cleared,
            vec![],
            Timestamp::now(),
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_balanced_transaction_succeeds(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "Income",
                AccountType::Income,
                CommodityCode::new("AUD"),
                None,
            )
            .await
            .expect("create Income account should succeed");
        let acc_b = acct_svc
            .create(
                "Checking",
                AccountType::Asset,
                CommodityCode::new("AUD"),
                None,
            )
            .await
            .expect("create Checking account should succeed");

        let svc = TransactionService::new(pool.clone());
        let tx = make_balanced_transaction(acc_a, acc_b);
        let id = tx.id.clone();
        svc.create(tx)
            .await
            .expect("balanced transaction should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.postings.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_unbalanced_transaction_fails(pool: sqlx::SqlitePool) {
        use jiff::Timestamp;
        let svc = TransactionService::new(pool.clone());
        let tx = Transaction::new(
            bc_models::TransactionId::new(),
            date(2026, 1, 15),
            None,
            "Unbalanced".to_owned(),
            vec![Posting::new(
                PostingId::new(),
                AccountId::new(),
                Amount::new(dec!(50.00), CommodityCode::new("AUD")),
                None,
            )],
            TransactionStatus::Cleared,
            vec![],
            Timestamp::now(),
        );
        let result = svc.create(tx).await;
        assert!(matches!(result, Err(BcError::UnbalancedTransaction)));
    }
}
