//! Balance calculation engine.

use bc_models::AccountId;
use bc_models::TransactionStatus;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::to_db_str;

/// A single time-bucket of posting aggregation data.
///
/// Used for sparklines and period-based cash-flow summaries.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostingBucket {
    /// Inclusive start of the bucket period.
    pub start: jiff::civil::Date,
    /// Exclusive end of the bucket period (= start of the next bucket).
    pub end: jiff::civil::Date,
    /// Sum of positive postings (money entering the account) in this period.
    pub inflow: rust_decimal::Decimal,
    /// Sum of absolute negative postings (money leaving the account) in this period.
    pub outflow: rust_decimal::Decimal,
}

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

    /// Fetches all non-voided postings for `account_id` in `commodity` within `[from, to)`.
    ///
    /// Returns `(transaction_date, amount)` pairs — both parsed from their stored strings.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `from`       - Inclusive start date.
    /// * `to`         - Exclusive end date.
    ///
    /// # Returns
    ///
    /// A vector of `(date, amount)` pairs for all matching postings.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    async fn fetch_postings_in_range(
        &self,
        account_id: &AccountId,
        commodity: &str,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
    ) -> BcResult<Vec<(jiff::civil::Date, Decimal)>> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT t.date, p.amount
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.account_id = ?
               AND p.commodity  = ?
               AND t.date >= ?
               AND t.date  < ?
               AND t.status != ?",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .bind(from.to_string())
        .bind(to.to_string())
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(date_str, amt_str)| {
                let date = date_str
                    .parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                let amount = amt_str
                    .parse::<Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid amount '{amt_str}': {e}")))?;
                Ok((date, amount))
            })
            .collect()
    }

    /// Returns the total inflow and outflow for `account_id` in `commodity` over `[from, to)`.
    ///
    /// - `inflow` — sum of all positive postings (money entering the account).
    /// - `outflow` — absolute sum of all negative postings (money leaving).
    ///
    /// Both values are non-negative. Voided transactions are excluded.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `from`       - Inclusive start date.
    /// * `to`         - Exclusive end date.
    ///
    /// # Returns
    ///
    /// `(inflow, outflow)` as [`Decimal`] values.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_flows(
        &self,
        account_id: &AccountId,
        commodity: &str,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
    ) -> BcResult<(Decimal, Decimal)> {
        let rows = self
            .fetch_postings_in_range(account_id, commodity, from, to)
            .await?;

        rows.into_iter().try_fold(
            (Decimal::ZERO, Decimal::ZERO),
            |(inflow, outflow), (_, amount)| {
                if amount >= Decimal::ZERO {
                    let new_inflow = inflow.checked_add(amount).ok_or_else(|| {
                        BcError::BadData("inflow overflow: sum exceeds Decimal range".into())
                    })?;
                    Ok((new_inflow, outflow))
                } else {
                    let new_outflow = outflow.checked_sub(amount).ok_or_else(|| {
                        BcError::BadData("outflow overflow: sum exceeds Decimal range".into())
                    })?;
                    Ok((inflow, new_outflow))
                }
            },
        )
    }

    /// Returns `count` contiguous period buckets ending with the period containing `as_of`.
    ///
    /// Buckets are returned oldest-first. Each bucket covers exactly one `period`
    /// length. Postings are fetched in a single query and assigned to buckets in Rust.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `period`     - Bucket width. Use [`bc_models::Period::Monthly`] for a 6-month sparkline.
    /// * `count`      - Number of buckets to return.
    /// * `as_of`      - Reference date; the most recent bucket contains this date.
    ///
    /// # Returns
    ///
    /// [`Vec<PostingBucket>`] ordered oldest-first. Length equals `count`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_buckets(
        &self,
        account_id: &AccountId,
        commodity: &str,
        period: &bc_models::Period,
        count: core::num::NonZeroUsize,
        as_of: jiff::civil::Date,
    ) -> BcResult<Vec<PostingBucket>> {
        // Build bucket boundaries, going backward from as_of.
        let mut ranges: Vec<(jiff::civil::Date, jiff::civil::Date)> =
            Vec::with_capacity(count.get());
        let current = period.range_containing(as_of);
        ranges.push(current);
        let mut prev_start = current.0;
        for _ in 1..count.get() {
            let prev =
                period.range_containing(prev_start.saturating_sub(jiff::Span::new().days(1_i32)));
            ranges.push(prev);
            prev_start = prev.0;
        }
        ranges.reverse(); // oldest first

        // ranges is non-empty: count is NonZeroUsize so we always push at least one entry.
        let Some(&(earliest_start, _)) = ranges.first() else {
            return Ok(vec![]);
        };
        let Some(&(_, latest_end)) = ranges.last() else {
            return Ok(vec![]);
        };

        // One query for all postings across the full range.
        let all_postings = self
            .fetch_postings_in_range(account_id, commodity, earliest_start, latest_end)
            .await?;

        // Distribute postings into buckets.
        let mut buckets: Vec<PostingBucket> = ranges
            .into_iter()
            .map(|(start, end)| PostingBucket {
                start,
                end,
                inflow: rust_decimal::Decimal::ZERO,
                outflow: rust_decimal::Decimal::ZERO,
            })
            .collect();

        for (date, amount) in all_postings {
            if let Some(bucket) = buckets.iter_mut().find(|b| date >= b.start && date < b.end) {
                if amount >= rust_decimal::Decimal::ZERO {
                    bucket.inflow = bucket
                        .inflow
                        .checked_add(amount)
                        .ok_or_else(|| BcError::BadData("inflow overflow".into()))?;
                } else {
                    bucket.outflow = bucket
                        .outflow
                        .checked_sub(amount)
                        .ok_or_else(|| BcError::BadData("outflow overflow".into()))?;
                }
            }
        }

        Ok(buckets)
    }

    /// Returns the commodity code of the first (default) commodity for `account_id`, or `None`.
    ///
    /// Prefers the configured default from `account_commodities` (position = 0). When no
    /// commodity is configured, falls back to the most-used posting commodity so that
    /// accounts imported without explicit commodity setup still return a useful value.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    #[inline]
    pub async fn default_commodity_for(&self, account_id: &AccountId) -> BcResult<Option<String>> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;
        let (commodity_code,): (Option<String>,) = sqlx::query_as(
            "SELECT COALESCE(
                 (SELECT c.code
                  FROM account_commodities ac
                  JOIN commodities c ON c.id = ac.commodity_id
                  WHERE ac.account_id = ?
                  ORDER BY ac.position
                  LIMIT 1),
                 (SELECT p.commodity
                  FROM postings p
                  JOIN transactions t ON t.id = p.transaction_id
                  WHERE p.account_id = ? AND t.status != ?
                  GROUP BY p.commodity
                  ORDER BY COUNT(*) DESC
                  LIMIT 1)
             ) AS commodity_code",
        )
        .bind(account_id.to_string())
        .bind(account_id.to_string())
        .bind(&voided_str)
        .fetch_one(&self.pool)
        .await?;
        Ok(commodity_code)
    }

    /// Returns the count of non-voided postings for `account_id`.
    ///
    /// Previously this counted postings without an envelope tag; with the
    /// budget model transition (budgets are now account-anchored, not
    /// posting-tagged) all non-voided postings are returned instead.
    /// Voided transactions are excluded from the count.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    ///
    /// # Returns
    ///
    /// The number of non-voided postings as a [`u32`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure or if the database driver returns a
    /// negative `COUNT(*)` value (which violates the SQLite invariant but is handled
    /// defensively).
    #[inline]
    pub async fn uncategorised_count(&self, account_id: &AccountId) -> BcResult<u32> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*)
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.account_id = ?
               AND t.status != ?",
        )
        .bind(account_id.to_string())
        .bind(&voided_str)
        .fetch_one(&self.pool)
        .await?;

        // COUNT(*) is always non-negative; a negative result would indicate a
        // database driver bug or schema mismatch, so we surface it as an error
        // rather than silently returning 0.
        u32::try_from(count).map_err(|_e| {
            BcError::BadData(format!(
                "COUNT(*) returned negative value {count}; SQLite invariant violated"
            ))
        })
    }

    /// Returns the default-commodity balance for every active account in one query.
    ///
    /// Balances are computed live from non-voided postings, not from the `balances`
    /// cache table (which is a write-through cache not yet populated by the application).
    ///
    /// The map key is [`AccountId`]; the value is `(commodity_code, balance)`.
    /// Accounts with neither a configured commodity nor any postings are omitted.
    /// Accounts with a commodity (configured or inferred) but no postings are included with a zero
    /// balance.
    ///
    /// The commodity for each account is resolved in priority order:
    /// 1. The configured default from `account_commodities` (position = 0).
    /// 2. The most-used posting commodity (for accounts imported without explicit commodity setup).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn default_balances(
        &self,
    ) -> BcResult<std::collections::HashMap<AccountId, (String, Decimal)>> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;

        // Fetch the effective default commodity per active account.
        // Prefers account_commodities (position = 0); falls back to the most-used posting
        // commodity so accounts imported without explicit commodity setup are still included.
        let commodity_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, commodity_code FROM (
                 SELECT a.id,
                        COALESCE(
                            c.code,
                            (SELECT p.commodity
                             FROM postings p
                             JOIN transactions t ON t.id = p.transaction_id
                             WHERE p.account_id = a.id AND t.status != ?
                             GROUP BY p.commodity
                             ORDER BY COUNT(*) DESC
                             LIMIT 1)
                        ) AS commodity_code
                 FROM accounts a
                 LEFT JOIN account_commodities ac ON ac.account_id = a.id AND ac.position = 0
                 LEFT JOIN commodities c ON c.id = ac.commodity_id
                 WHERE a.archived_at IS NULL
             )
             WHERE commodity_code IS NOT NULL",
        )
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        if commodity_rows.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        // Fetch all non-voided postings for those accounts (one query, filtered in Rust).
        let posting_rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT p.account_id, p.commodity, p.amount
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             JOIN accounts a ON a.id = p.account_id
             WHERE a.archived_at IS NULL
               AND t.status != ?",
        )
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        // Build commodity lookup: account_id_str → commodity_code.
        let commodity_by_account: std::collections::HashMap<String, String> = commodity_rows
            .iter()
            .map(|(id, code)| (id.clone(), code.clone()))
            .collect();

        // Sum posting amounts per account for that account's default commodity.
        let mut map: std::collections::HashMap<String, Decimal> = commodity_by_account
            .keys()
            .map(|id| (id.clone(), Decimal::ZERO))
            .collect();

        for (acc_id, commodity, amt_str) in &posting_rows {
            let Some(default_commodity) = commodity_by_account.get(acc_id) else {
                continue; // no default commodity — skip
            };
            if commodity != default_commodity {
                continue; // posting is in a non-default commodity — skip
            }
            let amount = amt_str.parse::<Decimal>().map_err(|e| {
                BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
            })?;
            let entry = map.entry(acc_id.clone()).or_insert(Decimal::ZERO);
            *entry = entry.checked_add(amount).ok_or_else(|| {
                BcError::BadData("balance overflow: sum exceeds Decimal range".into())
            })?;
        }

        // Convert string IDs to AccountId.
        map.into_iter()
            .map(|(id_str, balance)| {
                // id_str was inserted from commodity_by_account keys, so the lookup cannot miss.
                let commodity = commodity_by_account
                    .get(&id_str)
                    .ok_or_else(|| {
                        BcError::BadData(format!("commodity lookup missing for account '{id_str}'"))
                    })?
                    .clone();
                let id = id_str
                    .parse::<AccountId>()
                    .map_err(|e| BcError::BadData(format!("invalid account id '{id_str}': {e}")))?;
                Ok((id, (commodity, balance)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Transaction;
    use jiff::civil::date;
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
    async fn default_balances_returns_real_balance(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use rust_decimal_macros::dec;

        // Create commodity
        sqlx::query("INSERT INTO commodities (id, code) VALUES ('com_aud', 'AUD')")
            .execute(&pool)
            .await
            .expect("insert commodity");

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        // Wire the commodity to the account
        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position)
             VALUES (?, 'com_aud', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("link commodity");

        // Seed via a real transaction + posting
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('t1', '2026-01-01', 'Test', 'cleared', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('p1', 't1', ?, '1234.56', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("insert posting");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        let (code, bal) = map.get(&acc).expect("account should be in map");
        assert_eq!(code.as_str(), "AUD");
        assert_eq!(*bal, dec!(1234.56));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_zero_for_account_without_balance_row(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        sqlx::query("INSERT INTO commodities (id, code) VALUES ('com_aud2', 'AUD')")
            .execute(&pool)
            .await
            .expect("insert commodity");

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Empty")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position)
             VALUES (?, 'com_aud2', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("link commodity");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        let (code, bal) = map.get(&acc).expect("account in map");
        assert_eq!(code.as_str(), "AUD");
        assert_eq!(*bal, rust_decimal::Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_omits_account_without_commodity(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("NoCommodity")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        // Account has no commodity → not included in the map
        assert!(!map.contains_key(&acc));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_splits_inflow_and_outflow(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");
        let income = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income");

        // Two cleared transactions within the range
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tf1', '2026-04-10', 'Pay', 'cleared', '2026-04-10T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx 1");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pf1a', 'tf1', ?, '1000.00', 'AUD', 0),
                    ('pf1b', 'tf1', ?, '-1000.00', 'AUD', 1)",
        )
        .bind(wallet.to_string())
        .bind(income.to_string())
        .execute(&pool)
        .await
        .expect("postings 1");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tf2', '2026-04-20', 'Expense', 'cleared', '2026-04-20T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx 2");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pf2a', 'tf2', ?, '-250.00', 'AUD', 0),
                    ('pf2b', 'tf2', ?, '250.00', 'AUD', 1)",
        )
        .bind(wallet.to_string())
        .bind(income.to_string())
        .execute(&pool)
        .await
        .expect("postings 2");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        assert_eq!(inflow, dec!(1000.00));
        assert_eq!(outflow, dec!(250.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_excludes_voided_transactions(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tv1', '2026-04-15', 'Voided', 'voided', '2026-04-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("voided tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pv1', 'tv1', ?, '500.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        assert_eq!(inflow, rust_decimal::Decimal::ZERO);
        assert_eq!(outflow, rust_decimal::Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_respects_date_boundary(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");

        // Inside range
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tb1', '2026-04-01', 'In', 'cleared', '2026-04-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx in");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pb1', 'tb1', ?, '100.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting in");

        // On the exclusive upper bound — should be excluded
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tb2', '2026-05-01', 'Boundary', 'cleared', '2026-05-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx boundary");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pb2', 'tb2', ?, '999.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting boundary");

        let engine = Engine::new(pool.clone());
        let (inflow, _) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        // Only the 100.00 inside [2026-04-01, 2026-05-01) should be counted
        assert_eq!(inflow, dec!(100.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_buckets_returns_correct_count(pool: sqlx::SqlitePool) {
        use core::num::NonZeroUsize;

        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Period;
        use jiff::civil::date;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Test")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let engine = Engine::new(pool.clone());
        let buckets = engine
            .posting_buckets(
                &acc,
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(6).expect("6 > 0"),
                date(2026, 5, 25),
            )
            .await
            .expect("posting_buckets");

        // Should return exactly 6 monthly buckets, oldest first.
        // as_of = 2026-05-25 → current bucket = [2026-05-01, 2026-06-01)
        // Going back 5 more months: 2026-04, 2026-03, 2026-02, 2026-01, 2025-12
        assert_eq!(buckets.len(), 6);
        #[expect(
            clippy::indexing_slicing,
            reason = "test assertions on known-length vec"
        )]
        {
            assert_eq!(buckets[0].start.to_string(), "2025-12-01");
            assert_eq!(buckets[5].start.to_string(), "2026-05-01");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_buckets_assigns_postings_to_correct_bucket(pool: sqlx::SqlitePool) {
        use core::num::NonZeroUsize;

        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Period;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Test")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        // One inflow in April, one outflow in May
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tbk1', '2026-04-15', 'April pay', 'cleared', '2026-04-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx april");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pbk1', 'tbk1', ?, '500.00', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("posting april");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('tbk2', '2026-05-10', 'May rent', 'cleared', '2026-05-10T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx may");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pbk2', 'tbk2', ?, '-200.00', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("posting may");

        let engine = Engine::new(pool.clone());
        let buckets = engine
            .posting_buckets(
                &acc,
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(2).expect("2 > 0"),
                date(2026, 5, 25),
            )
            .await
            .expect("posting_buckets");

        assert_eq!(buckets.len(), 2);
        #[expect(
            clippy::indexing_slicing,
            reason = "test assertions on known-length vec"
        )]
        {
            // First bucket: April (inflow 500, outflow 0)
            assert_eq!(buckets[0].start.to_string(), "2026-04-01");
            assert_eq!(buckets[0].inflow, dec!(500.00));
            assert_eq!(buckets[0].outflow, rust_decimal::Decimal::ZERO);
            // Second bucket: May (inflow 0, outflow 200)
            assert_eq!(buckets[1].start.to_string(), "2026-05-01");
            assert_eq!(buckets[1].inflow, rust_decimal::Decimal::ZERO);
            assert_eq!(buckets[1].outflow, dec!(200.00));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_excludes_voided_transactions(pool: sqlx::SqlitePool) {
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

    #[sqlx::test(migrations = "./migrations")]
    async fn uncategorised_count_zero_for_account_with_no_postings(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Empty")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account should succeed");

        let engine = Engine::new(pool.clone());
        let count = engine
            .uncategorised_count(&acc)
            .await
            .expect("uncategorised_count should succeed");
        assert_eq!(count, 0, "account with no postings should have count 0");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn uncategorised_count_counts_null_envelope_postings(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Checking account should succeed");
        let other = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income account should succeed");

        // Two postings (budgets are now account-anchored, not posting-tagged)
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('uc_tx1', '2026-01-01', 'Deposit', 'cleared', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert transaction");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('uc_p1', 'uc_tx1', ?, '100.00', 'AUD', 0),
                    ('uc_p2', 'uc_tx1', ?, '-100.00', 'AUD', 1)",
        )
        .bind(acc.to_string())
        .bind(other.to_string())
        .execute(&pool)
        .await
        .expect("insert postings");

        let engine = Engine::new(pool.clone());
        let count = engine
            .uncategorised_count(&acc)
            .await
            .expect("uncategorised_count should succeed");
        // Only the posting for `acc` should be counted, not the `other` account posting
        assert_eq!(count, 1, "one posting counted for the queried account");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn uncategorised_count_excludes_categorised_postings(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Checking account should succeed");

        // With budget model, all non-voided postings are counted.
        // Two postings for this account.
        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('uc_cat_tx1', '2026-01-01', 'Groceries', 'cleared', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert transaction");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('uc_cat_p1', 'uc_cat_tx1', ?, '-50.00', 'AUD', 0),
                    ('uc_cat_p2', 'uc_cat_tx1', ?, '50.00', 'AUD', 1)",
        )
        .bind(acc.to_string())
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("insert postings");

        let engine = Engine::new(pool.clone());
        let count = engine
            .uncategorised_count(&acc)
            .await
            .expect("uncategorised_count should succeed");
        assert_eq!(count, 2, "both non-voided postings should be counted");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn uncategorised_count_mixed_categorised_and_uncategorised(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Checking account should succeed");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, status, created_at)
             VALUES ('mix_tx1', '2026-01-15', 'Partial', 'cleared', '2026-01-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert transaction");

        // Two postings for the same account (budgets are now account-anchored)
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('mix_p1', 'mix_tx1', ?, '-30.00', 'AUD', 0),
                    ('mix_p2', 'mix_tx1', ?, '-20.00', 'AUD', 1)",
        )
        .bind(acc.to_string())
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("insert postings");

        let engine = Engine::new(pool.clone());
        let count = engine
            .uncategorised_count(&acc)
            .await
            .expect("uncategorised_count should succeed");
        assert_eq!(count, 2, "both non-voided postings should be counted");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn uncategorised_count_excludes_voided_transactions(pool: sqlx::SqlitePool) {
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
        let count = engine
            .uncategorised_count(&acc_a)
            .await
            .expect("uncategorised_count should succeed");
        assert_eq!(count, 0, "voided transaction should not be counted");
    }
}
