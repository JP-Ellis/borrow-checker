//! Budget calculation engine: actuals, rollover, and budget status.

use sqlx::SqlitePool;

// MARK: BudgetService

/// Internal row type returned from the `budgets` table.
#[derive(sqlx::FromRow)]
struct BudgetRow {
    /// Raw budget ID string.
    id: String,
    /// Raw account ID string this budget is anchored to.
    account_id: String,
    /// Optional raw tag ID string for sub-budget filtering.
    tag_filter: Option<String>,
    /// Optional display name override.
    name: Option<String>,
    /// Decimal string for the allocation target amount; NULL = tracking-only.
    target_amount: Option<String>,
    /// Commodity code for the target amount; NULL when `target_amount` is NULL.
    target_currency: Option<String>,
    /// JSON-serialised [`bc_models::Period`].
    period: String,
    /// Snake-case rollover policy string.
    rollover: String,
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// ISO 8601 archive timestamp; NULL if still active.
    archived_at: Option<String>,
}

impl TryFrom<BudgetRow> for bc_models::Budget {
    type Error = crate::BcError;

    /// Converts a raw database row into a domain [`bc_models::Budget`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: BudgetRow) -> crate::BcResult<Self> {
        let id = row
            .id
            .parse::<bc_models::BudgetId>()
            .map_err(|e| crate::BcError::BadData(format!("invalid budget id '{}': {e}", row.id)))?;

        let account_id = row
            .account_id
            .parse::<bc_models::AccountId>()
            .map_err(|e| {
                crate::BcError::BadData(format!("invalid account_id '{}': {e}", row.account_id))
            })?;

        let tag_filter = row
            .tag_filter
            .as_deref()
            .map(|s| {
                s.parse::<bc_models::TagId>()
                    .map_err(|e| crate::BcError::BadData(format!("invalid tag_filter '{s}': {e}")))
            })
            .transpose()?;

        let target = match (row.target_amount, row.target_currency) {
            (Some(amt_str), Some(cur_str)) => {
                let qty = amt_str.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid target_amount '{amt_str}': {e}"))
                })?;
                Some(bc_models::Amount::new(
                    qty,
                    bc_models::CommodityCode::new(&cur_str),
                ))
            }
            (None, None) => None,
            _ => {
                return Err(crate::BcError::BadData(
                    "target_amount and target_currency must both be set or both NULL".to_owned(),
                ));
            }
        };

        let period: bc_models::Period = serde_json::from_str(&row.period).map_err(|e| {
            crate::BcError::BadData(format!("invalid period '{}': {e}", row.period))
        })?;

        let rollover = crate::db::from_db_str::<bc_models::RolloverPolicy>(&row.rollover)?;

        let created_at = row.created_at.parse::<jiff::Timestamp>().map_err(|e| {
            crate::BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;

        let archived_at = row
            .archived_at
            .as_deref()
            .map(|s| {
                s.parse::<jiff::Timestamp>()
                    .map_err(|e| crate::BcError::BadData(format!("invalid archived_at '{s}': {e}")))
            })
            .transpose()?;

        Ok(bc_models::Budget::builder()
            .id(id)
            .account_id(account_id)
            .maybe_tag_filter(tag_filter)
            .maybe_name(row.name)
            .maybe_target(target)
            .period(period)
            .rollover(rollover)
            .created_at(created_at)
            .maybe_archived_at(archived_at)
            .build())
    }
}

/// Internal row type for budget allocation queries.
#[derive(sqlx::FromRow)]
struct BudgetAllocationRow {
    /// Raw allocation ID string.
    id: String,
    /// Raw budget ID string this allocation belongs to.
    budget_id: String,
    /// YYYY-MM-DD canonical period start date.
    period_start: String,
    /// Decimal string of the allocated amount.
    amount: String,
    /// Commodity code for the allocation amount.
    commodity: String,
    /// ISO 8601 creation timestamp.
    created_at: String,
}

impl TryFrom<BudgetAllocationRow> for bc_models::BudgetAllocation {
    type Error = crate::BcError;

