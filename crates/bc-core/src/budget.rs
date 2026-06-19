//! Budget calculation engine: actuals, rollover, and budget status.

use sqlx::SqlitePool;

// MARK: BudgetService

/// Internal row type returned from the `budgets` table (anchor columns only).
#[derive(sqlx::FromRow)]
struct BudgetRow {
    /// Raw budget ID string.
    id: String,
    /// Raw account ID string this budget is anchored to.
    account_id: String,
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
            .created_at(created_at)
            .maybe_archived_at(archived_at)
            .build())
    }
}

/// Internal row type for budget revision queries.
#[derive(sqlx::FromRow)]
struct BudgetRevisionRow {
    /// Raw revision ID string.
    id: String,
    /// Raw budget ID string this revision belongs to.
    budget_id: String,
    /// YYYY-MM-DD effective-from date.
    effective_from: String,
    /// Optional display name.
    name: Option<String>,
    /// Decimal string for the target amount; NULL = tracking-only.
    target_amount: Option<String>,
    /// Commodity code for the target; NULL when `target_amount` is NULL.
    target_currency: Option<String>,
    /// JSON-serialised [`bc_models::Period`].
    period: String,
    /// Snake-case rollover policy string.
    rollover: String,
    /// Optional raw tag ID string for sub-budget filtering.
    tag_filter: Option<String>,
    /// ISO 8601 creation timestamp.
    created_at: String,
}

impl TryFrom<BudgetRevisionRow> for bc_models::BudgetRevision {
    type Error = crate::BcError;

