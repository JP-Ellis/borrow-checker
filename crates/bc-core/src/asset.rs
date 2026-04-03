//! Service for recording asset valuations and calculating depreciation.

use bc_models::AccountId;
use bc_models::DepreciationId;
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
fn days_between(from: Date, to: Date) -> u32 {
    use jiff::Unit;
    let days = from
        .until((Unit::Day, to))
        .map_or(0_i64, |s| s.get_days().into())
        .max(0_i64)
        .min(i64::from(u32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to u32::MAX above; a loan term exceeding ~11.7 million years is not a practical concern"
    )]
    #[expect(clippy::cast_sign_loss, reason = "clamped to non-negative by .max(0)")]
    {
        days as u32
    }
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
    /// 3. If `counterpart_id` is `Some` and the **change** from the previous valuation is
    ///    non-zero, inserts a double-entry transaction with two postings:
    ///    - `asset_account` receives `+change`
    ///    - `counterpart_account` receives `-change`
    ///
    ///    On the first valuation with a counterpart, the full market value is used as the
    ///    change (treating the initial opening as $0).
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
    #[expect(
        clippy::too_many_lines,
        reason = "record_valuation inherently spans account kind check, event logging, projection insert, and double-entry transaction; refactoring into helpers would obscure the atomic operation"
    )]
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
        // Verify the account is a ManualAsset.
        let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM accounts WHERE id = ?")
            .bind(account_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match kind.as_deref() {
            None => return Err(BcError::NotFound(format!("account {account_id}"))),
            Some(k) if k != "manual_asset" => {
                let parsed = crate::db::from_db_str::<bc_models::AccountKind>(k)?;
                return Err(BcError::InvalidAccountKind {
                    operation: "record asset valuation",
                    account_id: account_id.clone(),
                    kind: parsed,
                });
            }
            Some(_) => {}
        }

        if market_value <= Decimal::ZERO {
            return Err(BcError::BadData("market_value must be positive".into()));
        }

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

        // Query the previous market value BEFORE inserting, so we can compute the delta.
        // This must happen inside the transaction to get a consistent snapshot.
        let prev_row: Option<(String,)> = sqlx::query_as(
            "SELECT market_value \
             FROM asset_valuations \
             WHERE account_id = ? AND commodity = ? \
             ORDER BY recorded_at DESC, created_at DESC \
             LIMIT 1",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_optional(&mut *tx)
        .await?;

        let previous_market_value = prev_row
            .map(|(s,)| {
                s.parse::<Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid market_value '{s}': {e}")))
            })
            .transpose()?
            .unwrap_or(Decimal::ZERO);

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "subtracting two well-bounded market values; overflow is not possible in practice"
        )]
        let change = market_value - previous_market_value;

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

        // If a counterpart account is supplied and the change is non-zero, insert a
        // double-entry transaction to record the unrealised gain/loss.
        if let Some(cpt_id) = counterpart_id {
            if !change.is_zero() {
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

                // Asset posting: +change
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
                .bind(change.to_string())
                .bind(commodity)
                .execute(&mut *tx)
                .await?;

                // Counterpart posting: -change
                let counterpart_posting_id = PostingId::new();
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "negating a well-bounded delta value; overflow is not possible in practice"
                )]
                let neg_change = -change;
                sqlx::query(
                    "INSERT INTO postings \
                     (id, transaction_id, account_id, amount, commodity, memo, position, \
                      cost_total_value, cost_total_commodity, cost_date, cost_label) \
                     VALUES (?, ?, ?, ?, ?, NULL, 1, NULL, NULL, NULL, NULL)",
                )
                .bind(counterpart_posting_id.to_string())
                .bind(tx_id.to_string())
                .bind(cpt_id.to_string())
                .bind(neg_change.to_string())
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
        // Use a read transaction for a consistent snapshot across the two queries.
        let mut tx = self.pool.begin().await?;

        // Fetch acquisition_cost from the accounts table.
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT acquisition_cost FROM accounts WHERE id = ?")
                .bind(account_id.to_string())
                .fetch_optional(&mut *tx)
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
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

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
    /// Returns [`BcError::InvalidAccountKind`] if the account is not a `ManualAsset`.
    /// Returns [`BcError::BadData`] if `acquisition_cost`, `acquisition_date`, or
    /// `depreciation_policy` are not set, or if `as_of` is not after the period start.
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
        // Verify the account is a ManualAsset before accessing ManualAsset-specific fields.
        let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM accounts WHERE id = ?")
            .bind(account_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match kind.as_deref() {
            None => return Err(BcError::NotFound(format!("account {account_id}"))),
            Some(k) if k != "manual_asset" => {
                let parsed = crate::db::from_db_str::<bc_models::AccountKind>(k)?;
                return Err(BcError::InvalidAccountKind {
                    operation: "record depreciation",
                    account_id: account_id.clone(),
                    kind: parsed,
                });
            }
            Some(_) => {}
        }

        let acct_svc = crate::account::Service::new(self.pool.clone());
        let account = acct_svc.find_by_id(account_id).await?;

        let acquisition_cost = account.acquisition_cost().ok_or_else(|| {
            BcError::BadData(
                "account is missing acquisition_cost — set it when creating the account".into(),
            )
        })?;

        let acquisition_date = account.acquisition_date().ok_or_else(|| {
            BcError::BadData(
                "account is missing acquisition_date — set it when creating the account".into(),
            )
        })?;

        let policy = account.depreciation_policy().ok_or_else(|| {
            BcError::BadData(
                "account is missing depreciation_policy — set it when creating the account".into(),
            )
        })?;

        if matches!(policy, DepreciationPolicy::None) {
            return Err(BcError::BadData(
                "depreciation is disabled for this account (policy is None)".into(),
            ));
        }

        let mut db_tx = self.pool.begin().await?;

        // Determine period start: last recorded depreciation end or acquisition_date.
        // Read inside the transaction to avoid TOCTOU races.
        let last_end: Option<String> = sqlx::query_scalar(
            "SELECT period_end FROM asset_depreciations \
             WHERE account_id = ? AND commodity = ? \
             ORDER BY period_end DESC \
             LIMIT 1",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_optional(&mut *db_tx)
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
            return Err(BcError::BadData(
                "as_of date must be after the depreciation period start".into(),
            ));
        }

        // Compute book value within the transaction for a consistent snapshot.
        // book_value = acquisition_cost - SUM(asset_depreciations.amount)
        let depr_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT amount FROM asset_depreciations WHERE account_id = ? AND commodity = ?",
        )
        .bind(account_id.to_string())
        .bind(commodity)
        .fetch_all(&mut *db_tx)
        .await?;

        let total_depr = depr_rows.into_iter().try_fold(Decimal::ZERO, |acc, (s,)| {
            let d = s
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid depreciation amount '{s}': {e}")))?;
            acc.checked_add(d)
                .ok_or_else(|| BcError::BadData("depreciation sum overflow".into()))
        })?;

        let book_val = acquisition_cost
            .checked_sub(total_depr)
            .ok_or_else(|| BcError::BadData("book value underflow".into()))?;

        let days = days_between(period_start, as_of);
        if days == 0 {
            return Ok(());
        }

        let divisor = "365.25"
            .parse::<Decimal>()
            .map_err(|e| BcError::BadData(format!("cannot parse 365.25: {e}")))?;

        let days_decimal = Decimal::from(days);
        let amount = match policy {
            DepreciationPolicy::None => {
                unreachable!("DepreciationPolicy::None filtered by the guard above")
            }
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

        let depr_id = DepreciationId::new();
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

    #[sqlx::test(migrations = "./migrations")]
    async fn record_valuation_on_deposit_account_fails(pool: SqlitePool) {
        let deposit_id = crate::AccountService::new(pool.clone())
            .create(
                "Savings",
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
            .expect("create DepositAccount");
        let svc = super::Service::new(pool.clone());
        let result = svc
            .record_valuation(
                &deposit_id,
                dec!(1_000),
                "AUD",
                ValuationSource::ManualEstimate,
                jiff::civil::date(2026, 1, 1),
                None,
            )
            .await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidAccountKind { .. })),
            "expected InvalidAccountKind, got {result:?}"
        );
    }

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

    /// Second valuation should only post the delta, not the full absolute value.
    ///
    /// First valuation: 700_000 → posts +700_000 to asset, -700_000 to counterpart.
    /// Second valuation: 750_000 → change is +50_000 → posts +50_000 to asset, -50_000 to counterpart.
    /// Net counterpart balance: -700_000 + -50_000 = -750_000.
    #[sqlx::test(migrations = "./migrations")]
    async fn record_valuation_second_valuation_posts_delta(pool: SqlitePool) {
        let asset_id = make_manual_asset(&pool).await;

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

        // First valuation: full market value posted as change (previous = 0)
        svc.record_valuation(
            &asset_id,
            dec!(700_000),
            "AUD",
            ValuationSource::ProfessionalAppraisal,
            jiff::civil::date(2026, 3, 31),
            Some(&counterpart_id),
        )
        .await
        .expect("first valuation");

        // Second valuation: only the delta (750_000 - 700_000 = 50_000) should be posted
        svc.record_valuation(
            &asset_id,
            dec!(750_000),
            "AUD",
            ValuationSource::MarketData,
            jiff::civil::date(2026, 6, 30),
            Some(&counterpart_id),
        )
        .await
        .expect("second valuation");

        let balance_engine = crate::BalanceEngine::new(pool.clone());

        // Asset balance should reflect cumulative postings: +700_000 + 50_000 = +750_000
        let asset_balance = balance_engine
            .balance_for(&asset_id, "AUD")
            .await
            .expect("asset balance query");
        assert_eq!(asset_balance, dec!(750_000));

        // Counterpart balance: -700_000 + -50_000 = -750_000
        let counterpart_balance = balance_engine
            .balance_for(&counterpart_id, "AUD")
            .await
            .expect("counterpart balance query");
        assert_eq!(counterpart_balance, dec!(-750_000));
    }
}