    /// Converts a raw database row into a domain [`bc_models::BudgetAllocation`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: BudgetAllocationRow) -> crate::BcResult<Self> {
        let id = row
            .id
            .parse::<bc_models::BudgetAllocationId>()
            .map_err(|e| {
                crate::BcError::BadData(format!("invalid budget_alloc id '{}': {e}", row.id))
            })?;
        let budget_id = row.budget_id.parse::<bc_models::BudgetId>().map_err(|e| {
            crate::BcError::BadData(format!("invalid budget_id '{}': {e}", row.budget_id))
        })?;
        let period_start = row.period_start.parse::<jiff::civil::Date>().map_err(|e| {
            crate::BcError::BadData(format!("invalid period_start '{}': {e}", row.period_start))
        })?;
        let value = row.amount.parse::<bc_models::Decimal>().map_err(|e| {
            crate::BcError::BadData(format!("invalid amount '{}': {e}", row.amount))
        })?;
        let created_at = row.created_at.parse::<jiff::Timestamp>().map_err(|e| {
            crate::BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;

        Ok(bc_models::BudgetAllocation::builder()
            .id(id)
            .budget_id(budget_id)
            .period_start(period_start)
            .amount(bc_models::Amount::new(
                value,
                bc_models::CommodityCode::new(row.commodity),
            ))
            .created_at(created_at)
            .build())
    }
}

/// Budget CRUD and allocation service.
#[derive(Debug, Clone)]
pub struct BudgetService {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

#[bon::bon]
impl BudgetService {
    /// Creates a new [`BudgetService`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // MARK: Budget management