    /// Converts a raw database row into a domain [`bc_models::BudgetRevision`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: BudgetRevisionRow) -> crate::BcResult<Self> {
        let id = row
            .id
            .parse::<bc_models::BudgetRevisionId>()
            .map_err(|e| {
                crate::BcError::BadData(format!("invalid revision id '{}': {e}", row.id))
            })?;
        let budget_id = row.budget_id.parse::<bc_models::BudgetId>().map_err(|e| {
            crate::BcError::BadData(format!("invalid budget_id '{}': {e}", row.budget_id))
        })?;
        let effective_from =
            row.effective_from
                .parse::<jiff::civil::Date>()
                .map_err(|e| {
                    crate::BcError::BadData(format!(
                        "invalid effective_from '{}': {e}",
                        row.effective_from
                    ))
                })?;
        let target = match (row.target_amount, row.target_currency) {
            (Some(a), Some(c)) => {
                let qty = a.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid target_amount '{a}': {e}"))
                })?;
                Some(bc_models::Amount::new(
                    qty,
                    bc_models::CommodityCode::new(&c),
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
        let tag_filter = row
            .tag_filter
            .as_deref()
            .map(|s| {
                s.parse::<bc_models::TagId>()
                    .map_err(|e| crate::BcError::BadData(format!("invalid tag_filter '{s}': {e}")))
            })
            .transpose()?;
        let created_at = row.created_at.parse::<jiff::Timestamp>().map_err(|e| {
            crate::BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;
        Ok(bc_models::BudgetRevision::builder()
            .id(id)
            .budget_id(budget_id)
            .effective_from(effective_from)
            .maybe_name(row.name)
            .maybe_target(target)
            .period(period)
            .rollover(rollover)
            .maybe_tag_filter(tag_filter)
            .created_at(created_at)
            .build())
    }
}

/// Budget CRUD service (anchor + revision management).
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

    /// Creates a new budget anchor and its initial revision.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `rollover` is `CapAtTarget` and
    /// `target` is `None`.
    /// Returns [`crate::BcError`] on event append or database insert failure.
    #[builder]
    #[inline]
    pub async fn create(
        &self,
        account_id: bc_models::AccountId,
        effective_from: jiff::civil::Date,
        tag_filter: Option<bc_models::TagId>,
        #[builder(into)] name: Option<String>,
        target: Option<bc_models::Amount>,
        period: bc_models::Period,
        rollover: bc_models::RolloverPolicy,
    ) -> crate::BcResult<(bc_models::Budget, bc_models::BudgetRevision)> {
        if rollover == bc_models::RolloverPolicy::CapAtTarget && target.is_none() {
            return Err(crate::BcError::InvalidInput(
                "CapAtTarget rollover policy requires a target amount".to_owned(),
            ));
        }

        let budget_id = bc_models::BudgetId::new();
        let revision_id = bc_models::BudgetRevisionId::new();
        let now = jiff::Timestamp::now();

        let event = crate::events::Event::BudgetCreated {
            budget_id: budget_id.clone(),
            account_id: account_id.clone(),
            created_at: now,
            revision_id: revision_id.clone(),
            effective_from,
            name: name.clone(),
            target: target.clone(),
            period: period.clone(),
            rollover,
            tag_filter: tag_filter.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;

        sqlx::query("INSERT INTO budgets (id, account_id, created_at) VALUES (?, ?, ?)")
            .bind(budget_id.to_string())
            .bind(account_id.to_string())
            .bind(now.to_string())
            .execute(&mut *db_tx)
            .await?;

        let period_json = serde_json::to_string(&period)?;
        let rollover_db = crate::db::to_db_str(rollover)?;
        let (t_amt, t_cur) = target.as_ref().map_or((None, None), |a| {
            (Some(a.value().to_string()), Some(a.commodity().to_string()))
        });
        sqlx::query(
            "INSERT INTO budget_revisions \
             (id, budget_id, effective_from, name, target_amount, target_currency, \
              period, rollover, tag_filter, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(revision_id.to_string())
        .bind(budget_id.to_string())
        .bind(effective_from.to_string())
        .bind(&name)
        .bind(&t_amt)
        .bind(&t_cur)
        .bind(&period_json)
        .bind(&rollover_db)
        .bind(tag_filter.as_ref().map(ToString::to_string))
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(%budget_id, %account_id, "budget created");

        let budget = bc_models::Budget::builder()
            .id(budget_id.clone())
            .account_id(account_id)
            .created_at(now)
            .build();
        let revision = bc_models::BudgetRevision::builder()
            .id(revision_id)
            .budget_id(budget_id)
            .effective_from(effective_from)
            .maybe_name(name)
            .maybe_target(target)
            .period(period)
            .rollover(rollover)
            .maybe_tag_filter(tag_filter)
            .created_at(now)
            .build();
        Ok((budget, revision))
    }

    /// Lists all active (non-archived) budget anchors, ordered by `created_at`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn list(&self) -> crate::BcResult<Vec<bc_models::Budget>> {
        let rows = sqlx::query_as::<_, BudgetRow>(
            "SELECT id, account_id, created_at, archived_at FROM budgets \
             WHERE archived_at IS NULL ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(bc_models::Budget::try_from).collect()
    }

    /// Lists all active budget anchors for a specific account.
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
            "SELECT id, account_id, created_at, archived_at FROM budgets \
             WHERE account_id = ? AND archived_at IS NULL ORDER BY created_at ASC",
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(bc_models::Budget::try_from).collect()
    }

    /// Fetches an active budget anchor by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no active budget with that ID exists.
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn get(&self, id: &bc_models::BudgetId) -> crate::BcResult<bc_models::Budget> {
        let row = sqlx::query_as::<_, BudgetRow>(
            "SELECT id, account_id, created_at, archived_at FROM budgets \
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

    // MARK: Revision management

    /// Lists all revisions for a budget, ordered ascending by `effective_from`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn revisions(
        &self,
        budget_id: &bc_models::BudgetId,
    ) -> crate::BcResult<Vec<bc_models::BudgetRevision>> {
        let rows = sqlx::query_as::<_, BudgetRevisionRow>(
            "SELECT id, budget_id, effective_from, name, target_amount, target_currency, \
              period, rollover, tag_filter, created_at FROM budget_revisions \
             WHERE budget_id = ? ORDER BY effective_from ASC",
        )
        .bind(budget_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(bc_models::BudgetRevision::try_from)
            .collect()
    }

    /// Upserts a revision for an active budget (add new effective date, or amend existing).
    ///
    /// Conflict resolution is by `revision_id` (ON CONFLICT(id) DO UPDATE).
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `CapAtTarget` rollover has no target.
    /// Returns [`crate::BcError::NotFound`] if the budget is missing or archived.
    /// Returns [`crate::BcError`] on event or database failure.
    #[inline]
    pub async fn revise(
        &self,
        budget_id: &bc_models::BudgetId,
        revision: bc_models::BudgetRevision,
    ) -> crate::BcResult<bc_models::BudgetRevision> {
        let _ = self.get(budget_id).await?;
        if revision.budget_id() != budget_id {
            return Err(crate::BcError::InvalidInput(format!(
                "revision belongs to budget {}, not {budget_id}",
                revision.budget_id()
            )));
        }
        if revision.rollover() == bc_models::RolloverPolicy::CapAtTarget
            && revision.target().is_none()
        {
            return Err(crate::BcError::InvalidInput(
                "CapAtTarget rollover policy requires a target amount".to_owned(),
            ));
        }
        let event = crate::events::Event::BudgetRevisionSet {
            budget_id: budget_id.clone(),
            revision_id: revision.id().clone(),
            effective_from: revision.effective_from(),
            name: revision.name().map(str::to_owned),
            target: revision.target().cloned(),
            period: revision.period().clone(),
            rollover: revision.rollover(),
            tag_filter: revision.tag_filter().cloned(),
        };
        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;
        let period_json = serde_json::to_string(revision.period())?;
        let (t_amt, t_cur) = revision.target().map_or((None, None), |a| {
            (Some(a.value().to_string()), Some(a.commodity().to_string()))
        });
        sqlx::query(
            "INSERT INTO budget_revisions \
             (id, budget_id, effective_from, name, target_amount, target_currency, \
              period, rollover, tag_filter, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               effective_from = excluded.effective_from, name = excluded.name, \
               target_amount = excluded.target_amount, target_currency = excluded.target_currency, \
               period = excluded.period, rollover = excluded.rollover, \
               tag_filter = excluded.tag_filter",
        )
        .bind(revision.id().to_string())
        .bind(budget_id.to_string())
        .bind(revision.effective_from().to_string())
        .bind(revision.name())
        .bind(&t_amt)
        .bind(&t_cur)
        .bind(&period_json)
        .bind(crate::db::to_db_str(revision.rollover())?)
        .bind(revision.tag_filter().map(ToString::to_string))
        .bind(revision.created_at().to_string())
        .execute(&mut *db_tx)
        .await?;
        db_tx.commit().await?;
        tracing::info!(%budget_id, revision_id = %revision.id(), "budget revised");
        Ok(revision)
    }

    /// Removes a revision from a budget; rejects removing the last remaining revision.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if it is the only revision (archive instead).
    /// Returns [`crate::BcError::NotFound`] if the revision does not exist on this budget.
    /// Returns [`crate::BcError`] on event or database failure.
    #[inline]
    pub async fn remove_revision(
        &self,
        budget_id: &bc_models::BudgetId,
        revision_id: &bc_models::BudgetRevisionId,
    ) -> crate::BcResult<()> {
        let existing = self.revisions(budget_id).await?;
        if !existing.iter().any(|r| r.id() == revision_id) {
            return Err(crate::BcError::NotFound(revision_id.to_string()));
        }
        if existing.len() <= 1 {
            return Err(crate::BcError::InvalidInput(
                "cannot remove the last revision; archive the budget instead".to_owned(),
            ));
        }
        let event = crate::events::Event::BudgetRevisionRemoved {
            budget_id: budget_id.clone(),
            revision_id: revision_id.clone(),
        };
        let mut db_tx = self.pool.begin().await?;
        crate::events::insert_event(&event, &mut db_tx).await?;
        sqlx::query("DELETE FROM budget_revisions WHERE id = ? AND budget_id = ?")
            .bind(revision_id.to_string())
            .bind(budget_id.to_string())
            .execute(&mut *db_tx)
            .await?;
        db_tx.commit().await?;
        Ok(())
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
        if window_days == 0 {
            tracing::debug!(
                budget_id = %budget.id(),
                window_start = %window.start,
                "zero-day window: allocated will be 0"
            );
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

    // TODO: apply spread fields to period attribution (planned follow-on)
    /// Fetches raw `(amount, commodity)` pairs for postings to `account_id` or any
    /// descendant account in `[period_start, period_end)`, optionally filtered by tag.
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
                "WITH RECURSIVE \
                   acct_tree(id) AS ( \
                     SELECT ? UNION ALL \
                     SELECT a.id FROM accounts a \
                     INNER JOIN acct_tree ON a.parent_id = acct_tree.id \
                   ), \
                   tag_subtree(id) AS ( \
                     SELECT ? UNION ALL \
                     SELECT tg.id FROM tags tg \
                     INNER JOIN tag_subtree ON tg.parent_id = tag_subtree.id \
                   ) \
                 SELECT p.amount, p.commodity FROM postings p \
                 JOIN transactions t ON t.id = p.transaction_id \
                 WHERE p.account_id IN (SELECT id FROM acct_tree) \
                   AND t.date >= ? AND t.date < ? AND t.status != ? \
                   AND EXISTS ( \
                     SELECT 1 FROM posting_tags pt WHERE pt.posting_id = p.id \
                     AND pt.tag_id IN (SELECT id FROM tag_subtree))",
            )
            .bind(account_id.to_string())
            .bind(tag.to_string())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(voided_str)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as(
                "WITH RECURSIVE acct_tree(id) AS ( \
                   SELECT ? UNION ALL \
                   SELECT a.id FROM accounts a \
                   INNER JOIN acct_tree ON a.parent_id = acct_tree.id \
                 ) \
                 SELECT p.amount, p.commodity FROM postings p \
                 JOIN transactions t ON t.id = p.transaction_id \
                 WHERE p.account_id IN (SELECT id FROM acct_tree) \
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
    use bc_models::RolloverPolicy;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::BudgetService;
    use crate::account::Service as AccountService;

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
        let (b, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
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
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
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
    async fn create_makes_anchor_and_initial_revision(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Groceries")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, rev) = svc.create()
            .account_id(acc.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        assert_eq!(budget.account_id(), &acc);
        assert!(!budget.is_archived());
        assert_eq!(rev.budget_id(), budget.id());
        assert_eq!(rev.effective_from(), Date::constant(2026, 1, 1));
        let all = svc.revisions(budget.id()).await.expect("revisions");
        assert_eq!(all.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revise_adds_second_revision_ordered(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Salary")
            .account_type(AccountType::Income).kind(AccountKind::DepositAccount)
            .call().await.expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc.create()
            .account_id(acc).effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly).rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        let future = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .target(Amount::new(Decimal::from(9000_i32), CommodityCode::new("AUD")))
            .period(Period::Monthly).rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now()).build();
        svc.revise(budget.id(), future).await.expect("revise");
        let all = svc.revisions(budget.id()).await.expect("revisions");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].effective_from(), Date::constant(2026, 1, 1));
        assert_eq!(all[1].effective_from(), Date::constant(2027, 1, 1));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cannot_remove_last_revision(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Food")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, rev) = svc.create()
            .account_id(acc).effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly).rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        let err = svc.remove_revision(budget.id(), rev.id()).await;
        assert!(matches!(err, Err(crate::BcError::InvalidInput(_))), "got {err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revise_capattarget_without_target_rejected(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Fun")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc.create()
            .account_id(acc).effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly).rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        let bad = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2026, 6, 1))
            .period(Period::Weekly).rollover(RolloverPolicy::CapAtTarget)
            .created_at(Timestamp::now()).build();
        assert!(matches!(svc.revise(budget.id(), bad).await, Err(crate::BcError::InvalidInput(_))));
    }
}
