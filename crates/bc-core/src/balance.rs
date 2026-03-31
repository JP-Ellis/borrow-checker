//! Balance calculation engine.

use bc_models::AccountId;
use bc_models::TransactionStatus;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::to_db_str;

/// Calculates account balances from the `postings` projection table.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Engine {
    /// Creates a [`Engine`] with the given connection pool.
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
        let voided_str = to_db_str(TransactionStatus::Voided)?;

        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT p.amount
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.account_id = ?
               AND p.commodity  = ?
               AND t.status     != ?",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().try_fold(Decimal::ZERO, |acc, (amt,)| {
            let d = amt
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid decimal amount '{amt}': {e}")))?;
            acc.checked_add(d).ok_or_else(|| {
                BcError::BadData("balance overflow: sum exceeds Decimal range".into())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_reflects_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "Wallet",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
                None,
                None,
                None,
            )
            .await
            .expect("create Wallet account should succeed");
        let acc_b = acct_svc
            .create(
                "Income",
                AccountType::Income,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
                None,
                None,
                None,
            )
            .await
            .expect("create Income account should succeed");

        // Insert a transaction directly for simplicity
        sqlx::query("INSERT INTO transactions (id, date, description, status, created_at) VALUES ('tx_1', '2026-01-01', 'Test', 'cleared', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p1', 'tx_1', ?, '100.00', 'AUD', 0)")
            .bind(acc_a.to_string()).execute(&pool).await.expect("insert posting p1 should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p2', 'tx_1', ?, '-100.00', 'AUD', 1)")
            .bind(acc_b.to_string()).execute(&pool).await.expect("insert posting p2 should succeed");

        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(balance, dec!(100.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_zero_for_account_with_no_postings(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create(
                "Empty",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
                None,
                None,
                None,
            )
            .await
            .expect("create should succeed");
        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(balance, Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_excludes_voided_transactions(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Posting;
        use bc_models::PostingId;
        use bc_models::Transaction;
        use bc_models::TransactionStatus;
        use jiff::civil::date;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create(
                "Wallet",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
                None,
                None,
                None,
            )
            .await
            .expect("create Wallet should succeed");
        let acc_b = acct_svc
            .create(
                "Income",
                AccountType::Income,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
                None,
                None,
                None,
            )
            .await
            .expect("create Income should succeed");

        let tx_svc = crate::transaction::Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 1))
            .description("Voided")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .status(TransactionStatus::Cleared)
            .created_at(jiff::Timestamp::now())
            .build();
        let tx_id = tx.id().clone();
        tx_svc.create(tx).await.expect("create should succeed");
        tx_svc.void(&tx_id).await.expect("void should succeed");

        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(
            balance,
            Decimal::ZERO,
            "voided transaction should not affect balance"
        );
    }
}