    /// Creates a new budget anchored to `account_id`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `rollover` is `CapAtTarget` and
    /// `target` is `None`.
    /// Returns [`crate::BcError`] on event append or database insert failure (including
    /// uniqueness constraint violations — duplicate untagged or tagged budget).
    #[builder]
    #[inline]
    pub async fn create(
        &self,
        account_id: bc_models::AccountId,
        tag_filter: Option<bc_models::TagId>,
        #[builder(into)] name: Option<String>,
        target: Option<bc_models::Amount>,
        period: bc_models::Period,
        rollover: bc_models::RolloverPolicy,
    ) -> crate::BcResult<bc_models::Budget> {
        if rollover == bc_models::RolloverPolicy::CapAtTarget && target.is_none() {
            return Err(crate::BcError::InvalidInput(
                "CapAtTarget rollover policy requires a target amount".to_owned(),
            ));
        }

        let id = bc_models::BudgetId::new();
        let now = jiff::Timestamp::now();

        let event = crate::events::Event::BudgetCreated {
            budget_id: id.clone(),
            account_id: account_id.clone(),
            tag_filter: tag_filter.clone(),
            name: name.clone(),
            target: target.clone(),
            period: period.clone(),
            rollover,
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        let period_json = serde_json::to_string(&period)?;
        let rollover_db = crate::db::to_db_str(rollover)?;
        let (target_amount, target_currency) = target.as_ref().map_or((None, None), |a| {
            (Some(a.value().to_string()), Some(a.commodity().to_string()))
        });

        sqlx::query(
            "INSERT INTO budgets \
             (id, account_id, tag_filter, name, target_amount, target_currency, \
              period, rollover, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(account_id.to_string())
        .bind(tag_filter.as_ref().map(ToString::to_string))
        .bind(&name)
        .bind(&target_amount)
        .bind(&target_currency)
        .bind(&period_json)
        .bind(&rollover_db)
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(budget_id = %id, %account_id, "budget created");

        Ok(bc_models::Budget::builder()
            .id(id)
            .account_id(account_id)
            .maybe_tag_filter(tag_filter)
            .maybe_name(name)
            .maybe_target(target)
            .period(period)
            .rollover(rollover)
            .created_at(now)
            .build())
    }

    /// Lists all active (non-archived) budgets, ordered by `account_id` then `tag_filter`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn list(&self) -> crate::BcResult<Vec<bc_models::Budget>> {
        let rows = sqlx::query_as::<_, BudgetRow>(
            "SELECT id, account_id, tag_filter, name, target_amount, target_currency, \
              period, rollover, created_at, archived_at \
             FROM budgets \
             WHERE archived_at IS NULL \
             ORDER BY account_id ASC, tag_filter ASC NULLS FIRST",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(bc_models::Budget::try_from).collect()
    }

    /// Lists all active budgets anchored to a specific account.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_for_account(
        &self,
        account_id: &bc_models::AccountId,
    ) -> crate::BcResult<Vec<bc_models::Budget>> {
        let rows = sqlx::query_as::<_, BudgetRow>(
            "SELECT id, account_id, tag_filter, name, target_amount, target_currency, \
              period, rollover, created_at, archived_at \
             FROM budgets \
             WHERE account_id = ? AND archived_at IS NULL \
             ORDER BY tag_filter ASC NULLS FIRST",
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(bc_models::Budget::try_from).collect()
    }

    /// Fetches an active budget by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no active budget with that ID exists.
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn get(&self, id: &bc_models::BudgetId) -> crate::BcResult<bc_models::Budget> {
        let row = sqlx::query_as::<_, BudgetRow>(
            "SELECT id, account_id, tag_filter, name, target_amount, target_currency, \
              period, rollover, created_at, archived_at \
             FROM budgets \
             WHERE id = ? AND archived_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| crate::BcError::NotFound(id.to_string()))?;

        bc_models::Budget::try_from(row)
    }

    /// Archives a budget by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no active budget with that ID exists.
    /// Returns [`crate::BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &bc_models::BudgetId) -> crate::BcResult<()> {
        let now = jiff::Timestamp::now();
        let event = crate::events::Event::BudgetArchived {
            budget_id: id.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        let result =
            sqlx::query("UPDATE budgets SET archived_at = ? WHERE id = ? AND archived_at IS NULL")
                .bind(now.to_string())
                .bind(id.to_string())
                .execute(&mut *db_tx)
                .await?;

        if result.rows_affected() == 0 {
            return Err(crate::BcError::NotFound(id.to_string()));
        }

        db_tx.commit().await?;
        tracing::info!(budget_id = %id, "budget archived");
        Ok(())
    }

    // MARK: Allocation management

    /// Records (or replaces) the allocation for `budget_id` in the period starting on `period_start`.
    ///
    /// If an allocation already exists for that period, it is replaced (upsert).
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if the budget does not exist or is archived.
    /// Returns [`crate::BcError`] on event append or database failure.
    #[inline]
    pub async fn allocate(
        &self,
        budget_id: &bc_models::BudgetId,
        period_start: jiff::civil::Date,
        amount: bc_models::Amount,
    ) -> crate::BcResult<bc_models::BudgetAllocation> {
        // Verify budget exists.
        let _budget = self.get(budget_id).await?;

        let id = bc_models::BudgetAllocationId::new();
        let now = jiff::Timestamp::now();

        let event = crate::events::Event::BudgetAllocated {
            budget_id: budget_id.clone(),
            period_start,
            amount: amount.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO budget_allocations \
             (id, budget_id, period_start, amount, commodity, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (budget_id, period_start) \
             DO UPDATE SET id = excluded.id, amount = excluded.amount, \
                           commodity = excluded.commodity, created_at = excluded.created_at",
        )
        .bind(id.to_string())
        .bind(budget_id.to_string())
        .bind(period_start.to_string())
        .bind(amount.value().to_string())
        .bind(amount.commodity().as_str())
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(budget_id = %budget_id, %period_start, "budget allocated");

        Ok(bc_models::BudgetAllocation::builder()
            .id(id)
            .budget_id(budget_id.clone())
            .period_start(period_start)
            .amount(amount)
            .created_at(now)
            .build())
    }

    /// Retrieves the allocation for a budget in a specific period, if one exists.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn get_allocation(
        &self,
        budget_id: &bc_models::BudgetId,
        period_start: jiff::civil::Date,
    ) -> crate::BcResult<Option<bc_models::BudgetAllocation>> {
        let row: Option<BudgetAllocationRow> = sqlx::query_as(
            "SELECT id, budget_id, period_start, amount, commodity, created_at \
             FROM budget_allocations \
             WHERE budget_id = ? AND period_start = ?",
        )
        .bind(budget_id.to_string())
        .bind(period_start.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(bc_models::BudgetAllocation::try_from).transpose()
    }
}

// MARK: BudgetStatusEngine

/// Computed budget status for one budget in one period.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct BudgetStatus {
    /// The budget this status is for.
    pub budget: bc_models::Budget,
    /// Period start date (inclusive).
    pub period_start: jiff::civil::Date,
    /// Period end date (exclusive).
    pub period_end: jiff::civil::Date,
    /// The viewing window for which this status was computed.
    pub window: bc_models::BudgetWindow,
    /// Allocated amount, pro-rated to the window duration.
    pub allocated: bc_models::Decimal,
    /// Commodity of all monetary values in this status (from the budget's target commodity,
    /// or `None` for tracking-only multi-commodity budgets).
    pub commodity: Option<bc_models::CommodityCode>,
    /// Sum of postings matched to this budget in the period.
    pub actuals: bc_models::Decimal,
    /// Balance rolled over from the previous period (zero for `ResetToZero` policy).
    pub rollover: bc_models::Decimal,
    /// Funds available: `allocated + rollover - actuals`.
    pub available: bc_models::Decimal,
}

/// Computes budget actuals, rollover, and status for budgets.
#[derive(Debug, Clone)]
pub struct BudgetStatusEngine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl BudgetStatusEngine {
    /// Creates a new [`BudgetStatusEngine`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Computes the budget status for `budget` over an explicit [`bc_models::BudgetWindow`].
    ///
    /// The allocation is pro-rated: `prorated = allocation × (window_days / natural_period_days)`.
    /// Actuals are summed only within `[window.start, window.end)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for_window(
        &self,
        budget: &bc_models::Budget,
        window: bc_models::BudgetWindow,
    ) -> crate::BcResult<BudgetStatus> {
        let (period_start, period_end) = budget.period().range_containing(window.start);
        let commodity: Option<bc_models::CommodityCode> =
            budget.target().map(|t| t.commodity().clone());

        let svc = BudgetService::new(self.pool.clone());
        let allocation = svc.get_allocation(budget.id(), period_start).await?;
        let full_allocated = allocation
            .as_ref()
            .map_or(bc_models::Decimal::ZERO, |a| a.amount().value());

        let window_days = window.days();
        debug_assert!(window_days >= 0, "BudgetWindow has end before start");
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date returns a Span; get_days() is safe for any realistic period"
        )]
        let period_days = i64::from((period_end - period_start).get_days());

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal / and * panic on overflow/divide-by-zero; guarded by period_days != 0 check"
        )]
        let allocated = if period_days == 0 {
            bc_models::Decimal::ZERO
        } else {
            let ratio =
                bc_models::Decimal::from(window_days) / bc_models::Decimal::from(period_days);
            (full_allocated * ratio).round_dp(2)
        };

