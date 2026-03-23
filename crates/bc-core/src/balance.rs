//! Balance calculation engine.

use bc_models::AccountId;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::error::{BcError, BcResult};

/// Calculates account balances from the `postings` projection table.
#[expect(
    clippy::module_name_repetitions,
    reason = "BalanceEngine is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone)]
pub struct BalanceEngine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl BalanceEngine {
    /// Creates a [`BalanceEngine`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns the running balance for `account_id` in `commodity`.
    ///
    /// Returns [`Decimal::ZERO`] if no postings exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a stored amount cannot be parsed.
    #[inline]
    pub async fn balance_for(&self, account_id: &AccountId, commodity: &str) -> BcResult<Decimal> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT p.amount
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.account_id = ?
               AND p.commodity   = ?
               AND t.status     != 'voided'",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().try_fold(Decimal::ZERO, |acc, (amt,)| {
            amt.parse::<Decimal>()
                .map(|d| acc.saturating_add(d))
                .map_err(|e| BcError::BadData(format!("invalid decimal amount '{amt}': {e}")))
        })
    }
}

#[cfg(test)]
mod tests {
    use bc_models::{AccountType, CommodityCode};
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_reflects_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::AccountService::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "Wallet",
                AccountType::Asset,
                CommodityCode::new("AUD"),
                None,
            )
            .await
            .expect("create Wallet account should succeed");
        let acc_b = acct_svc
            .create(
                "Income",
                AccountType::Income,
                CommodityCode::new("AUD"),
                None,
            )
            .await
            .expect("create Income account should succeed");

        // Insert a transaction directly for simplicity
        sqlx::query("INSERT INTO transactions (id, date, description, status, tags, created_at) VALUES ('tx_1', '2026-01-01', 'Test', 'cleared', '[]', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p1', 'tx_1', ?, '100.00', 'AUD', 0)")
            .bind(acc_a.to_string()).execute(&pool).await.expect("insert posting p1 should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p2', 'tx_1', ?, '-100.00', 'AUD', 1)")
            .bind(acc_b.to_string()).execute(&pool).await.expect("insert posting p2 should succeed");

        let engine = BalanceEngine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(balance, dec!(100.00));
    }
}
