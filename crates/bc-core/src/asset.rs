//! Service for recording asset valuations and calculating depreciation.

use bc_models::AccountId;
use bc_models::DepreciationPolicy;
use bc_models::PostingId;
use bc_models::TransactionId;
use bc_models::TransactionStatus;
use bc_models::ValuationId;
use bc_models::ValuationSource;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Service for recording asset valuations and calculating depreciation.
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

/// Returns the number of days between two dates (0 if `to <= from`).
///
/// # Arguments
///
/// * `from` - Start date (inclusive).
/// * `to` - End date (inclusive).
///
/// # Returns
///
/// Non-negative day count, or `0` if `to` is not after `from`.
fn days_between(from: Date, to: Date) -> i32 {
    use jiff::Unit;
    from.until((Unit::Day, to))
        .map(|s| s.get_days())
        .unwrap_or(0)
        .max(0)
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a point-in-time market value for a [`ManualAsset`] account.
    ///
    /// Atomically:
    /// 1. Appends an [`Event::AssetValuationRecorded`] to the event log.
    /// 2. Inserts a row into `asset_valuations`.
    /// 3. If `counterpart_id` is `Some` and `market_value` is non-zero, inserts a
    ///    double-entry transaction with two postings:
    ///    - `asset_account` receives `+market_value`
    ///    - `counterpart_account` receives `-market_value`
    ///
    /// # Arguments
    ///
    /// * `account_id` - The `ManualAsset` account being valued.
    /// * `market_value` - Assessed market value (positive).
    /// * `commodity` - Commodity of the valuation (e.g. `"AUD"`).
    /// * `source` - Authority for this valuation.
    /// * `recorded_at` - Business date of the assessment.
    /// * `counterpart_id` - Optional equity/liability account for the balancing posting.
    ///
    /// # Returns
    ///
    /// The [`ValuationId`] of the newly created record.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append, serialisation, or database insert failure.
    #[inline]
    pub async fn record_valuation(
        &self,
        account_id: &AccountId,
        market_value: Decimal,
        commodity: &str,
        source: ValuationSource,
        recorded_at: Date,
        counterpart_id: Option<&AccountId>,
    ) -> BcResult<ValuationId> {
        let id = ValuationId::new();
        let now = Timestamp::now();
        let source_str = to_db_str(source)?;

        let event = Event::AssetValuationRecorded {
            id: id.clone(),
            account_id: account_id.clone(),
            market_value,
            commodity: commodity.to_owned(),
            source,
            recorded_at,
        };

        let mut tx = self.pool.begin().await?;

        insert_event(&event, &mut tx).await?;

        sqlx::query(
            "INSERT INTO asset_valuations \
             (id, account_id, market_value, commodity, source, recorded_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(account_id.to_string())
        .bind(market_value.to_string())
        .bind(commodity)
        .bind(&source_str)
        .bind(recorded_at.to_string())
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;

        // If a counterpart account is supplied and value is non-zero, insert a
        // double-entry transaction to record the unrealised gain/loss.
        if let Some(cpt_id) = counterpart_id {
            if !market_value.is_zero() {
                let tx_id = TransactionId::new();
                let status_str = to_db_str(TransactionStatus::Cleared)?;

                let event_tx = Event::TransactionCreated { id: tx_id.clone() };
                insert_event(&event_tx, &mut tx).await?;

                sqlx::query(
                    "INSERT INTO transactions \
                     (id, date, payee, description, status, created_at) \
                     VALUES (?, ?, NULL, ?, ?, ?)",
                )
                .bind(tx_id.to_string())
                .bind(recorded_at.to_string())
                .bind("Asset valuation recorded")
                .bind(&status_str)
                .bind(now.to_string())
                .execute(&mut *tx)
                .await?;

                // Asset posting: +market_value
                let asset_posting_id = PostingId::new();
                sqlx::query(
                    "INSERT INTO postings \
                     (id, transaction_id, account_id, amount, commodity, memo, position, \
                      cost_total_value, cost_total_commodity, cost_date, cost_label) \
                     VALUES (?, ?, ?, ?, ?, NULL, 0, NULL, NULL, NULL, NULL)",
                )
                .bind(asset_posting_id.to_string())
                .bind(tx_id.to_string())
                .bind(account_id.to_string())
                .bind(market_value.to_string())
                .bind(commodity)
                .execute(&mut *tx)
                .await?;

                // Counterpart posting: -market_value
                let counterpart_posting_id = PostingId::new();
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "negating a well-bounded user-supplied market value; overflow is not possible in practice"
                )]
                let neg_value = -market_value;
                sqlx::query(
                    "INSERT INTO postings \
                     (id, transaction_id, account_id, amount, commodity, memo, position, \
                      cost_total_value, cost_total_commodity, cost_date, cost_label) \
                     VALUES (?, ?, ?, ?, ?, NULL, 1, NULL, NULL, NULL, NULL)",
                )
                .bind(counterpart_posting_id.to_string())
                .bind(tx_id.to_string())
                .bind(cpt_id.to_string())
                .bind(neg_value.to_string())
                .bind(commodity)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        tracing::info!(
            account_id = %account_id,
            valuation_id = %id,
            %market_value,
            %commodity,
            "asset valuation recorded"
        );
        Ok(id)
    }

    /// Returns the most recent market value for `account_id` in `commodity`.
    ///
    /// Returns `None` if no valuations have been recorded.
    ///
    /// Rows are ordered by `recorded_at DESC` (business date), then `created_at DESC`
    /// (insertion order) to break ties.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose market value is queried.
    /// * `commodity` - Commodity filter (e.g. `"AUD"`).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn latest_market_value(
        &self,
        account_id: &AccountId,
        commodity: &str,
    ) -> BcResult<Option<Decimal>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT market_value \
             FROM asset_valuations \
             WHERE account_id = ? AND commodity = ? \
             ORDER BY recorded_at DESC, created_at DESC \
             LIMIT 1",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|(s,)| {
            s.parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid market_value '{s}': {e}")))
        })
        .transpose()
    }

    /// Returns the book value for `account_id` in `commodity`.
    ///
    /// `book_value = acquisition_cost - SUM(asset_depreciations.amount)`.
    ///
    /// Returns `None` if the account has no `acquisition_cost`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose book value is queried.
    /// * `commodity` - Commodity filter (e.g. `"AUD"`).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn book_value(
        &self,
        account_id: &AccountId,
        commodity: &str,
    ) -> BcResult<Option<Decimal>> {
        // Fetch acquisition_cost from the accounts table.
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT acquisition_cost FROM accounts WHERE id = ?")
                .bind(account_id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        let acquisition_cost_str = match row {
            None => return Err(BcError::NotFound(account_id.to_string())),
            Some((None,)) => return Ok(None),
            Some((Some(s),)) => s,
        };

        let acquisition_cost = acquisition_cost_str.parse::<Decimal>().map_err(|e| {
            BcError::BadData(format!(
                "invalid acquisition_cost '{acquisition_cost_str}': {e}"
            ))
        })?;

        // Sum all depreciation amounts recorded for this account + commodity.
        let depr_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT amount FROM asset_depreciations WHERE account_id = ? AND commodity = ?",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_all(&self.pool)
        .await?;

        let total_depr = depr_rows.into_iter().try_fold(Decimal::ZERO, |acc, (s,)| {
            let d = s
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid depreciation amount '{s}': {e}")))?;
            acc.checked_add(d)
                .ok_or_else(|| BcError::BadData("depreciation sum overflow".into()))
        })?;

        Ok(Some(acquisition_cost.checked_sub(total_depr).ok_or_else(
            || BcError::BadData("book value underflow".into()),
        )?))
    }

    /// Calculates and records depreciation for `account_id` up to `as_of`.
    ///
    /// The depreciation period runs from the last recorded depreciation end date
    /// (or `acquisition_date` if none) up to `as_of` (inclusive).
    ///
    /// Atomically:
    /// 1. Appends a [`Event::DepreciationCalculated`] to the event log.
    /// 2. Inserts a row into `asset_depreciations`.
    /// 3. Inserts a double-entry transaction:
    ///    - `expense_account_id` receives `+amount` (depreciation expense)
    ///    - `account_id` (asset) receives `-amount` (asset value reduced)
    ///
    /// # Arguments
    ///
    /// * `account_id` - The `ManualAsset` account to depreciate.
    /// * `commodity` - Commodity of the depreciation (e.g. `"AUD"`).
    /// * `as_of` - End of the depreciation period (inclusive).
    /// * `expense_account_id` - The expense account to debit.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::InvalidAccountKind`] if the account has no `acquisition_cost` or
    /// `depreciation_policy`.
    /// Returns [`BcError`] on event append, serialisation, or database insert failure.
    #[expect(
        clippy::too_many_lines,
        reason = "depreciation recording inherently spans event logging, projection insert, and double-entry transaction; refactoring into helpers would obscure the atomic operation"
    )]
    #[inline]
    pub async fn record_depreciation(
        &self,
        account_id: &AccountId,
        commodity: &str,
        as_of: Date,
        expense_account_id: &AccountId,
    ) -> BcResult<()> {
        let acct_svc = crate::account::Service::new(self.pool.clone());
        let account = acct_svc.find_by_id(account_id).await?;

        let acquisition_cost =
            account
                .acquisition_cost()
                .ok_or_else(|| BcError::InvalidAccountKind {
                    operation: "record_depreciation",
                    account_id: account_id.clone(),
                    kind: account.kind(),
                })?;

        let acquisition_date =
            account
                .acquisition_date()
                .ok_or_else(|| BcError::InvalidAccountKind {
                    operation: "record_depreciation",
                    account_id: account_id.clone(),
                    kind: account.kind(),
                })?;

        let policy = account
            .depreciation_policy()
            .ok_or_else(|| BcError::InvalidAccountKind {
                operation: "record_depreciation",
                account_id: account_id.clone(),
                kind: account.kind(),
            })?;

        if matches!(policy, DepreciationPolicy::None) {
            return Ok(());
        }

        // Determine period start: last recorded depreciation end or acquisition_date.
        let last_end: Option<String> = sqlx::query_scalar(
            "SELECT period_end FROM asset_depreciations \
             WHERE account_id = ? AND commodity = ? \
             ORDER BY period_end DESC \
             LIMIT 1",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_optional(&self.pool)
        .await?;

        let period_start = match last_end {
            Some(s) => {
                let end = s
                    .parse::<Date>()
                    .map_err(|e| BcError::BadData(format!("invalid period_end '{s}': {e}")))?;
                // Period starts the day after the last recorded end.
                end.tomorrow()
                    .map_err(|e| BcError::BadData(format!("date overflow after {end}: {e}")))?
            }
            None => acquisition_date,
        };

        if as_of <= period_start {
            // Nothing to depreciate.
            return Ok(());
        }

        // Book value for declining balance.
        let book_val = self
            .book_value(account_id, commodity)
            .await?
            .unwrap_or(acquisition_cost);

        let days = days_between(period_start, as_of);
        if days <= 0_i32 {
            return Ok(());
        }

        let divisor = "365.25"
            .parse::<Decimal>()
            .map_err(|e| BcError::BadData(format!("cannot parse 365.25: {e}")))?;

        let days_decimal = Decimal::from(days);
        let amount = match policy {
            DepreciationPolicy::None => return Ok(()),
            DepreciationPolicy::StraightLine { annual_rate } => {
                let daily = acquisition_cost
                    .checked_mul(*annual_rate)
                    .and_then(|v: Decimal| v.checked_div(divisor))
                    .ok_or_else(|| BcError::BadData("straight-line daily rate overflow".into()))?;
                daily
                    .checked_mul(days_decimal)
                    .ok_or_else(|| BcError::BadData("straight-line depreciation overflow".into()))?
            }
            DepreciationPolicy::DecliningBalance { annual_rate } => {
                let daily = book_val
                    .checked_mul(*annual_rate)
                    .and_then(|v: Decimal| v.checked_div(divisor))
                    .ok_or_else(|| {
                        BcError::BadData("declining-balance daily rate overflow".into())
                    })?;
                daily.checked_mul(days_decimal).ok_or_else(|| {
                    BcError::BadData("declining-balance depreciation overflow".into())
                })?
            }
            _ => {
                return Err(BcError::BadData(
                    "unknown depreciation policy variant".into(),
                ));
            }
        };

        let depr_id = ValuationId::new();
        let now = Timestamp::now();
        let status_str = to_db_str(TransactionStatus::Cleared)?;

        let event = Event::DepreciationCalculated {
            id: depr_id.clone(),
            account_id: account_id.clone(),
            amount,
            commodity: commodity.to_owned(),
            period_start,
            period_end: as_of,
        };

        let mut db_tx = self.pool.begin().await?;

        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO asset_depreciations \
             (id, account_id, amount, commodity, period_start, period_end, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(depr_id.to_string())
        .bind(account_id.to_string())
        .bind(amount.to_string())
        .bind(commodity)
        .bind(period_start.to_string())
        .bind(as_of.to_string())
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        // Double-entry: expense account gets +amount, asset account gets -amount.
        let tx_id = TransactionId::new();
        let event_tx = Event::TransactionCreated { id: tx_id.clone() };
        insert_event(&event_tx, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO transactions \
             (id, date, payee, description, status, created_at) \
             VALUES (?, ?, NULL, ?, ?, ?)",
        )
        .bind(tx_id.to_string())
        .bind(as_of.to_string())
        .bind("Depreciation expense")
        .bind(&status_str)
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        // Expense posting: +amount (debit)
        let expense_posting_id = PostingId::new();
        sqlx::query(
            "INSERT INTO postings \
             (id, transaction_id, account_id, amount, commodity, memo, position, \
              cost_total_value, cost_total_commodity, cost_date, cost_label) \
             VALUES (?, ?, ?, ?, ?, NULL, 0, NULL, NULL, NULL, NULL)",
        )
        .bind(expense_posting_id.to_string())
        .bind(tx_id.to_string())
        .bind(expense_account_id.to_string())
        .bind(amount.to_string())
        .bind(commodity)
        .execute(&mut *db_tx)
        .await?;

        // Asset posting: -amount (credit — asset value reduced)
        let asset_posting_id = PostingId::new();
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "negating a computed depreciation amount; overflow is not possible in practice"
        )]
        let neg_amount = -amount;
        sqlx::query(
            "INSERT INTO postings \
             (id, transaction_id, account_id, amount, commodity, memo, position, \
              cost_total_value, cost_total_commodity, cost_date, cost_label) \
             VALUES (?, ?, ?, ?, ?, NULL, 1, NULL, NULL, NULL, NULL)",
        )
        .bind(asset_posting_id.to_string())
        .bind(tx_id.to_string())
        .bind(account_id.to_string())
        .bind(neg_amount.to_string())
        .bind(commodity)
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(
            account_id = %account_id,
            depreciation_id = %depr_id,
            %amount,
            %commodity,
            %period_start,
            period_end = %as_of,
            "depreciation recorded"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::ValuationSource;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    async fn make_manual_asset(pool: &SqlitePool) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create(
                "House",
                AccountType::Asset,
                AccountKind::ManualAsset,
                None,
                None,
                &[],
                &[],
                Some(jiff::civil::date(2020, 1, 1)),
                Some(dec!(650_000)),
                Some(&bc_models::DepreciationPolicy::StraightLine {
                    annual_rate: dec!(0.025),
                }),
            )
            .await
            .expect("create ManualAsset account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn record_valuation_persists_to_db(pool: SqlitePool) {
        let account_id = make_manual_asset(&pool).await;
        let svc = super::Service::new(pool.clone());

        svc.record_valuation(
            &account_id,
            dec!(700_000),
            "AUD",
            ValuationSource::ProfessionalAppraisal,
            jiff::civil::date(2026, 3, 31),
            None, // no counterpart — no auto-transaction
        )
        .await
        .expect("record valuation should succeed");

        let mv = svc
            .latest_market_value(&account_id, "AUD")
            .await
            .expect("query market value");
        assert_eq!(mv, Some(dec!(700_000)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn latest_market_value_returns_most_recent(pool: SqlitePool) {
        let account_id = make_manual_asset(&pool).await;
        let svc = super::Service::new(pool.clone());

        svc.record_valuation(
            &account_id,
            dec!(500_000),
            "AUD",
            ValuationSource::ManualEstimate,
            jiff::civil::date(2024, 1, 1),
            None,
        )
        .await
        .expect("first valuation");

        svc.record_valuation(
            &account_id,
            dec!(650_000),
            "AUD",
            ValuationSource::MarketData,
            jiff::civil::date(2025, 6, 1),
            None,
        )
        .await
        .expect("second valuation");

        let mv = svc
            .latest_market_value(&account_id, "AUD")
            .await
            .expect("query");
        assert_eq!(mv, Some(dec!(650_000)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn latest_market_value_none_when_no_valuations(pool: SqlitePool) {
        let account_id = make_manual_asset(&pool).await;
        let svc = super::Service::new(pool.clone());
        let mv = svc
            .latest_market_value(&account_id, "AUD")
            .await
            .expect("query");
        assert_eq!(mv, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn record_valuation_with_counterpart_creates_transaction(pool: SqlitePool) {
        // Create the asset account
        let asset_id = make_manual_asset(&pool).await;

        // Create an equity counterpart account
        let counterpart_id = crate::AccountService::new(pool.clone())
            .create(
                "Equity:Unrealized",
                AccountType::Equity,
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
            .expect("create equity account");

        let svc = super::Service::new(pool.clone());
        svc.record_valuation(
            &asset_id,
            dec!(700_000),
            "AUD",
            ValuationSource::ProfessionalAppraisal,
            jiff::civil::date(2026, 3, 31),
            Some(&counterpart_id),
        )
        .await
        .expect("record valuation with counterpart");

        // Check the balance on the equity counterpart (should be non-zero)
        let balance = crate::BalanceEngine::new(pool.clone())
            .balance_for(&counterpart_id, "AUD")
            .await
            .expect("balance query");
        // The counterpart receives the opposite posting — if asset goes up, equity goes down
        assert_eq!(balance, dec!(-700_000));
    }
}