        let actuals = self
            .sum_actuals(budget, window.start, window.end, commodity.as_ref())
            .await?;
        let rollover = self
            .rollover_for(budget, period_start, commodity.as_ref())
            .await?;

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "budget arithmetic on Decimal values"
        )]
        let available = allocated + rollover - actuals;

        Ok(BudgetStatus {
            budget: budget.clone(),
            period_start,
            period_end,
            window,
            allocated,
            commodity,
            actuals,
            rollover,
            available,
        })
    }

    /// Computes the budget status for `budget` as of `as_of`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for(
        &self,
        budget: &bc_models::Budget,
        as_of: jiff::civil::Date,
    ) -> crate::BcResult<BudgetStatus> {
        let (start, end) = budget.period().range_containing(as_of);
        let label = format!("{start} \u{2013} {end}");
        self.status_for_window(budget, bc_models::BudgetWindow::custom(start, end, label))
            .await
    }

    /// Computes budget status for multiple budgets as of `as_of`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_all(
        &self,
        budgets: &[bc_models::Budget],
        as_of: jiff::civil::Date,
    ) -> crate::BcResult<Vec<BudgetStatus>> {
        let mut out = Vec::with_capacity(budgets.len());
        for b in budgets {
            out.push(self.status_for(b, as_of).await?);
        }
        Ok(out)
    }

    /// Fetches posting amount strings for `account_id` in `[period_start, period_end)`.
    ///
    /// When `tag_filter` is `Some`, only postings carrying that tag or a descendant are
    /// returned.  When `commodity` is `Some`, the query is further restricted to that
    /// commodity.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure.
    #[inline]
    async fn fetch_posting_amounts(
        &self,
        account_id: &bc_models::AccountId,
        period_start: jiff::civil::Date,
        period_end: jiff::civil::Date,
        commodity: Option<&bc_models::CommodityCode>,
        tag_filter: Option<&bc_models::TagId>,
        voided_str: &str,
    ) -> crate::BcResult<Vec<(String,)>> {
        // The four variants differ only by the presence of commodity / tag filters.
        // We branch here rather than building a dynamic query string.
        match (tag_filter, commodity) {
            (Some(tag), Some(comm)) => sqlx::query_as(
                "SELECT p.amount FROM postings p
                 JOIN transactions t ON t.id = p.transaction_id
                 WHERE p.account_id = ? AND p.commodity = ?
                   AND t.date >= ? AND t.date < ? AND t.status != ?
                   AND EXISTS (
                     SELECT 1 FROM posting_tags pt WHERE pt.posting_id = p.id
                     AND pt.tag_id IN (
                       WITH RECURSIVE subtree(id) AS (
                         SELECT ? UNION ALL
                         SELECT tg.id FROM tags tg
                         INNER JOIN subtree s ON tg.parent_id = s.id
                       ) SELECT id FROM subtree))",
            )
            .bind(account_id.to_string())
            .bind(comm.as_str())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(voided_str)
            .bind(tag.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into),
            (Some(tag), None) => sqlx::query_as(
                "SELECT p.amount FROM postings p
                 JOIN transactions t ON t.id = p.transaction_id
                 WHERE p.account_id = ?
                   AND t.date >= ? AND t.date < ? AND t.status != ?
                   AND EXISTS (
                     SELECT 1 FROM posting_tags pt WHERE pt.posting_id = p.id
                     AND pt.tag_id IN (
                       WITH RECURSIVE subtree(id) AS (
                         SELECT ? UNION ALL
                         SELECT tg.id FROM tags tg
                         INNER JOIN subtree s ON tg.parent_id = s.id
                       ) SELECT id FROM subtree))",
            )
            .bind(account_id.to_string())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(voided_str)
            .bind(tag.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into),
            (None, Some(comm)) => sqlx::query_as(
                "SELECT p.amount FROM postings p
                 JOIN transactions t ON t.id = p.transaction_id
                 WHERE p.account_id = ? AND p.commodity = ?
                   AND t.date >= ? AND t.date < ? AND t.status != ?",
            )
            .bind(account_id.to_string())
            .bind(comm.as_str())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(voided_str)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into),
            (None, None) => sqlx::query_as(
                "SELECT p.amount FROM postings p
                 JOIN transactions t ON t.id = p.transaction_id
                 WHERE p.account_id = ?
                   AND t.date >= ? AND t.date < ? AND t.status != ?",
            )
            .bind(account_id.to_string())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(voided_str)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into),
        }
    }

    /// Sums actuals for `budget` in `[period_start, period_end)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    async fn sum_actuals(
        &self,
        budget: &bc_models::Budget,
        period_start: jiff::civil::Date,
        period_end: jiff::civil::Date,
        commodity: Option<&bc_models::CommodityCode>,
    ) -> crate::BcResult<bc_models::Decimal> {
        let voided_str = crate::db::to_db_str(bc_models::TransactionStatus::Voided)?;
        let rows = self
            .fetch_posting_amounts(
                budget.account_id(),
                period_start,
                period_end,
                commodity,
                budget.tag_filter(),
                &voided_str,
            )
            .await?;

        rows.into_iter()
            .try_fold(bc_models::Decimal::ZERO, |acc, (amt_str,)| {
                let d = amt_str.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
                })?;
                acc.checked_add(d)
                    .ok_or_else(|| crate::BcError::BadData("actuals sum overflow".into()))
            })
    }

    /// Computes rollover from the period immediately before `period_start`.
    ///
    /// For `ResetToZero`: always returns `Decimal::ZERO`.
    /// For `CarryForward`: returns `prev_allocated + prev_rollover - prev_actuals`.
    /// For `CapAtTarget`: clamps to `[0, target]`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    fn rollover_for<'a>(
        &'a self,
        budget: &'a bc_models::Budget,
        period_start: jiff::civil::Date,
        commodity: Option<&'a bc_models::CommodityCode>,
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = crate::BcResult<bc_models::Decimal>> + Send + 'a>,
    > {
        Box::pin(async move {
            if matches!(budget.rollover(), bc_models::RolloverPolicy::ResetToZero) {
                return Ok(bc_models::Decimal::ZERO);
            }

            let prev_period_date = period_start
                .checked_sub(jiff::Span::new().days(1_i32))
                .map_err(|e| crate::BcError::BadData(format!("period underflow: {e}")))?;
            let (prev_start, prev_end) = budget.period().range_containing(prev_period_date);

            let svc = BudgetService::new(self.pool.clone());
            let prev_alloc = svc.get_allocation(budget.id(), prev_start).await?;
            let prev_allocated = prev_alloc
                .as_ref()
                .map_or(bc_models::Decimal::ZERO, |a| a.amount().value());

            let prev_actuals = self
                .sum_actuals(budget, prev_start, prev_end, commodity)
                .await?;

            if prev_alloc.is_none() && prev_actuals == bc_models::Decimal::ZERO {
                return Ok(bc_models::Decimal::ZERO);
            }

            let prev_rollover = self.rollover_for(budget, prev_start, commodity).await?;

            #[expect(
                clippy::arithmetic_side_effects,
                reason = "budget arithmetic on Decimal values bounded by allocation amounts"
            )]
            let surplus = prev_allocated + prev_rollover - prev_actuals;

            Ok(match budget.rollover() {
                bc_models::RolloverPolicy::CarryForward => surplus,
                bc_models::RolloverPolicy::CapAtTarget => {
                    #[expect(
                        clippy::expect_used,
                        reason = "CapAtTarget budgets validated to have target at creation"
                    )]
                    let cap = budget
                        .target()
                        .expect("CapAtTarget budget must have target; BudgetService::create validates this")
                        .value();
                    surplus.max(bc_models::Decimal::ZERO).min(cap)
                }
                bc_models::RolloverPolicy::ResetToZero => bc_models::Decimal::ZERO,
                _ => {
                    tracing::warn!(
                        policy = ?budget.rollover(),
                        "unrecognised rollover policy variant — defaulting to zero"
                    );
                    bc_models::Decimal::ZERO
                }
            })
        })
    }
}

