//! Balance calculation engine.

use bc_models::AccountId;
use bc_models::Amount;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

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
    pub inflow: Amount,
    /// Sum of absolute negative postings (money leaving the account) in this period.
    pub outflow: Amount,
}

/// Windowed account statistics for the dashboard: in-window flows plus the
/// opening/closing running balances that bracket the window.
///
/// All [`Amount`]s carry the queried commodity. `income`, `expenses`, and
/// `tx_count` cover `[from, until)`; `opening` is the running balance
/// immediately before the window; `closing` is the running balance at the
/// window end.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PeriodStats {
    /// In-window inflow (non-negative).
    pub income: Amount,
    /// In-window outflow magnitude (non-negative).
    pub expenses: Amount,
    /// `income − expenses` (signed).
    pub net: Amount,
    /// Running balance immediately before the window (`[genesis, from)`).
    pub opening: Amount,
    /// Running balance at the window end (`opening + net`).
    pub closing: Amount,
    /// Count of distinct in-window transactions involving the account
    /// (commodity-agnostic; matches the register's row count).
    pub tx_count: u32,
}

/// Calculates account balances from the `postings` projection table.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

// MARK: Queries
//
// Every query whose *plan* is load-bearing lives here rather than inline, so the
// `EXPLAIN QUERY PLAN` tests below assert against the text that actually runs. A
// non-sargable rewrite still returns correct rows, so a test holding its own copy
// of the SQL would keep passing while the real query regressed.

/// Concrete legs of one account in one commodity, unbounded in date.
const BALANCE_SQL: &str = "SELECT p.amount
     FROM postings p
     WHERE p.account_id = ?
       AND p.commodity  = ?";

/// Concrete legs of one account in one commodity, over a half-open date window.
const WINDOW_CONCRETE_SQL: &str = "SELECT p.date, p.amount
     FROM postings p
     WHERE p.account_id = ?
       AND p.commodity  = ?
       AND p.date      >= ?
       AND p.date       < ?";

/// Elided legs of one account over a half-open date window.
///
/// Commodity-agnostic: an elided leg carries neither amount nor commodity, so its
/// value comes from the transaction's residual rather than from this row.
const WINDOW_ELIDED_SQL: &str = "SELECT p.id, p.date
     FROM postings p
     WHERE p.account_id = ?
       AND p.amount IS NULL
       AND p.date >= ?
       AND p.date  < ?";

/// Distinct transactions touching one account over a half-open date window.
const TX_COUNT_SQL: &str = "SELECT COUNT(DISTINCT p.transaction_id)
     FROM postings p
     WHERE p.account_id = ?
       AND p.date >= ?
       AND p.date  < ?";

