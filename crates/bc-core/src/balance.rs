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

    /// Computes total net worth in `commodity` across all asset and liability accounts.
    ///
    /// - [`DepositAccount`], [`Receivable`], [`VirtualAllocation`]: balance from postings.
    /// - [`ManualAsset`]: latest recorded market value from `asset_valuations`.
    /// - Accounts with `AccountType` other than `Asset`/`Liability` are excluded.
    ///
    /// Returns `Decimal::ZERO` if no relevant accounts exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    ///
    /// [`DepositAccount`]: bc_models::AccountKind::DepositAccount
    /// [`Receivable`]: bc_models::AccountKind::Receivable
    /// [`VirtualAllocation`]: bc_models::AccountKind::VirtualAllocation
    /// [`ManualAsset`]: bc_models::AccountKind::ManualAsset
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "intentional fallback with warning for future AccountKind variants"
    )]
    #[inline]
    pub async fn net_worth(&self, commodity: &str) -> BcResult<Decimal> {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        // Load all active asset + liability accounts.
        let accounts = crate::account::Service::new(self.pool.clone())
            .list_active()
            .await?;

        let mut total = Decimal::ZERO;
        let asset_svc = crate::asset::Service::new(self.pool.clone());

        for account in &accounts {
            match account.account_type() {
                AccountType::Asset | AccountType::Liability => {}
                _ => continue,
            }

            let contribution = match account.kind() {
                AccountKind::ManualAsset => {
                    // Use latest recorded market value, not posting-based balance.
                    asset_svc
                        .latest_market_value(account.id(), commodity)
                        .await?
                        .unwrap_or(Decimal::ZERO)
                }
                AccountKind::DepositAccount
                | AccountKind::Receivable
                | AccountKind::VirtualAllocation => {
                    self.balance_for(account.id(), commodity).await?
                }
                _ => {
                    tracing::warn!(
                        account_id = %account.id(),
                        kind = ?account.kind(),
                        "unknown AccountKind in net_worth; using posting-based balance"
                    );
                    self.balance_for(account.id(), commodity).await?
                }
            };

            total = total
                .checked_add(contribution)
                .ok_or_else(|| BcError::BadData("net worth overflow".into()))?;
        }

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_reflects_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet account should succeed");
        let acc_b = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
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
            .create()
            .name("Empty")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
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
    async fn net_worth_includes_manual_asset_valuation(pool: sqlx::SqlitePool) {
        use bc_models::ValuationSource;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());

        // A ManualAsset with a recorded valuation.
        let house_id = acct_svc
            .create()
            .name("House")
            .account_type(AccountType::Asset)
            .kind(AccountKind::ManualAsset)
            .acquisition_date(jiff::civil::date(2020, 1, 1))
            .acquisition_cost(dec!(500_000))
            .call()
            .await
            .expect("create ManualAsset");

        // A DepositAccount with a posting-based balance.
        let savings_id = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create DepositAccount");

        // Give the savings account a balance via a direct insert.
        let income_id = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income");
        sqlx::query("INSERT INTO transactions (id, date, description, status, created_at) VALUES ('tx_nw1', '2026-01-01', 'Test', 'cleared', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("tx insert");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_nw1', 'tx_nw1', ?, '50000.00', 'AUD', 0)")
            .bind(savings_id.to_string()).execute(&pool).await.expect("posting insert");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_nw2', 'tx_nw1', ?, '-50000.00', 'AUD', 1)")
            .bind(income_id.to_string()).execute(&pool).await.expect("posting insert 2");

        // Record a valuation for the house.
        let asset_svc = crate::asset::Service::new(pool.clone());
        asset_svc
            .record_valuation(
                &house_id,
                dec!(650_000),
                "AUD",
                ValuationSource::ProfessionalAppraisal,
                jiff::civil::date(2026, 3, 1),
                None,
            )
            .await
            .expect("record valuation");

        let engine = Engine::new(pool.clone());
        let net_worth = engine.net_worth("AUD").await.expect("net worth");

        // Expected: savings (50_000) + house valuation (650_000) = 700_000
        // (Income account is excluded from net worth as it's not Asset/Liability)
        assert_eq!(net_worth, dec!(700_000));
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
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");
        let acc_b = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
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
