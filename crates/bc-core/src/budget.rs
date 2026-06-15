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
            created_at: now,
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
        // Verify budget exists before opening the transaction.
        let _budget = self.get(id).await?;

        let now = jiff::Timestamp::now();
        let event = crate::events::Event::BudgetArchived {
            budget_id: id.clone(),
            archived_at: now,
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

    /// Updates the mutable properties of an active budget.
    ///
    /// Only `name`, `target`, and `rollover` may be changed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no active budget exists with `id`.
    /// Returns [`crate::BcError::InvalidInput`] if `rollover` is `CapAtTarget` but
    /// no target is set after applying the update.
    /// Returns [`crate::BcError`] on event or database failure.
    #[inline]
    pub async fn update(
        &self,
        id: &bc_models::BudgetId,
        name: Option<Option<String>>,
        target: Option<Option<bc_models::Amount>>,
        rollover: Option<bc_models::RolloverPolicy>,
    ) -> crate::BcResult<bc_models::Budget> {
        let budget = self.get(id).await?;

        let new_name = name
            .clone()
            .unwrap_or_else(|| budget.name().map(str::to_owned));
        let new_target = target.clone().unwrap_or_else(|| budget.target().cloned());
        let new_rollover = rollover.unwrap_or_else(|| budget.rollover());

        if matches!(new_rollover, bc_models::RolloverPolicy::CapAtTarget) && new_target.is_none() {
            return Err(crate::BcError::InvalidInput(
                "rollover policy CapAtTarget requires a non-None target".into(),
            ));
        }

        let event = crate::events::Event::BudgetUpdated {
            budget_id: id.clone(),
            name: name.clone(),
            target: target.clone(),
            rollover,
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "UPDATE budgets \
             SET name = ?, \
                 target_amount = ?, \
                 target_currency = ?, \
                 rollover = ? \
             WHERE id = ? AND archived_at IS NULL",
        )
        .bind(new_name.as_deref())
        .bind(new_target.as_ref().map(|a| a.value().to_string()))
        .bind(new_target.as_ref().map(|a| a.commodity().as_str()))
        .bind(crate::db::to_db_str(new_rollover)?)
        .bind(id.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(budget_id = %id, "budget updated");

        self.get(id).await
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
        let budget = self.get(budget_id).await?;

        // Validate allocation commodity matches budget target (when target is present).
        if let Some(target) = budget.target()
            && target.commodity() != amount.commodity()
        {
            return Err(crate::BcError::InvalidInput(format!(
                "allocation commodity '{}' does not match budget target commodity '{}'",
                amount.commodity(),
                target.commodity(),
            )));
        }

        // Validate that period_start is the canonical start of a period.
        let canonical = budget.period().range_containing(period_start).0;
        if canonical != period_start {
            return Err(crate::BcError::InvalidInput(format!(
                "'{period_start}' is not a canonical period start for {:?} period; \
                 did you mean '{canonical}'?",
                budget.period(),
            )));
        }

        let id = bc_models::BudgetAllocationId::new();
        let candidate_created_at = jiff::Timestamp::now();

        let event = crate::events::Event::BudgetAllocated {
            budget_id: budget_id.clone(),
            period_start,
            amount: amount.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        let row = sqlx::query_as::<_, BudgetAllocationRow>(
            "INSERT INTO budget_allocations \
             (id, budget_id, period_start, amount, commodity, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (budget_id, period_start) \
             DO UPDATE SET amount = excluded.amount, commodity = excluded.commodity \
             RETURNING *",
        )
        .bind(id.to_string())
        .bind(budget_id.to_string())
        .bind(period_start.to_string())
        .bind(amount.value().to_string())
        .bind(amount.commodity().as_str())
        .bind(candidate_created_at.to_string())
        .fetch_one(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(budget_id = %budget_id, %period_start, "budget allocated");

        bc_models::BudgetAllocation::try_from(row)
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
#[derive(Clone)]
pub struct BudgetStatusEngine {
    /// The SQLite connection pool.
    pool: SqlitePool,
    /// Foreign exchange rate service for cross-commodity conversion.
    fx: std::sync::Arc<dyn crate::fx::FxRateService>,
}

impl core::fmt::Debug for BudgetStatusEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BudgetStatusEngine")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

impl BudgetStatusEngine {
    /// Creates a new [`BudgetStatusEngine`] with the given connection pool and FX service.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool, fx: std::sync::Arc<dyn crate::fx::FxRateService>) -> Self {
        Self { pool, fx }
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

        let svc = BudgetService::new(self.pool.clone());
        let allocation = svc.get_allocation(budget.id(), period_start).await?;
        let full_allocated = allocation
            .as_ref()
            .map_or(bc_models::Decimal::ZERO, |a| a.amount().value());

        let window_days = window.days();
        if window_days < 0 {
            return Err(crate::BcError::InvalidInput(format!(
                "BudgetWindow has end before start: {} to {}",
                window.start, window.end,
            )));
        }
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

        let (actuals, commodity) = self.sum_actuals(budget, window.start, window.end).await?;
        let rollover = self.rollover_for(budget, period_start).await?;

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

    /// Fetches posting amount and commodity strings for `account_id` in `[period_start, period_end)`.
    ///
    /// When `tag_filter` is `Some`, only postings carrying that tag or a descendant are returned.
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
        tag_filter: Option<&bc_models::TagId>,
        voided_str: &str,
    ) -> crate::BcResult<Vec<(String, String)>> {
        match tag_filter {
            Some(tag) => sqlx::query_as(
                "SELECT p.amount, p.commodity FROM postings p
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
            None => sqlx::query_as(
                "SELECT p.amount, p.commodity FROM postings p
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
    /// Returns the total and the commodity it is denominated in.  For budgets with a target
    /// commodity, foreign postings are converted via the FX service (and skipped with a warning
    /// if conversion is unavailable).  For tracking-only budgets, postings are grouped by
    /// commodity and the dominant group (by absolute value) is returned.
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
    ) -> crate::BcResult<(bc_models::Decimal, Option<bc_models::CommodityCode>)> {
        let voided_str = crate::db::to_db_str(bc_models::TransactionStatus::Voided)?;
        let rows = self
            .fetch_posting_amounts(
                budget.account_id(),
                period_start,
                period_end,
                budget.tag_filter(),
                &voided_str,
            )
            .await?;

        let target_commodity: Option<bc_models::CommodityCode> =
            budget.target().map(|t| t.commodity().clone());

        if let Some(ref target) = target_commodity {
            // Budget has a target commodity: sum native, convert foreign via FX.
            let mut total = bc_models::Decimal::ZERO;
            for (amt_str, comm_str) in rows {
                let value = amt_str.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
                })?;
                let posting_commodity = bc_models::CommodityCode::new(&comm_str);
                let posting_amount = bc_models::Amount::new(value, posting_commodity);
                match self.fx.convert(&posting_amount, target) {
                    Ok(a) => {
                        total = total.checked_add(a.value()).ok_or_else(|| {
                            crate::BcError::BadData("actuals sum overflow".into())
                        })?;
                    }
                    Err(e) => {
                        tracing::warn!(
                            budget_id = %budget.id(),
                            %e,
                            "skipping posting: FX conversion unavailable"
                        );
                    }
                }
            }
            Ok((total, Some(target.clone())))
        } else {
            // Tracking-only: group by commodity, return dominant group.
            let mut groups: std::collections::HashMap<String, bc_models::Decimal> =
                std::collections::HashMap::new();
            for (amt_str, comm_str) in rows {
                let value = amt_str.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
                })?;
                let entry = groups.entry(comm_str).or_insert(bc_models::Decimal::ZERO);
                *entry = entry
                    .checked_add(value)
                    .ok_or_else(|| crate::BcError::BadData("actuals sum overflow".into()))?;
            }
            if groups.is_empty() {
                return Ok((bc_models::Decimal::ZERO, None));
            }
            let group_count = groups.len();
            #[expect(
                clippy::expect_used,
                reason = "groups is non-empty; checked immediately above"
            )]
            let (dominant_comm, dominant_total) = groups
                .into_iter()
                .max_by(|(_, a), (_, b)| {
                    a.abs()
                        .partial_cmp(&b.abs())
                        .unwrap_or(core::cmp::Ordering::Equal)
                })
                .expect("groups is non-empty");
            if group_count > 1 {
                tracing::warn!(
                    budget_id = %budget.id(),
                    commodity = %dominant_comm,
                    "tracking-only budget has multi-commodity postings; \
                     reporting dominant commodity only"
                );
            }
            Ok((
                dominant_total,
                Some(bc_models::CommodityCode::new(dominant_comm)),
            ))
        }
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
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = crate::BcResult<bc_models::Decimal>> + Send + 'a>,
    > {
        Box::pin(async move {
            if matches!(budget.rollover(), bc_models::RolloverPolicy::ResetToZero) {
                return Ok(bc_models::Decimal::ZERO);
            }

            // Custom periods have no natural boundary alignment; "previous period"
            // is undefined so there is nothing to roll over.
            if matches!(budget.period(), bc_models::Period::Custom { .. }) {
                return Ok(bc_models::Decimal::ZERO);
            }

            let prev_period_date = period_start
                .checked_sub(jiff::Span::new().days(1_i32))
                .map_err(|e| crate::BcError::BadData(format!("period underflow: {e}")))?;
            let (prev_start, prev_end) = budget.period().range_containing(prev_period_date);

            // Don't recurse before the budget itself existed.
            let budget_epoch = budget
                .period()
                .range_containing(budget.created_at().to_zoned(jiff::tz::TimeZone::UTC).date())
                .0;
            if prev_start < budget_epoch {
                return Ok(bc_models::Decimal::ZERO);
            }

            let svc = BudgetService::new(self.pool.clone());
            let prev_alloc = svc.get_allocation(budget.id(), prev_start).await?;
            let prev_allocated = prev_alloc
                .as_ref()
                .map_or(bc_models::Decimal::ZERO, |a| a.amount().value());

            let (prev_actuals, _) = self.sum_actuals(budget, prev_start, prev_end).await?;

            let prev_rollover = self.rollover_for(budget, prev_start).await?;

            // Only short-circuit when there is genuinely nothing to carry forward.
            if prev_alloc.is_none()
                && prev_actuals == bc_models::Decimal::ZERO
                && prev_rollover == bc_models::Decimal::ZERO
            {
                return Ok(bc_models::Decimal::ZERO);
            }

            #[expect(
                clippy::arithmetic_side_effects,
                reason = "budget arithmetic on Decimal values bounded by allocation amounts"
            )]
            let surplus = prev_allocated + prev_rollover - prev_actuals;

            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "ResetToZero is handled by early return above"
            )]
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
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::RolloverPolicy;
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionStatus;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::BudgetService;
    use super::BudgetStatusEngine;
    use crate::account::Service as AccountService;
    use crate::fx::noop_fx;
    use crate::transaction::Service as TransactionService;

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
            .account_id(acc)
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

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_returns_not_found_on_double_archive(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Food")
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

        svc.archive(budget.id())
            .await
            .expect("first archive should succeed");

        let result = svc.archive(budget.id()).await;
        assert!(
            matches!(result, Err(crate::BcError::NotFound(_))),
            "second archive should return NotFound, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn carry_forward_rollover_adds_surplus_to_next_period(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let budget_account = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create budget account");
        let offset_account = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create offset account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(budget_account.clone())
            .target(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create budget");

        svc.allocate(
            budget.id(),
            Date::constant(2030, 7, 1),
            Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("allocate July");

        let txns = TransactionService::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(Date::constant(2030, 7, 15))
            .description("Weekly shop")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(budget_account)
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(offset_account)
                    .amount(Amount::new(dec!(-60), CommodityCode::new("AUD")))
                    .build(),
            ])
            .status(TransactionStatus::Cleared)
            .created_at(Timestamp::now())
            .build();
        txns.create(tx).await.expect("create transaction");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let statuses = engine
            .status_all(&[budget], Date::constant(2030, 8, 15))
            .await
            .expect("status_all August");

        assert_eq!(statuses.len(), 1, "expected exactly one status");
        let aug = statuses
            .first()
            .expect("statuses is non-empty; checked above");
        assert_eq!(
            aug.rollover,
            dec!(40),
            "rollover should be 40 AUD (100 allocated - 60 spent in July)"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn carry_forward_rollover_includes_first_period_when_created_on_period_start(
        pool: sqlx::SqlitePool,
    ) {
        let accounts = AccountService::new(pool.clone());
        let budget_account = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create budget account");
        let offset_account = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create offset account");

        let svc = BudgetService::new(pool.clone());

        let created_budget = svc
            .create()
            .account_id(budget_account.clone())
            .target(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create budget");

        // Backdate created_at to 2030-07-01 so budget_epoch = 2030-07-01.
        sqlx::query("UPDATE budgets SET created_at = '2030-07-01T00:00:00Z' WHERE id = ?")
            .bind(created_budget.id().to_string())
            .execute(&pool)
            .await
            .expect("backdate created_at");

        let budget = svc.get(created_budget.id()).await.expect("re-fetch budget");

        svc.allocate(
            budget.id(),
            Date::constant(2030, 7, 1),
            Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("allocate July");

        let txns = TransactionService::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(Date::constant(2030, 7, 15))
            .description("Weekly shop")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(budget_account)
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(offset_account)
                    .amount(Amount::new(dec!(-60), CommodityCode::new("AUD")))
                    .build(),
            ])
            .status(bc_models::TransactionStatus::Cleared)
            .created_at(Timestamp::now())
            .build();
        txns.create(tx).await.expect("create transaction");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let statuses = engine
            .status_all(&[budget], Date::constant(2030, 8, 15))
            .await
            .expect("status_all August");

        let aug = statuses.first().expect("one status");
        assert_eq!(
            aug.rollover,
            dec!(40),
            "July surplus (100 - 60 = 40) must carry into August even when budget was \
             created on 2030-07-01 (the first day of the period)"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn carry_forward_rollover_survives_gap_period(pool: sqlx::SqlitePool) {
        // Period A: allocate 100, spend 60 → surplus 40.
        // Period B: no allocation, no activity (gap).
        // Period C: rollover should be 40, not 0.
        let accounts = AccountService::new(pool.clone());
        let budget_account = accounts
            .create()
            .name("Entertainment")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create budget account");
        let offset_account = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create offset account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(budget_account.clone())
            .target(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create budget");

        // Period A: July 2030 — allocate 100, spend 60.
        svc.allocate(
            budget.id(),
            Date::constant(2030, 7, 1),
            Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("allocate July");

        let txns = TransactionService::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(Date::constant(2030, 7, 15))
            .description("Concert")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(budget_account)
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(offset_account)
                    .amount(Amount::new(dec!(-60), CommodityCode::new("AUD")))
                    .build(),
            ])
            .status(bc_models::TransactionStatus::Cleared)
            .created_at(Timestamp::now())
            .build();
        txns.create(tx).await.expect("create transaction");

        // Period B (August 2030): no allocation, no activity — pure gap.
        // Period C: September 2030 — should see 40 rollover from July.
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let statuses = engine
            .status_all(&[budget], Date::constant(2030, 9, 15))
            .await
            .expect("status_all September");

        let sep = statuses.first().expect("one status");
        assert_eq!(
            sep.rollover,
            dec!(40),
            "40 AUD surplus from July must survive an empty August and appear in September"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cap_at_target_clamps_rollover_to_target(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let budget_account = accounts
            .create()
            .name("Entertainment")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create budget account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(budget_account)
            .target(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CapAtTarget)
            .call()
            .await
            .expect("create budget");

        svc.allocate(
            budget.id(),
            Date::constant(2030, 7, 1),
            Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("allocate July");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let statuses = engine
            .status_all(&[budget], Date::constant(2030, 8, 15))
            .await
            .expect("status_all August");

        assert_eq!(statuses.len(), 1, "expected exactly one status");
        let aug = statuses
            .first()
            .expect("statuses is non-empty; checked above");
        assert_eq!(
            aug.rollover,
            dec!(100),
            "rollover capped at target (100), not 200"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn duplicate_tagged_budget_on_same_account_fails(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Dining")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let tag_id = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, 'restaurant', ?)")
            .bind(tag_id.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert tag");

        let svc = BudgetService::new(pool.clone());
        svc.create()
            .account_id(acc.clone())
            .tag_filter(tag_id.clone())
            .target(Amount::new(
                Decimal::from(200_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("first tagged budget should succeed");

        let result = svc
            .create()
            .account_id(acc)
            .tag_filter(tag_id)
            .target(Amount::new(
                Decimal::from(200_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await;

        assert!(
            result.is_err(),
            "duplicate tagged budget on same account should fail"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn can_create_untagged_budget_after_archiving_previous(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Transport")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let first = svc
            .create()
            .account_id(acc.clone())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create first untagged budget");

        svc.archive(first.id()).await.expect("archive first budget");

        let second = svc
            .create()
            .account_id(acc)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await;

        assert!(
            second.is_ok(),
            "creating a new untagged budget after archiving the previous one should succeed"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_budget_name(pool: sqlx::SqlitePool) {
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
            .name("Old Name")
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create budget");

        let updated = svc
            .update(budget.id(), Some(Some("New Name".into())), None, None)
            .await
            .expect("update budget");

        assert_eq!(updated.name(), Some("New Name"), "name updated");
        assert_eq!(updated.rollover(), budget.rollover(), "rollover unchanged");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_budget_target_and_rollover(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Entertainment")
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

        let updated = svc
            .update(
                budget.id(),
                None,
                Some(Some(Amount::new(
                    Decimal::from(200_i32),
                    CommodityCode::new("AUD"),
                ))),
                Some(RolloverPolicy::CarryForward),
            )
            .await
            .expect("update budget");

        assert_eq!(
            updated.target().map(bc_models::Amount::value),
            Some(Decimal::from(200_i32)),
            "target set to 200 AUD"
        );
        assert_eq!(
            updated.rollover(),
            RolloverPolicy::CarryForward,
            "rollover updated"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_cap_at_target_without_target_fails(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Savings")
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
            .expect("create tracking-only budget");

        let result = svc
            .update(budget.id(), None, None, Some(RolloverPolicy::CapAtTarget))
            .await;

        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "CapAtTarget without target must fail with InvalidInput"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_rejects_non_canonical_period_start(pool: sqlx::SqlitePool) {
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

        // 2026-03-15 is mid-month; canonical start for Monthly is 2026-03-01.
        let result = svc
            .allocate(
                budget.id(),
                Date::constant(2026, 3, 15),
                Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
            )
            .await;

        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "mid-period date must be rejected with InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reallocate_returns_id_that_exists_in_db(pool: sqlx::SqlitePool) {
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

        let first = svc
            .allocate(
                budget.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("first allocation");

        let second = svc
            .allocate(
                budget.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(300_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("re-allocation (upsert)");

        // The first allocation's ID must survive the upsert.
        assert_eq!(
            first.id(),
            second.id(),
            "re-allocation must return the original row's ID, not a freshly-generated one"
        );

        // Verify the returned ID actually exists in the database.
        let fetched = svc
            .get_allocation(budget.id(), Date::constant(2026, 3, 1))
            .await
            .expect("get_allocation succeeds")
            .expect("allocation exists");

        assert_eq!(fetched.id(), second.id(), "fetched ID matches returned ID");
        assert_eq!(
            fetched.amount().value(),
            Decimal::from(300_i32),
            "amount updated to 300"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_for_window_rejects_inverted_window(pool: sqlx::SqlitePool) {
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

        // end (Jan 1) is before start (Feb 1) — inverted window.
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 2, 1),
            Date::constant(2026, 1, 1),
            "inverted",
        );

        let engine = BudgetStatusEngine::new(pool, noop_fx());
        let result = engine.status_for_window(&budget, window).await;

        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "inverted window must return InvalidInput, got: {result:?}"
        );
    }

    #[cfg(test)]
    impl BudgetStatusEngine {
        /// Test accessor for `rollover_for` private method.
        pub(crate) async fn rollover_for_test(
            &self,
            budget: &bc_models::Budget,
            period_start: jiff::civil::Date,
        ) -> crate::BcResult<bc_models::Decimal> {
            self.rollover_for(budget, period_start).await
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_for_custom_period_returns_zero(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let account = accounts
            .create()
            .name("Test Account")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let svc = BudgetService::new(pool.clone());
        let budget = svc
            .create()
            .account_id(account)
            .period(Period::custom(Some(30), None, None).expect("valid"))
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("budget created");

        let engine = BudgetStatusEngine::new(pool, noop_fx());
        let rollover = engine
            .rollover_for_test(&budget, Date::constant(2026, 2, 1))
            .await
            .expect("no error");
        assert_eq!(rollover, bc_models::Decimal::ZERO);
    }
}