/// Builds `count` contiguous period buckets ending with the period containing
/// `as_of`, oldest-first. Each bucket is a half-open `[start, end)` range one
/// `period` wide.
///
/// Every bucket is snapped to the period's own calendar boundary, so the first
/// bucket generally starts *before* `as_of - count * period` and the last one
/// ends *after* `as_of`.
///
/// The WASM side mirrors these snapping rules: `bc-ui` re-implements them in its
/// `coverage_count` helper because `bc-core` is native-only and absent from the
/// WASM bundle. Any change to the snapping here must be mirrored there, or the
/// sparkline's coverage estimate silently drifts from the buckets it labels.
///
/// # Arguments
///
/// * `period` - Bucket width.
/// * `count`  - Number of buckets.
/// * `as_of`  - Reference date; the newest bucket contains it.
///
/// # Returns
///
/// A `Vec` of `(start, end)` ranges, oldest-first, of length `count`.
pub(crate) fn bucket_ranges(
    period: &bc_models::Period,
    count: core::num::NonZeroUsize,
    as_of: jiff::civil::Date,
) -> Vec<(jiff::civil::Date, jiff::civil::Date)> {
    let mut ranges: Vec<(jiff::civil::Date, jiff::civil::Date)> = Vec::with_capacity(count.get());
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
    ranges
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
    /// Returns an [`Amount`] carrying `commodity`, zero-valued if no postings exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a stored amount cannot be parsed.
    #[inline]
    pub async fn balance_for(&self, account_id: &AccountId, commodity: &str) -> BcResult<Amount> {
        let rows: Vec<(String,)> = sqlx::query_as(BALANCE_SQL)
            .bind(account_id.to_string())
            .bind(commodity)
            .fetch_all(&self.pool)
            .await?;

        let concrete = rows.into_iter().try_fold(Decimal::ZERO, |acc, (amt,)| {
            let d = amt
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid decimal amount '{amt}': {e}")))?;
            acc.checked_add(d).ok_or_else(|| {
                BcError::BadData("balance overflow: sum exceeds Decimal range".into())
            })
        })?;

        // Elided legs carry no stored amount, so they are absent from the query
        // above; their residual is derived and added here (see `crate::residual`).
        let residual = crate::residual::Residuals::for_account(&self.pool, account_id)
            .await?
            .total_in(commodity)?;

        let total = concrete.checked_add(residual).ok_or_else(|| {
            BcError::BadData("balance overflow: sum exceeds Decimal range".into())
        })?;
        Ok(Amount::new(total, commodity))
    }

    /// Computes total net worth in `commodity` across all asset and liability accounts.
    ///
    /// - [`DepositAccount`], [`Receivable`], [`VirtualAllocation`], [`Group`]: balance from postings.
    ///   A [`Group`] is an organisational node whose postings belong on its
    ///   descendants, so its own balance is normally zero.
    /// - [`ManualAsset`]: latest recorded market value from `asset_valuations`.
    /// - Accounts with `AccountType` other than `Asset`/`Liability` are excluded.
    ///
    /// Returns an [`Amount`] carrying `commodity`, zero-valued if no relevant accounts exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    ///
    /// [`DepositAccount`]: bc_models::AccountKind::DepositAccount
    /// [`Receivable`]: bc_models::AccountKind::Receivable
    /// [`VirtualAllocation`]: bc_models::AccountKind::VirtualAllocation
    /// [`ManualAsset`]: bc_models::AccountKind::ManualAsset
    /// [`Group`]: bc_models::AccountKind::Group
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "intentional fallback with warning for future AccountKind variants"
    )]
    #[inline]
    pub async fn net_worth(&self, commodity: &str) -> BcResult<Amount> {
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
                | AccountKind::VirtualAllocation
                | AccountKind::Group => self.balance_for(account.id(), commodity).await?.value(),
                _ => {
                    tracing::warn!(
                        account_id = %account.id(),
                        kind = ?account.kind(),
                        "unknown AccountKind in net_worth; using posting-based balance"
                    );
                    self.balance_for(account.id(), commodity).await?.value()
                }
            };

            total = total
                .checked_add(contribution)
                .ok_or_else(|| BcError::BadData("net worth overflow".into()))?;
        }

        Ok(Amount::new(total, commodity))
    }

    /// Fetches all postings for `account_id` in `commodity` within `[from, to)`.
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
        // One deferred transaction across all three queries below. The elided-id
        // query and the residual load must agree on which postings exist: a date
        // amendment landing between them would leave an id absent from the loaded
        // `Residuals`, which `component` reports as an out-of-scope error. Under WAL
        // a read transaction is a consistent snapshot and does not block writers.
        let mut tx = self.pool.begin().await?;

        let rows: Vec<(String, String)> = sqlx::query_as(WINDOW_CONCRETE_SQL)
            .bind(account_id.to_string())
            .bind(commodity)
            .bind(from.to_string())
            .bind(to.to_string())
            .fetch_all(&mut *tx)
            .await?;

        let mut out: Vec<(jiff::civil::Date, Decimal)> = rows
            .into_iter()
            .map(|(date_str, amt_str)| {
                let date = date_str
                    .parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                let amount = amt_str
                    .parse::<Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid amount '{amt_str}': {e}")))?;
                Ok((date, amount))
            })
            .collect::<BcResult<Vec<_>>>()?;

        // Elided legs have a NULL commodity, so the query above cannot match
        // them. Fetch them separately and resolve each one's residual; a
        // residual carries its transaction's date, so ordering is unaffected.
        let elided: Vec<(String, String)> = sqlx::query_as(WINDOW_ELIDED_SQL)
            .bind(account_id.to_string())
            .bind(from.to_string())
            .bind(to.to_string())
            .fetch_all(&mut *tx)
            .await?;

        if !elided.is_empty() {
            let residuals =
                crate::residual::Residuals::for_account_in_range(&mut *tx, account_id, from, to)
                    .await?;
            for (posting_id, date_str) in elided {
                let Some(value) = residuals.component(&posting_id, commodity)? else {
                    continue;
                };
                let date = date_str
                    .parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid date '{date_str}': {e}")))?;
                out.push((date, value));
            }
        }

        // Nothing was written, so the snapshot is released rather than committed.
        tx.rollback().await?;

        Ok(out)
    }

    /// Returns the total inflow and outflow for `account_id` in `commodity` over `[from, to)`.
    ///
    /// - `inflow` — sum of all positive postings (money entering the account).
    /// - `outflow` — absolute sum of all negative postings (money leaving).
    ///
    /// Both values are non-negative.
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
    /// `(inflow, outflow)` as [`Amount`] values.
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
    ) -> BcResult<(Amount, Amount)> {
        let rows = self
            .fetch_postings_in_range(account_id, commodity, from, to)
            .await?;

        Self::sum_flows(&rows, commodity)
    }

    /// Splits signed posting amounts into non-negative `(inflow, outflow)` totals.
    ///
    /// `inflow` is the sum of positive amounts; `outflow` is the absolute sum of
    /// negative amounts. Both carry `commodity`. Shared by [`Service::posting_flows`]
    /// and [`Service::account_period_stats`] so a single fetch can serve both.
    ///
    /// # Arguments
    ///
    /// * `rows`      - `(date, amount)` pairs; only the amounts are summed.
    /// * `commodity` - Commodity code carried by the returned amounts.
    ///
    /// # Returns
    ///
    /// `(inflow, outflow)` as [`Amount`] values, both non-negative.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if either running total overflows [`Decimal`].
    fn sum_flows(
        rows: &[(jiff::civil::Date, Decimal)],
        commodity: &str,
    ) -> BcResult<(Amount, Amount)> {
        let (inflow, outflow) = rows.iter().try_fold(
            (Decimal::ZERO, Decimal::ZERO),
            |(inflow, outflow), &(_, amount)| -> BcResult<(Decimal, Decimal)> {
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
        )?;
        Ok((
            Amount::new(inflow, commodity),
            Amount::new(outflow, commodity),
        ))
    }

    /// Counts distinct transactions involving `account_id` whose canonical date
    /// falls in the half-open interval `[from, until)`.
    ///
    /// Commodity-agnostic, so the count matches the row count of
    /// [`Service::list_for_account_in_range`] — a dashboard "transactions" stat
    /// agrees with the register even when a transaction has multiple postings to
    /// the account or spans several commodities.
    ///
    /// Counts distinct `transaction_id`s among the account's own postings in the
    /// window, rather than joining back to `transactions`: `postings.date` mirrors
    /// its transaction's date via the `postings_date_*` triggers, and
    /// `postings.transaction_id` is a NOT NULL foreign key, so this is exactly the
    /// set of transactions with at least one posting to the account in `[from,
    /// until)` — identical to the old `transactions`-driven query, but sargable on
    /// `idx_postings_account_date`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `from`       - Inclusive lower bound on the posting date.
    /// * `until`      - Exclusive upper bound on the posting date.
    ///
    /// # Returns
    ///
    /// The number of matching transactions, saturating at [`u32::MAX`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    async fn count_transactions_in_range(
        &self,
        account_id: &AccountId,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
    ) -> BcResult<u32> {
        let (count,): (i64,) = sqlx::query_as(TX_COUNT_SQL)
            .bind(account_id.to_string())
            .bind(from.to_string())
            .bind(until.to_string())
            .fetch_one(&self.pool)
            .await?;

        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Computes [`PeriodStats`] for `account_id` in `commodity` over `[from, until)`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity` - Commodity code (e.g. `"AUD"`).
    /// * `from` - Inclusive window start.
    /// * `until` - Exclusive window end.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a running
    /// balance would overflow [`Decimal`]'s range.
    #[inline]
    pub async fn account_period_stats(
        &self,
        account_id: &AccountId,
        commodity: &str,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
    ) -> BcResult<PeriodStats> {
        let genesis = jiff::civil::Date::MIN;

        let in_window = self
            .fetch_postings_in_range(account_id, commodity, from, until)
            .await?;
        let (income, expenses) = Self::sum_flows(&in_window, commodity)?;

        let (open_in, open_out) = self
            .posting_flows(account_id, commodity, genesis, from)
            .await?;
        let opening = open_in
            .value()
            .checked_sub(open_out.value())
            .ok_or_else(|| BcError::BadData("opening balance overflow".into()))?;
        let net = income
            .value()
            .checked_sub(expenses.value())
            .ok_or_else(|| BcError::BadData("net overflow".into()))?;
        let closing = opening
            .checked_add(net)
            .ok_or_else(|| BcError::BadData("closing balance overflow".into()))?;

        let tx_count = self
            .count_transactions_in_range(account_id, from, until)
            .await?;

        Ok(PeriodStats {
            income,
            expenses,
            net: Amount::new(net, commodity),
            opening: Amount::new(opening, commodity),
            closing: Amount::new(closing, commodity),
            tx_count,
        })
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
        let ranges = bucket_ranges(period, count, as_of);

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

        // Distribute postings into per-range Decimal accumulators.
        let mut acc: Vec<(jiff::civil::Date, jiff::civil::Date, Decimal, Decimal)> = ranges
            .into_iter()
            .map(|(start, end)| (start, end, Decimal::ZERO, Decimal::ZERO))
            .collect();

        for (date, amount) in all_postings {
            if let Some(slot) = acc
                .iter_mut()
                .find(|(start, end, _, _)| date >= *start && date < *end)
            {
                if amount >= Decimal::ZERO {
                    slot.2 = slot
                        .2
                        .checked_add(amount)
                        .ok_or_else(|| BcError::BadData("inflow overflow".into()))?;
                } else {
                    slot.3 = slot
                        .3
                        .checked_sub(amount)
                        .ok_or_else(|| BcError::BadData("outflow overflow".into()))?;
                }
            }
        }

        Ok(acc
            .into_iter()
            .map(|(start, end, inflow, outflow)| PostingBucket {
                start,
                end,
                inflow: Amount::new(inflow, commodity),
                outflow: Amount::new(outflow, commodity),
            })
            .collect())
    }

    /// Returns the commodity code of the first (default) commodity for `account_id`, or `None`.
    ///
    /// Prefers the configured default from `account_commodities` (position = 0). When no
    /// commodity is configured, falls back to the most-used posting commodity so that
    /// accounts imported without explicit commodity setup still return a useful value. When
    /// every posting on the account is elided (so no stored commodity exists at all), falls
    /// back further to the account's first-seen residual commodity — see
    /// [`Self::residual_commodities`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    #[inline]
    pub async fn default_commodity_for(&self, account_id: &AccountId) -> BcResult<Option<String>> {
        let (result,): (Option<String>,) = sqlx::query_as(
            "SELECT COALESCE(
                 (SELECT c.code
                  FROM account_commodities ac
                  JOIN commodities c ON c.id = ac.commodity_id
                  WHERE ac.account_id = ?
                  ORDER BY ac.position
                  LIMIT 1),
                 (SELECT p.commodity
                  FROM postings p
                  WHERE p.account_id = ?
                    AND p.commodity IS NOT NULL
                  GROUP BY p.commodity
                  ORDER BY COUNT(*) DESC
                  LIMIT 1)
             ) AS commodity_code",
        )
        .bind(account_id.to_string())
        .bind(account_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        if let Some(code) = result {
            return Ok(Some(code));
        }

        // Every posting on this account may be elided, in which case no stored
        // commodity exists anywhere — derive one from the residual instead.
        let residuals = crate::residual::Residuals::for_account(&self.pool, account_id).await?;
        let totals = residuals.totals_by_account()?;
        Ok(totals
            .get(&account_id.to_string())
            .and_then(|balances| balances.iter().next().map(|(code, _)| code.to_owned())))
    }

    /// Returns each account's first-seen residual commodity.
    ///
    /// Used only as the last tier of commodity inference, for an account whose
    /// postings are *all* elided and therefore carry no stored commodity. The
    /// chosen commodity is whichever one iterates first from that account's
    /// residual [`bc_models::Balances`] — no counting or weighting by
    /// magnitude, purely iteration order.
    ///
    /// Callers must additionally check the account is still active: this
    /// derives purely from [`crate::residual::Residuals`], which is not
    /// filtered by `archived_at` (see [`Self::default_balances`]).
    ///
    /// # Arguments
    ///
    /// * `totals` - Per-account residual totals, e.g. from
    ///   [`crate::residual::Residuals::totals_by_account`].
    ///
    /// # Returns
    ///
    /// A map from account id string to commodity code.
    fn residual_commodities(
        totals: &std::collections::HashMap<String, bc_models::Balances>,
    ) -> std::collections::HashMap<String, String> {
        totals
            .iter()
            .filter_map(|(account_id, balances)| {
                let (code, _) = balances.iter().next()?;
                Some((account_id.clone(), code.to_owned()))
            })
            .collect()
    }

    /// Sums each account's postings that are in its own default commodity.
    ///
    /// Elided postings (no commodity/amount) and postings in a non-default
    /// commodity are skipped; elided postings contribute via the residual
    /// fallback added by the caller instead.
    ///
    /// # Arguments
    ///
    /// * `posting_rows` - `(account_id, commodity, amount)` rows, amount/commodity
    ///   `None` for an elided posting.
    /// * `commodity_by_account` - Each in-scope account's default commodity.
    ///
    /// # Returns
    ///
    /// A map from account id string to summed balance, zero-seeded for every
    /// key of `commodity_by_account` so accounts with no matching postings
    /// still appear.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if an amount fails to parse or a running
    /// total overflows.
    fn sum_default_commodity_postings(
        posting_rows: &[(String, Option<String>, Option<String>)],
        commodity_by_account: &std::collections::HashMap<String, String>,
    ) -> BcResult<std::collections::HashMap<String, Decimal>> {
        let mut map: std::collections::HashMap<String, Decimal> = commodity_by_account
            .keys()
            .map(|id| (id.clone(), Decimal::ZERO))
            .collect();

        for (acc_id, opt_commodity, opt_amt_str) in posting_rows {
            let (Some(commodity), Some(amt_str)) = (opt_commodity, opt_amt_str) else {
                continue; // elided posting — contributes via its residual below
            };
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

        Ok(map)
    }

    /// Returns the default-commodity balance for every active account in one query.
    ///
    /// Balances are computed live from all postings, not from the `balances`
    /// cache table (which is a write-through cache not yet populated by the application).
    ///
    /// The map key is [`AccountId`]; the value is an [`Amount`] (carrying the default commodity).
    /// Accounts with neither a configured commodity nor any postings are omitted.
    /// Accounts with a commodity (configured or inferred) but no postings are included with a zero
    /// balance.
    ///
    /// The commodity for each account is resolved in priority order:
    /// 1. The configured default from `account_commodities` (position = 0).
    /// 2. The most-used posting commodity (for accounts imported without explicit commodity setup).
    /// 3. The first-seen commodity of the account's own residuals (for an account whose
    ///    postings are all elided and therefore carry no stored commodity at all) —
    ///    iteration order only, with no weighting; see [`Self::residual_commodities`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn default_balances(&self) -> BcResult<std::collections::HashMap<AccountId, Amount>> {
        // Fetch every active account, with its effective default commodity when one is
        // resolvable. Prefers account_commodities (position = 0); falls back to the
        // most-used posting commodity so accounts imported without explicit commodity
        // setup are still included. This is also the authoritative active-account set
        // used below: `Residuals::load` has no `archived_at` filter (it does not know
        // which account owns the transaction it is resolving), so the residual
        // commodity/balance fallback must be intersected against this set rather than
        // trusted on its own — otherwise an archived account whose only postings are
        // elided would leak back into the result.
        let account_rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT a.id,
                    COALESCE(
                        c.code,
                        (SELECT p.commodity
                         FROM postings p
                         WHERE p.account_id = a.id
                           AND p.commodity IS NOT NULL
                         GROUP BY p.commodity
                         ORDER BY COUNT(*) DESC
                         LIMIT 1)
                    ) AS commodity_code
             FROM accounts a
             LEFT JOIN account_commodities ac ON ac.account_id = a.id AND ac.position = 0
             LEFT JOIN commodities c ON c.id = ac.commodity_id
             WHERE a.archived_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        if account_rows.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let active_account_ids: std::collections::HashSet<String> =
            account_rows.iter().map(|(id, _)| id.clone()).collect();

        let residuals = crate::residual::Residuals::for_all_accounts(&self.pool).await?;
        let residual_totals = residuals.totals_by_account()?;
        let residual_commodities = Self::residual_commodities(&residual_totals);

        // Fetch all postings for those accounts (one query, filtered in Rust).
        // Elided postings (NULL amount/commodity) are included in the SQL result but
        // skipped in Rust so they do not contribute to the balance sum.
        let posting_rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT p.account_id, p.commodity, p.amount
             FROM postings p
             JOIN accounts a ON a.id = p.account_id
             WHERE a.archived_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        // Build commodity lookup: account_id_str → commodity_code. An account
        // whose postings are all elided has no stored commodity anywhere, so it
        // falls back to the commodity of its own residuals — but only when the
        // account is still active (see the comment above on `account_rows`).
        let mut commodity_by_account: std::collections::HashMap<String, String> = account_rows
            .into_iter()
            .filter_map(|(id, code)| code.map(|c| (id, c)))
            .collect();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "iteration order is irrelevant: each entry only fills a gap left by account_rows, and insertion is idempotent regardless of order"
        )]
        for (account_id, code) in residual_commodities {
            if active_account_ids.contains(&account_id) {
                commodity_by_account.entry(account_id).or_insert(code);
            }
        }

        // Sum posting amounts per account for that account's default commodity.
        let mut map = Self::sum_default_commodity_postings(&posting_rows, &commodity_by_account)?;

        // Add each account's derived residual for its default commodity.
        #[expect(
            clippy::iter_over_hash_type,
            reason = "iteration order is irrelevant: each account's residual is added to its own map entry independently, via commutative Decimal addition"
        )]
        for (acc_id, balances) in residual_totals {
            let Some(default_commodity) = commodity_by_account.get(&acc_id) else {
                continue; // account archived (commodity_by_account is the active-account set) or otherwise out of scope
            };
            let Some(value) = balances.get(default_commodity) else {
                continue; // residual holds nothing in this account's commodity
            };
            let entry = map.entry(acc_id).or_insert(Decimal::ZERO);
            *entry = entry.checked_add(value).ok_or_else(|| {
                BcError::BadData("balance overflow: sum exceeds Decimal range".into())
            })?;
        }

        // Convert string IDs to AccountId.
        map.into_iter()
            .map(|(id_str, balance)| {
                let commodity = commodity_by_account
                    .get(&id_str)
                    .ok_or_else(|| {
                        BcError::BadData(format!("commodity lookup missing for account '{id_str}'"))
                    })?
                    .clone();
                let id = id_str
                    .parse::<AccountId>()
                    .map_err(|e| BcError::BadData(format!("invalid account id '{id_str}': {e}")))?;
                Ok((id, Amount::new(balance, commodity)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroUsize;

    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use sqlx::Row as _;

    use super::*;
    use crate::account::Cascade;

    /// Returns the `detail` column of every `EXPLAIN QUERY PLAN` row for `sql`.
    ///
    /// Parameters are left unbound; SQLite treats them as NULL, which is what the
    /// planner sees for a prepared statement. Only `detail` is read, so the number of
    /// columns SQLite returns is irrelevant.
    async fn query_plan(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(pool)
            .await
            .expect("explain query plan")
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect()
    }

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
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')")
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
        assert_eq!(balance.value(), dec!(100.00));
        assert_eq!(balance.commodity().as_str(), "AUD");
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
        assert_eq!(balance.value(), Decimal::ZERO);
        assert_eq!(balance.commodity().as_str(), "AUD");
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
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_nw1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')")
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
        assert_eq!(net_worth.value(), dec!(700_000));
        assert_eq!(net_worth.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_returns_real_balance(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use rust_decimal_macros::dec;

        // Create commodity
        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES ('com_aud', 'AUD', 2, 1, 0)")
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('t1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')",
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

        let bal = map.get(&acc).expect("account should be in map");
        assert_eq!(bal.commodity().as_str(), "AUD");
        assert_eq!(bal.value(), dec!(1234.56));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_zero_for_account_without_balance_row(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES ('com_aud2', 'AUD', 2, 1, 0)")
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

        let bal = map.get(&acc).expect("account in map");
        assert_eq!(bal.commodity().as_str(), "AUD");
        assert_eq!(bal.value(), rust_decimal::Decimal::ZERO);
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tf1', '2026-04-10', 'Pay', 'reconciled', '2026-04-10T00:00:00Z')",
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tf2', '2026-04-20', 'Expense', 'reconciled', '2026-04-20T00:00:00Z')",
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

        assert_eq!(inflow.value(), dec!(1000.00));
        assert_eq!(inflow.commodity().as_str(), "AUD");
        assert_eq!(outflow.value(), dec!(250.00));
        assert_eq!(outflow.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_includes_all_reconciliation_states(pool: sqlx::SqlitePool) {
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

        // Transaction with unreconciled state — should still be included.
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tu1', '2026-04-15', 'Unreconciled', 'unreconciled', '2026-04-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("unreconciled tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pu1', 'tu1', ?, '500.00', 'AUD', 0)",
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

        // Unreconciled transactions are included.
        assert_eq!(inflow.value(), rust_decimal::Decimal::from(500_i32));
        assert_eq!(inflow.commodity().as_str(), "AUD");
        assert_eq!(outflow.value(), rust_decimal::Decimal::ZERO);
        assert_eq!(outflow.commodity().as_str(), "AUD");
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tb1', '2026-04-01', 'In', 'reconciled', '2026-04-01T00:00:00Z')",
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tb2', '2026-05-01', 'Boundary', 'reconciled', '2026-05-01T00:00:00Z')",
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
        assert_eq!(inflow.value(), dec!(100.00));
        assert_eq!(inflow.commodity().as_str(), "AUD");
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tbk1', '2026-04-15', 'April pay', 'reconciled', '2026-04-15T00:00:00Z')",
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
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tbk2', '2026-05-10', 'May rent', 'reconciled', '2026-05-10T00:00:00Z')",
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
            assert_eq!(buckets[0].inflow.value(), dec!(500.00));
            assert_eq!(buckets[0].inflow.commodity().as_str(), "AUD");
            assert_eq!(buckets[0].outflow.value(), rust_decimal::Decimal::ZERO);
            assert_eq!(buckets[0].outflow.commodity().as_str(), "AUD");
            // Second bucket: May (inflow 0, outflow 200)
            assert_eq!(buckets[1].start.to_string(), "2026-05-01");
            assert_eq!(buckets[1].inflow.value(), rust_decimal::Decimal::ZERO);
            assert_eq!(buckets[1].inflow.commodity().as_str(), "AUD");
            assert_eq!(buckets[1].outflow.value(), dec!(200.00));
            assert_eq!(buckets[1].outflow.commodity().as_str(), "AUD");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_includes_all_reconciliation_states(pool: sqlx::SqlitePool) {
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
            .description("Unreconciled")
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
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        tx_svc.create(tx).await.expect("create should succeed");

        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        // Unreconciled transactions are included in balances.
        assert_eq!(balance.value(), dec!(100));
        assert_eq!(balance.commodity().as_str(), "AUD");
    }

    /// Seeds `wallet` with one transaction per `(date, amount)` pair in AUD,
    /// each balanced against a counter "Other" account.
    async fn seed_postings_aud(
        pool: &sqlx::SqlitePool,
        wallet: &AccountId,
        pairs: &[(jiff::civil::Date, Decimal)],
    ) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let other = acct_svc
            .create()
            .name("Other")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Other account should succeed");

        let tx_svc = crate::transaction::Service::new(pool.clone());
        for &(date, amount) in pairs {
            let tx = Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(date)
                .description("Seed")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(wallet.clone())
                        .amount(Amount::new(amount, CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(other.clone())
                        .amount(Amount::new(
                            Decimal::ZERO
                                .checked_sub(amount)
                                .expect("negation should not overflow"),
                            CommodityCode::new("AUD"),
                        ))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build();
            tx_svc
                .create(tx)
                .await
                .expect("seed transaction should succeed");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_period_stats_windows_flows_and_balances(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");

        seed_postings_aud(
            &pool,
            &acc,
            &[
                (date(2026, 5, 20), dec!(100)),
                (date(2026, 6, 10), dec!(-30)),
                (date(2026, 6, 20), dec!(50)),
            ],
        )
        .await;

        let engine = Engine::new(pool.clone());
        let s = engine
            .account_period_stats(&acc, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("account_period_stats should succeed");

        assert_eq!(s.income.value(), dec!(50));
        assert_eq!(s.expenses.value(), dec!(30));
        assert_eq!(s.net.value(), dec!(20));
        assert_eq!(s.opening.value(), dec!(100));
        assert_eq!(s.closing.value(), dec!(120));
        assert_eq!(s.tx_count, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_period_stats_tx_count_is_distinct_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");
        let other = acct_svc
            .create()
            .name("Other")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Other account should succeed");

        // A single transaction with TWO postings to the wallet (a within-account
        // split). tx_count must count the transaction once, matching the
        // register's row count — not the two commodity-scoped postings.
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 6, 10))
            .description("Split")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(wallet.clone())
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(wallet.clone())
                    .amount(Amount::new(dec!(40), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(other.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        crate::transaction::Service::new(pool.clone())
            .create(tx)
            .await
            .expect("seed split transaction should succeed");

        let engine = Engine::new(pool.clone());
        let s = engine
            .account_period_stats(&wallet, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("account_period_stats should succeed");

        // One distinct transaction, even though it has two wallet postings.
        assert_eq!(s.tx_count, 1);
        // Flows still aggregate every posting: 60 + 40 in.
        assert_eq!(s.income.value(), dec!(100));
    }

    #[rstest]
    #[case::weekly(
        Period::Weekly,
        date(2025, 2, 10),
        date(2025, 1, 27),
        date(2025, 2, 17)
    )]
    #[case::monthly(Period::Monthly, date(2025, 3, 15), date(2025, 1, 1), date(2025, 4, 1))]
    #[case::quarterly(
        Period::Quarterly,
        date(2025, 8, 20),
        date(2025, 1, 1),
        date(2025, 10, 1)
    )]
    #[case::calendar_year(
        Period::CalendarYear,
        date(2025, 5, 4),
        date(2023, 1, 1),
        date(2026, 1, 1)
    )]
    fn bucket_ranges_are_contiguous_oldest_first(
        #[case] period: Period,
        #[case] as_of: Date,
        #[case] expected_first_start: Date,
        #[case] expected_last_end: Date,
    ) {
        let ranges = super::bucket_ranges(&period, NonZeroUsize::new(3).expect("3 > 0"), as_of);
        assert_eq!(ranges.len(), 3);

        let first = ranges.first().expect("three buckets");
        let last = ranges.last().expect("three buckets");
        assert_eq!(first.0, expected_first_start);
        assert_eq!(last.1, expected_last_end);
        // Newest bucket contains `as_of`.
        assert!(last.0 <= as_of && as_of < last.1);

        for pair in ranges.windows(2) {
            let [earlier, later] = pair else {
                unreachable!("windows(2) always yields two elements")
            };
            assert!(earlier.0 < later.0, "buckets must be oldest-first");
            assert_eq!(earlier.1, later.0, "buckets must be contiguous");
        }
    }

    /// Anchors the WASM-side mirror of this crate's calendar snapping.
    ///
    /// `bc-ui`'s `coverage_count` re-implements [`super::bucket_ranges`]'s
    /// snapping rules because `bc-core` is absent from the WASM bundle. These
    /// cases pin the `(period, count, as_of)` triples that `bc-ui`'s own tests
    /// assume, so a change to the snapping here fails on this side too.
    #[rstest]
    #[case::daily(
        Period::Custom { days: Some(1), weeks: None, months: None },
        20,
        date(2025, 2, 10),
        date(2025, 1, 22)
    )]
    #[case::weekly(Period::Weekly, 7, date(2025, 2, 10), date(2025, 1, 1))]
    #[case::monthly(Period::Monthly, 8, date(2025, 8, 19), date(2025, 1, 15))]
    #[case::quarterly(Period::Quarterly, 5, date(2025, 8, 19), date(2024, 8, 1))]
    #[case::calendar_year(Period::CalendarYear, 3, date(2025, 8, 19), date(2023, 6, 1))]
    fn bucket_ranges_cover_span_start(
        #[case] period: Period,
        #[case] count: usize,
        #[case] as_of: Date,
        #[case] span_start: Date,
    ) {
        let ranges =
            super::bucket_ranges(&period, NonZeroUsize::new(count).expect("count > 0"), as_of);
        let first = ranges.first().expect("at least one bucket");
        let last = ranges.last().expect("at least one bucket");
        assert!(
            first.0 <= span_start,
            "oldest bucket start {:?} must reach span start {span_start:?}",
            first.0
        );
        assert!(as_of < last.1, "newest bucket must contain as_of");
    }

    /// The #354 repro: `Expenses:Food +50 / Assets:Bank <elided>`.
    ///
    /// Before the fix `Assets:Bank` read zero while `Expenses:Food` read +50.
    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_moves_its_account_balance(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food', 'tx_e1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank', 'tx_e1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .balance_for(&food, "AUD")
                .await
                .expect("food balance")
                .value(),
            dec!(50.00),
        );
        assert_eq!(
            engine
                .balance_for(&bank, "AUD")
                .await
                .expect("bank balance")
                .value(),
            dec!(-50.00),
            "the elided leg must absorb the residual",
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn ambiguous_transaction_contributes_no_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");
        let fun = acct_svc
            .create()
            .name("Fun")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Fun");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e2', '2026-01-01', 'Ambiguous', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food2', 'tx_e2', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank2', 'tx_e2', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert first elided leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_fun2', 'tx_e2', ?, NULL, NULL, 2)")
            .bind(fun.to_string()).execute(&pool).await.expect("insert second elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .balance_for(&bank, "AUD")
                .await
                .expect("bank balance")
                .value(),
            Decimal::ZERO,
            "a residual split across two elided legs is not attributable",
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_counts_toward_period_flows(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e3', '2026-01-15', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food3', 'tx_e3', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank3', 'tx_e3', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&bank, "AUD", date(2026, 1, 1), date(2026, 2, 1))
            .await
            .expect("flows");

        assert_eq!(inflow.value(), Decimal::ZERO);
        assert_eq!(outflow.value(), dec!(50.00), "the residual is an outflow");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_include_elided_residuals(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_d1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_d', 'tx_d1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_d', 'tx_d1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());
        let balances = engine.default_balances().await.expect("default balances");

        // The bank account's only posting is elided, so its commodity is
        // inferable only from the residual.
        let bank_balance = balances.get(&bank).expect("bank must appear");
        assert_eq!(bank_balance.value(), dec!(-50.00));
        assert_eq!(bank_balance.commodity().as_str(), "AUD");
        assert_eq!(
            balances.get(&food).expect("food must appear").value(),
            dec!(50.00)
        );
    }

    /// The invariant the ledger migration reconciles against: every commodity
    /// closes to zero across all accounts once residuals are derived.
    #[sqlx::test(migrations = "./migrations")]
    async fn all_accounts_sum_to_zero_per_commodity(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");
        let rent = acct_svc
            .create()
            .name("Rent")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Rent");

        // Three transactions, each with an elided bank leg — the Beancount idiom.
        for (n, account, amount) in [
            ("1", &food, "50.00"),
            ("2", &rent, "1200.00"),
            ("3", &food, "25.50"),
        ] {
            sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES (?, '2026-01-01', 'Test', 'unreconciled', '2026-01-01T00:00:00Z')")
                .bind(format!("tx_z{n}")).execute(&pool).await.expect("insert transaction");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, ?, 'AUD', 0)")
                .bind(format!("p_c{n}")).bind(format!("tx_z{n}")).bind(account.to_string()).bind(amount)
                .execute(&pool).await.expect("insert concrete leg");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, NULL, NULL, 1)")
                .bind(format!("p_e{n}")).bind(format!("tx_z{n}")).bind(bank.to_string())
                .execute(&pool).await.expect("insert elided leg");
        }

        let engine = Engine::new(pool.clone());
        let mut total = Decimal::ZERO;
        for account in [&bank, &food, &rent] {
            total = total
                .checked_add(
                    engine
                        .balance_for(account, "AUD")
                        .await
                        .expect("balance")
                        .value(),
                )
                .expect("no overflow");
        }

        assert_eq!(
            total,
            Decimal::ZERO,
            "AUD must close to zero across all accounts"
        );

        // `default_balances` exercises a different path than `balance_for`: it
        // infers each account's commodity (tier-3 falls back to the residual
        // for Bank, whose every posting is elided) and intersects against the
        // set of non-archived accounts. It must agree with the direct sum.
        let default_balances = engine.default_balances().await.expect("default balances");
        let default_total = [&bank, &food, &rent]
            .into_iter()
            .try_fold(Decimal::ZERO, |acc, account| {
                let value = default_balances
                    .get(account)
                    .unwrap_or_else(|| panic!("{account} must appear in default_balances"))
                    .value();
                acc.checked_add(value)
            })
            .expect("no overflow");

        assert_eq!(
            default_total,
            Decimal::ZERO,
            "default_balances must also close to zero across all accounts"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_commodity_falls_back_to_the_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_c1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_c', 'tx_c1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_c', 'tx_c1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .default_commodity_for(&bank)
                .await
                .expect("commodity"),
            Some("AUD".to_owned()),
        );
    }

    /// Elided legs must not form their own `GROUP BY` bucket in the tier-2
    /// "most-used posting commodity" subselect. When they outnumber every
    /// stored commodity, an unguarded `GROUP BY p.commodity` returns the NULL
    /// group, `COALESCE` yields NULL, and tier 3 fires on an account that has
    /// stored commodities — dropping every concrete posting as "non-default".
    #[sqlx::test(migrations = "./migrations")]
    async fn stored_commodity_wins_over_more_numerous_elided_legs(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        // Bank holds one stored USD leg and three elided legs whose siblings are
        // in AUD, so the NULL group (3) outnumbers the USD group (1).
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_s0', '2026-01-01', 'Stored', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_s', 'tx_s0', ?, '-20.00', 'USD', 0)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert stored leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_s', 'tx_s0', ?, '20.00', 'USD', 1)")
            .bind(food.to_string()).execute(&pool).await.expect("insert stored sibling");

        for (i, tx) in ["tx_s1", "tx_s2", "tx_s3"].into_iter().enumerate() {
            sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES (?, '2026-01-02', 'Groceries', 'unreconciled', '2026-01-02T00:00:00Z')")
                .bind(tx).execute(&pool).await.expect("insert transaction");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, '50.00', 'AUD', 0)")
                .bind(format!("p_food_{i}")).bind(tx).bind(food.to_string())
                .execute(&pool).await.expect("insert concrete leg");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, NULL, NULL, 1)")
                .bind(format!("p_bank_{i}")).bind(tx).bind(bank.to_string())
                .execute(&pool).await.expect("insert elided leg");
        }

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .default_commodity_for(&bank)
                .await
                .expect("commodity"),
            Some("USD".to_owned()),
            "a stored commodity must beat the elided legs' NULL group",
        );

        let balances = engine.default_balances().await.expect("default balances");
        assert_eq!(
            balances
                .get(&bank)
                .map(|a| (a.value(), a.commodity().as_str().to_owned())),
            Some((dec!(-20.00), "USD".to_owned())),
            "the stored USD leg must not be dropped as a non-default commodity",
        );
    }

    /// An archived account whose only postings are elided must not leak into
    /// `default_balances` via the residual fallback: `Residuals::load` has no
    /// `archived_at` filter, so the fallback must intersect against the
    /// active-account set explicitly.
    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_excludes_archived_account_via_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_a1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_a', 'tx_a1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_a', 'tx_a1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        acct_svc
            .archive(&bank, Cascade::Reject)
            .await
            .expect("archive Bank");

        let engine = Engine::new(pool.clone());
        let balances = engine.default_balances().await.expect("default balances");

        assert_eq!(
            balances.get(&bank),
            None,
            "archived account must not appear even though its only leg is elided"
        );
        assert_eq!(
            balances.get(&food).expect("food must appear").value(),
            dec!(50.00)
        );
    }

    /// Inserts a transaction with the given id and date.
    async fn insert_tx(pool: &sqlx::SqlitePool, id: &str, date: &str) {
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, 'Test', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(date)
        .execute(pool)
        .await
        .expect("insert transaction");
    }

    /// Inserts a posting; `amount`/`commodity` are `None` for an elided leg.
    async fn insert_posting(
        pool: &sqlx::SqlitePool,
        id: &str,
        tx_id: &str,
        account_id: &str,
        amount: Option<&str>,
        commodity: Option<&str>,
        position: i64,
    ) {
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(tx_id)
        .bind(account_id)
        .bind(amount)
        .bind(commodity)
        .bind(position)
        .execute(pool)
        .await
        .expect("insert posting");
    }

    /// Creates an account and returns its id.
    async fn make_account(
        pool: &sqlx::SqlitePool,
        name: &str,
        account_type: AccountType,
    ) -> bc_models::AccountId {
        crate::account::Service::new(pool.clone())
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    /// D1: inserting a posting copies its transaction's date.
    #[sqlx::test(migrations = "./migrations")]
    async fn posting_date_is_populated_on_insert(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_d1", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_d1",
            "tx_d1",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d1'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-01-15".to_owned()));
    }

    /// D2: amending a transaction's date moves every one of its postings.
    #[sqlx::test(migrations = "./migrations")]
    async fn amending_a_transaction_date_moves_its_postings(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_d2", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_d2",
            "tx_d2",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        sqlx::query("UPDATE transactions SET date = '2026-03-01' WHERE id = 'tx_d2'")
            .execute(&pool)
            .await
            .expect("amend date");

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d2'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-03-01".to_owned()));
    }

    /// D3: re-pointing a posting at another transaction resyncs its date.
    #[sqlx::test(migrations = "./migrations")]
    async fn reparenting_a_posting_resyncs_its_date(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_a", "2026-01-15").await;
        insert_tx(&pool, "tx_b", "2026-06-30").await;
        insert_posting(
            &pool,
            "p_d3",
            "tx_a",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        sqlx::query("UPDATE postings SET transaction_id = 'tx_b' WHERE id = 'p_d3'")
            .execute(&pool)
            .await
            .expect("reparent");

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d3'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-06-30".to_owned()));
    }

    /// D4: no posting's denormalised date may diverge from its transaction's.
    ///
    /// `postings.date` cannot be `NOT NULL`, because an `AFTER INSERT` trigger populates
    /// it after the row already exists. This assertion is the guard in its place: a write
    /// path the triggers miss leaves a NULL date, which would silently vanish from every
    /// windowed query rather than failing.
    #[sqlx::test(migrations = "./migrations")]
    async fn every_posting_date_matches_its_transaction(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_i1", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_i1",
            "tx_i1",
            &wallet.to_string(),
            Some("-10.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_i2", "tx_i1", &food.to_string(), None, None, 1).await;
        sqlx::query("UPDATE transactions SET date = '2026-02-20' WHERE id = 'tx_i1'")
            .execute(&pool)
            .await
            .expect("amend date");

        let (divergent,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.date IS NOT t.date",
        )
        .fetch_one(&pool)
        .await
        .expect("integrity check");

        assert_eq!(divergent, 0);
    }

    /// E2: the concrete-leg window query must be fully index-driven.
    ///
    /// The date range has to be a term *inside* the index, not a post-join filter.
    #[sqlx::test(migrations = "./migrations")]
    async fn concrete_window_query_is_index_driven(pool: sqlx::SqlitePool) {
        let plan = query_plan(&pool, WINDOW_CONCRETE_SQL).await;

        let joined = plan.join("\n");
        assert!(
            joined.contains("idx_postings_account_commodity_date"),
            "concrete window query does not use the composite index:\n{joined}"
        );
        assert!(
            joined.contains("date>?") && joined.contains("date<?"),
            "date range is not a term inside the index:\n{joined}"
        );
    }

    /// E2: the elided-leg window query must be index-driven too.
    #[sqlx::test(migrations = "./migrations")]
    async fn elided_window_query_is_index_driven(pool: sqlx::SqlitePool) {
        let plan = query_plan(&pool, WINDOW_ELIDED_SQL).await;

        let joined = plan.join("\n");
        assert!(
            joined.contains("idx_postings_account_date"),
            "elided window query does not use idx_postings_account_date:\n{joined}"
        );
    }

    /// Finding 1: the transaction-count query must be sargable on `postings`, not
    /// materialise the account's full history into a temp b-tree before probing
    /// `transactions` by id.
    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_count_query_is_index_driven(pool: sqlx::SqlitePool) {
        let plan = query_plan(&pool, TX_COUNT_SQL).await;

        let joined = plan.join("\n");
        assert!(
            !joined.contains("SCAN"),
            "transaction count query scans a table instead of seeking an index:\n{joined}"
        );
        assert!(
            joined.contains("idx_postings_account_date"),
            "transaction count query does not use idx_postings_account_date:\n{joined}"
        );
    }

    /// Finding 1: a transaction with multiple postings to the same account must count
    /// once, and boundary transactions on `from`/`to` must land on the correct side.
    ///
    /// A missing `DISTINCT` in the rewritten `count_transactions_in_range` query would
    /// double-count the multi-posting transaction below; an off-by-one in the boundary
    /// comparison would place either boundary transaction on the wrong side.
    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_count_deduplicates_multi_posting_transactions(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;

        // Two postings to `wallet` in the same transaction, inside the window — a
        // missing DISTINCT would count this transaction twice.
        insert_tx(&pool, "tx_multi", "2026-04-15").await;
        insert_posting(
            &pool,
            "p_multi_a",
            "tx_multi",
            &wallet.to_string(),
            Some("-40.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_multi_b",
            "tx_multi",
            &wallet.to_string(),
            Some("-10.00"),
            Some("AUD"),
            1,
        )
        .await;
        insert_posting(
            &pool,
            "p_multi_c",
            "tx_multi",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            2,
        )
        .await;

        // On the inclusive lower boundary.
        insert_tx(&pool, "tx_lo", "2026-04-01").await;
        insert_posting(
            &pool,
            "p_lo",
            "tx_lo",
            &wallet.to_string(),
            Some("1.00"),
            Some("AUD"),
            0,
        )
        .await;

        // On the exclusive upper boundary — must be excluded.
        insert_tx(&pool, "tx_hi", "2026-05-01").await;
        insert_posting(
            &pool,
            "p_hi",
            "tx_hi",
            &wallet.to_string(),
            Some("1.00"),
            Some("AUD"),
            0,
        )
        .await;

        let engine = Engine::new(pool.clone());
        let stats = engine
            .account_period_stats(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("stats");

        // tx_multi counts once despite two postings, plus tx_lo; tx_hi is excluded.
        assert_eq!(stats.tx_count, 2);
    }

    /// Finding 5: `balance_for` must not join `transactions` — nothing from it is
    /// selected or filtered, so the join was a wasted index probe per posting.
    #[sqlx::test(migrations = "./migrations")]
    async fn balance_for_query_does_not_join_transactions(pool: sqlx::SqlitePool) {
        let plan = query_plan(&pool, BALANCE_SQL).await;

        let joined = plan.join("\n");
        assert_eq!(
            plan.len(),
            1,
            "balance_for query should be a single index search, not a join:\n{joined}"
        );
        assert!(
            joined.contains("idx_postings_account_commodity_date"),
            "balance_for query does not use idx_postings_account_commodity_date:\n{joined}"
        );
    }

    /// C1: splitting a window anywhere must not change the total.
    ///
    /// The single strongest invariant here: it fails if the opening query's upper bound
    /// and the in-window query's lower bound ever disagree.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal sums in a test assertion, not production arithmetic"
    )]
    async fn period_net_is_additive_across_a_split(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [
            ("1", "2026-01-05", "10.00"),
            ("2", "2026-02-20", "20.00"),
            ("3", "2026-04-10", "30.00"),
            ("4", "2026-05-25", "40.00"),
        ] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let from = date(2026, 1, 1);
        let until = date(2026, 6, 1);
        for split in [date(2026, 1, 1), date(2026, 3, 15), date(2026, 6, 1)] {
            let whole = engine
                .account_period_stats(&bank, "AUD", from, until)
                .await
                .expect("whole");
            let left = engine
                .account_period_stats(&bank, "AUD", from, split)
                .await
                .expect("left");
            let right = engine
                .account_period_stats(&bank, "AUD", split, until)
                .await
                .expect("right");

            assert_eq!(
                whole.net.value(),
                left.net.value() + right.net.value(),
                "net is not additive across {split}"
            );
            assert_eq!(
                right.opening.value(),
                left.opening.value() + left.net.value(),
                "opening/net disagree at {split}",
            );
            assert_eq!(
                whole.closing.value(),
                right.closing.value(),
                "closing disagrees across {split}"
            );
        }
    }

    /// C2: opening plus net equals closing.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal sum in a test assertion, not production arithmetic"
    )]
    async fn opening_plus_net_equals_closing(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [("1", "2025-11-01", "100.00"), ("2", "2026-02-20", "20.00")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let stats = Engine::new(pool.clone())
            .account_period_stats(&bank, "AUD", date(2026, 1, 1), date(2026, 6, 1))
            .await
            .expect("stats");

        assert_eq!(stats.opening.value(), dec!(-100.00));
        assert_eq!(stats.net.value(), dec!(-20.00));
        assert_eq!(
            stats.closing.value(),
            stats.opening.value() + stats.net.value()
        );
    }

    /// C3: the windowed path agrees with the unwindowed one.
    ///
    /// `balance_for` is untouched by this change, so it is a clean oracle.
    #[sqlx::test(migrations = "./migrations")]
    async fn full_span_closing_equals_balance_for(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [("1", "2020-03-04", "12.34"), ("2", "2026-02-20", "56.78")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let stats = engine
            .account_period_stats(&bank, "AUD", jiff::civil::Date::MIN, jiff::civil::Date::MAX)
            .await
            .expect("stats");
        let direct = engine.balance_for(&bank, "AUD").await.expect("balance_for");

        assert_eq!(stats.closing.value(), direct.value());
    }

    /// C4: every account's period net sums to zero, per commodity, for any window.
    ///
    /// Catches a residual dropped for one account but not its counterparty — a shape the
    /// per-account tests cannot see. Includes a USD leg alongside the AUD ones so the "per
    /// commodity" claim is actually exercised: with only one commodity in the fixture, a bug
    /// that crossed commodities (e.g. summing a USD residual into an AUD total) could not
    /// show up here.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal accumulation in a test assertion, not production arithmetic"
    )]
    async fn period_nets_sum_to_zero_across_all_accounts(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        let fun = make_account(&pool, "Fun", AccountType::Expense).await;
        for (n, acct, day, amount) in [
            ("1", &food, "2026-02-20", "20.00"),
            ("2", &fun, "2026-03-05", "35.00"),
        ] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_c_{n}"),
                &tx,
                &acct.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        // A USD-denominated transaction, elided the same way, so the USD total is
        // exercised independently of the AUD total above.
        insert_tx(&pool, "tx_usd", "2026-04-10").await;
        insert_posting(
            &pool,
            "p_c_usd",
            "tx_usd",
            &fun.to_string(),
            Some("15.00"),
            Some("USD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_bank_usd",
            "tx_usd",
            &bank.to_string(),
            None,
            None,
            1,
        )
        .await;

        let engine = Engine::new(pool.clone());
        for commodity in ["AUD", "USD"] {
            let mut total = Decimal::ZERO;
            for acct in [&bank, &food, &fun] {
                let stats = engine
                    .account_period_stats(acct, commodity, date(2026, 1, 1), date(2026, 6, 1))
                    .await
                    .expect("stats");
                total += stats.net.value();
            }
            assert_eq!(
                total,
                Decimal::ZERO,
                "commodity {commodity} did not net to zero"
            );
        }
    }

    /// F: the genesis sentinel sorts below every real date.
    ///
    /// Dates are TEXT and compared lexicographically, so `Date::MIN` (-9999-01-01) has to
    /// stringify to something that sorts below a four-digit positive year. The test
    /// asserts the behaviour rather than pinning the format, so it survives a jiff
    /// formatting change while still catching a real ordering break.
    #[sqlx::test(migrations = "./migrations")]
    async fn opening_balance_includes_the_earliest_transactions(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_ancient", "0001-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_ancient",
            &food.to_string(),
            Some("7.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_bank",
            "tx_ancient",
            &bank.to_string(),
            None,
            None,
            1,
        )
        .await;

        let stats = Engine::new(pool.clone())
            .account_period_stats(&bank, "AUD", date(2026, 1, 1), date(2026, 6, 1))
            .await
            .expect("stats");

        assert_eq!(stats.opening.value(), dec!(-7.00));
    }

    /// A3: the window is half-open on the elided path as well as the concrete one.
    ///
    /// `posting_flows_respects_date_boundary` covers only concrete legs. This is its
    /// elided twin, and it is what catches `<=` versus `<` drift between the driving
    /// elided query and the residual subquery.
    #[sqlx::test(migrations = "./migrations")]
    async fn elided_legs_respect_the_half_open_boundary(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        // One transaction on the inclusive lower bound, one on the exclusive upper bound.
        for (n, day) in [("lo", "2026-04-01"), ("hi", "2026-05-01")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some("50.00"),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&bank, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        // Only the 2026-04-01 transaction is in window; its elided bank leg absorbs -50.
        assert_eq!(inflow.value(), dec!(0));
        assert_eq!(outflow.value(), dec!(50.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_group_account_contributes_zero_without_warning(pool: SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let group = accounts
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .call()
            .await
            .expect("create the group account");

        let engine = Engine::new(pool.clone());
        let total = engine.net_worth("AUD").await.expect("net worth");

        assert_eq!(
            total.value(),
            Decimal::ZERO,
            "a Group account holds no postings, so it contributes nothing"
        );
        assert_eq!(
            engine
                .balance_for(&group, "AUD")
                .await
                .expect("balance")
                .value(),
            Decimal::ZERO
        );
    }
}