#[cfg(test)]
mod budget_service_tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::BudgetService;
    use crate::account::Service as AccountService;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_budget_returns_budget_with_id(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let groceries = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(groceries.clone())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create budget");

        assert!(budget.id().to_string().starts_with("budget_"));
        assert_eq!(budget.account_id(), &groceries);
        assert!(budget.is_tracking_only());
        assert!(budget.tag_filter().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_budget_with_target(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let groceries = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(groceries.clone())
            .target(Amount::new(
                Decimal::from(600_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create budget");

        assert!(!budget.is_tracking_only());
        assert_eq!(
            budget.target(),
            Some(&Amount::new(
                Decimal::from(600_i32),
                CommodityCode::new("AUD")
            ))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_returns_only_active_budgets(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Dining")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let b = svc
            .create()
            .account_id(acc.clone())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");

        svc.archive(b.id()).await.expect("archive");

        let list = svc.list().await.expect("list");
        assert!(list.is_empty(), "archived budget should not appear");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_and_get_allocation(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(acc)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create budget");

        let alloc = svc
            .allocate(
                budget.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(600_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate");

        assert_eq!(alloc.period_start(), Date::constant(2026, 3, 1));
        assert_eq!(alloc.budget_id(), budget.id());

        let fetched = svc
            .get_allocation(budget.id(), Date::constant(2026, 3, 1))
            .await
            .expect("get_allocation");
        assert!(fetched.is_some());
        assert_eq!(
            fetched.expect("allocation should exist").amount(),
            &Amount::new(Decimal::from(600_i32), CommodityCode::new("AUD"))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn duplicate_untagged_budget_on_same_account_fails(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        svc.create()
            .account_id(acc.clone())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("first create should succeed");

        let result = svc
            .create()
            .account_id(acc)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await;

        assert!(
            result.is_err(),
            "second untagged budget should fail uniqueness constraint"
        );
    }
}
