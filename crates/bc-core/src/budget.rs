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
        drop(self.get(budget_id).await?);
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

/// Computed budget status for one budget over a viewing window.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct BudgetStatus {
    /// The budget anchor this status is for.
    pub budget: bc_models::Budget,
    /// The viewing window.
    pub window: bc_models::BudgetWindow,
    /// Revision governing the window start (config the consumer should display).
    pub governing: Option<bc_models::BudgetRevision>,
    /// Sum of revision targets across the window's resolved periods (pro-rated to overlap).
    pub allocated: bc_models::Decimal,
    /// Commodity of the monetary values, if determinable.
    pub commodity: Option<bc_models::CommodityCode>,
    /// Sum of matched postings in the window.
    pub actuals: bc_models::Decimal,
    /// Rollover into the first resolved period in the window.
    pub rollover: bc_models::Decimal,
    /// `allocated + rollover - actuals`.
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
    /// Allocations are summed across all resolved periods overlapping the window, with each
    /// segment pro-rated to its overlap with the window. Actuals are summed only within
    /// `[window.start, window.end)`. Rollover is carried into the first resolved period.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `window.end < window.start`.
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for_window(
        &self,
        budget: &bc_models::Budget,
        window: bc_models::BudgetWindow,
    ) -> crate::BcResult<BudgetStatus> {
        if window.days() < 0 {
            return Err(crate::BcError::InvalidInput(format!(
                "BudgetWindow has end before start: {} to {}", window.start, window.end)));
        }
        let svc = BudgetService::new(self.pool.clone());
        let revisions = svc.revisions(budget.id()).await?;
        let account_id = budget.account_id().clone();

        let periods = bc_models::periods_overlapping(&revisions, window.start, window.end);

        let mut allocated = bc_models::Decimal::ZERO;
        let mut actuals = bc_models::Decimal::ZERO;
        let mut commodity: Option<bc_models::CommodityCode> = None;
        for p in &periods {
            // Clip to window for actuals and proration.
            let seg_start = p.start.max(window.start);
            let seg_end = p.end.min(window.end);
            allocated = allocated
                .checked_add(Self::period_target_prorated(p.revision, seg_start, seg_end))
                .ok_or_else(|| crate::BcError::BadData("allocated overflow".into()))?;
            let (a, c) = self.sum_actuals(&account_id, p.revision, seg_start, seg_end).await?;
            actuals = actuals.checked_add(a)
                .ok_or_else(|| crate::BcError::BadData("actuals overflow".into()))?;
            if commodity.is_none() { commodity = c; }
        }

        let governing = bc_models::governing_revision(&revisions, window.start).cloned();
        let rollover = match periods.first() {
            Some(first) => self.rollover_into(&account_id, &revisions, first.start).await?,
            None => bc_models::Decimal::ZERO,
        };
        #[expect(clippy::arithmetic_side_effects, reason = "decimal budget arithmetic")]
        let available = allocated + rollover - actuals;

        Ok(BudgetStatus { budget: budget.clone(), window, governing, allocated, commodity, actuals, rollover, available })
    }

    /// Computes the budget status for `budget` as of `as_of`.
    ///
    /// Builds the natural period containing `as_of` under the governing revision at that date,
    /// then delegates to [`Self::status_for_window`]. If no revision governs `as_of` (date
    /// precedes all revisions), returns an empty status with all fields zero.
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
        let svc = BudgetService::new(self.pool.clone());
        let revisions = svc.revisions(budget.id()).await?;
        let (start, end) = match bc_models::governing_revision(&revisions, as_of) {
            Some(rev) => {
                // Re-anchored period containing as_of within this reign.
                let mut s = rev.effective_from();
                loop {
                    let e = rev.period().advance(s);
                    if e > as_of { break (s, e); }
                    s = e;
                }
            }
            None => return Ok(BudgetStatus {
                budget: budget.clone(),
                window: bc_models::BudgetWindow::custom(as_of, as_of, "n/a"),
                governing: None, allocated: bc_models::Decimal::ZERO, commodity: None,
                actuals: bc_models::Decimal::ZERO, rollover: bc_models::Decimal::ZERO,
                available: bc_models::Decimal::ZERO }),
        };
        let label = format!("{start} \u{2013} {end}");
        self.status_for_window(budget, bc_models::BudgetWindow::custom(start, end, label)).await
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

    /// Sums actuals for `account_id` governed by `rev` in `[period_start, period_end)`.
    ///
    /// Returns the total and the commodity it is denominated in.  For revisions with a target
    /// commodity, foreign postings are converted via the FX service (and skipped with a warning
    /// if conversion is unavailable).  For tracking-only revisions, postings are grouped by
    /// commodity and the dominant group (by absolute value) is returned.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    async fn sum_actuals(
        &self,
        account_id: &bc_models::AccountId,
        rev: &bc_models::BudgetRevision,
        period_start: jiff::civil::Date,
        period_end: jiff::civil::Date,
    ) -> crate::BcResult<(bc_models::Decimal, Option<bc_models::CommodityCode>)> {
        let voided_str = crate::db::to_db_str(bc_models::TransactionStatus::Voided)?;
        let rows = self
            .fetch_posting_amounts(
                account_id,
                period_start,
                period_end,
                rev.tag_filter(),
                &voided_str,
            )
            .await?;

        let target_commodity: Option<bc_models::CommodityCode> =
            rev.target().map(|t| t.commodity().clone());

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
                            %account_id,
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
                    %account_id,
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

    /// Target pro-rated to a (possibly stub) period's day count.
    fn period_target_prorated(
        rev: &bc_models::BudgetRevision,
        start: jiff::civil::Date,
        end: jiff::civil::Date,
    ) -> bc_models::Decimal {
        let Some(target) = rev.target() else { return bc_models::Decimal::ZERO };
        let natural_end = rev.period().advance(start);
        #[expect(clippy::arithmetic_side_effects, reason = "Date - Date Span; realistic ranges")]
        let period_days = i64::from((natural_end - start).get_days());
        #[expect(clippy::arithmetic_side_effects, reason = "Date - Date Span; realistic ranges")]
        let actual_days = i64::from((end - start).get_days());
        if period_days <= 0 { return target.value(); }
        #[expect(clippy::arithmetic_side_effects, reason = "guarded by period_days > 0")]
        let ratio = bc_models::Decimal::from(actual_days) / bc_models::Decimal::from(period_days);
        #[expect(clippy::arithmetic_side_effects, reason = "decimal mul bounded by target")]
        let v = (target.value() * ratio).round_dp(2);
        v
    }

    /// Rollover carried into the period beginning at `period_start`.
    ///
    /// Walks backward across reign boundaries. Carry occurs only when BOTH the
    /// source period's revision and the destination period's revision use a
    /// carrying policy (`CarryForward`/`CapAtTarget`); if either is
    /// `ResetToZero`, the destination starts at zero. `CapAtTarget` clamps on the
    /// destination side. Stub periods are pro-rated by day count.
    fn rollover_into<'a>(
        &'a self,
        account_id: &'a bc_models::AccountId,
        revisions: &'a [bc_models::BudgetRevision],
        period_start: jiff::civil::Date,
    ) -> core::pin::Pin<Box<dyn core::future::Future<
        Output = crate::BcResult<bc_models::Decimal>> + Send + 'a>> {
        Box::pin(async move {
            let Some(dst) = bc_models::governing_revision(revisions, period_start) else {
                return Ok(bc_models::Decimal::ZERO);
            };
            if matches!(dst.rollover(), bc_models::RolloverPolicy::ResetToZero) {
                return Ok(bc_models::Decimal::ZERO);
            }
            if matches!(dst.period(), bc_models::Period::Custom { .. }) {
                return Ok(bc_models::Decimal::ZERO);
            }
            // Find the period immediately preceding period_start.
            let prev_day = period_start.checked_sub(jiff::Span::new().days(1_i32))
                .map_err(|e| crate::BcError::BadData(format!("period underflow: {e}")))?;
            let earliest = revisions.first().map(bc_models::BudgetRevision::effective_from);
            if earliest.is_none_or(|e| prev_day < e) {
                return Ok(bc_models::Decimal::ZERO);
            }
            // The previous period is the last resolved period strictly before period_start.
            let prev_periods = bc_models::periods_overlapping(
                revisions, earliest.unwrap_or(period_start), period_start);
            let Some(prev) = prev_periods.into_iter().rfind(|p| p.end <= period_start)
            else { return Ok(bc_models::Decimal::ZERO) };
            // Both-sides rule: source must also carry.
            if matches!(prev.revision.rollover(), bc_models::RolloverPolicy::ResetToZero) {
                return Ok(bc_models::Decimal::ZERO);
            }
            let prev_allocated = Self::period_target_prorated(prev.revision, prev.start, prev.end);
            let (prev_actuals, _) =
                self.sum_actuals(account_id, prev.revision, prev.start, prev.end).await?;
            let prev_rollover = self.rollover_into(account_id, revisions, prev.start).await?;
            #[expect(clippy::arithmetic_side_effects, reason = "decimal budget arithmetic")]
            let surplus = prev_allocated + prev_rollover - prev_actuals;
            Ok(match dst.rollover() {
                bc_models::RolloverPolicy::CarryForward => surplus,
                bc_models::RolloverPolicy::CapAtTarget => {
                    #[expect(clippy::expect_used, reason = "CapAtTarget validated to have target")]
                    let cap = dst.target().expect("CapAtTarget requires target").value();
                    surplus.max(bc_models::Decimal::ZERO).min(cap)
                }
                bc_models::RolloverPolicy::ResetToZero => bc_models::Decimal::ZERO,
                _ => {
                    tracing::warn!(
                        policy = ?dst.rollover(),
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
    use bc_models::Transaction;
    use bc_models::TransactionStatus;
    use rust_decimal_macros::dec;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::BudgetService;
    use super::BudgetStatusEngine;
    use crate::account::Service as AccountService;
    use crate::fx::noop_fx;
    use crate::transaction::Service as TransactionService;

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
        #[expect(clippy::indexing_slicing, reason = "index known valid: asserted len == 2 above")]
        {
            assert_eq!(all[0].effective_from(), Date::constant(2026, 1, 1));
            assert_eq!(all[1].effective_from(), Date::constant(2027, 1, 1));
        }
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

    /// Shared setup for rollover-across-boundary tests.
    async fn rollover_across_boundary_case(
        pool: sqlx::SqlitePool,
        src_policy: RolloverPolicy,
        dst_policy: RolloverPolicy,
        expected: Decimal,
    ) {
        // Revision 1 (src): Jul 2030 monthly, target 100, spend 60 -> surplus 40.
        // Revision 2 (dst): Aug 1 2030 monthly. Rollover into Aug depends on policies.
        let accounts = AccountService::new(pool.clone());
        let budget_acc = accounts.create().name("Groceries")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("acc");
        let offset = accounts.create().name("Checking")
            .account_type(AccountType::Asset).kind(AccountKind::DepositAccount)
            .call().await.expect("offset");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc.create()
            .account_id(budget_acc.clone())
            .effective_from(Date::constant(2030, 7, 1))
            .target(Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")))
            .period(Period::Monthly).rollover(src_policy)
            .call().await.expect("create");
        svc.revise(budget.id(), bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2030, 8, 1))
            .target(Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")))
            .period(Period::Monthly).rollover(dst_policy)
            .created_at(Timestamp::now()).build()).await.expect("revise");

        let txns = TransactionService::new(pool.clone());
        txns.create(Transaction::builder().id(bc_models::TransactionId::new())
            .date(Date::constant(2030, 7, 15)).description("Shop")
            .postings(vec![
                Posting::builder().id(PostingId::new()).account_id(budget_acc)
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD"))).build(),
                Posting::builder().id(PostingId::new()).account_id(offset)
                    .amount(Amount::new(dec!(-60), CommodityCode::new("AUD"))).build(),
            ]).status(TransactionStatus::Cleared).created_at(Timestamp::now()).build())
            .await.expect("tx");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let status = engine.status_for(&budget, Date::constant(2030, 8, 15)).await.expect("status");
        assert_eq!(status.rollover, expected);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_carry_into_carry_preserves_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::CarryForward,
            RolloverPolicy::CarryForward,
            dec!(40),
        ).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_carry_into_reset_drops_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::CarryForward,
            RolloverPolicy::ResetToZero,
            dec!(0),
        ).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_reset_into_carry_drops_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::ResetToZero,
            RolloverPolicy::CarryForward,
            dec!(0),
        ).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn future_revision_dormant_until_effective(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Groceries")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("acc");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc.create().account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(Decimal::from(200_i32), CommodityCode::new("AUD")))
            .period(Period::Weekly).rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        svc.revise(budget.id(), bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone()).effective_from(Date::constant(2027, 1, 1))
            .target(Amount::new(Decimal::from(250_i32), CommodityCode::new("AUD")))
            .period(Period::Weekly).rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now()).build()).await.expect("revise");
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        // A week in 2026 uses the $200 revision.
        let s = engine.status_for(&budget, Date::constant(2026, 6, 3)).await.expect("status");
        assert_eq!(s.allocated, dec!(200));
        assert_eq!(
            s.governing.as_ref().expect("governing revision set").effective_from(),
            Date::constant(2026, 1, 1)
        );
        // A week in 2027 uses the $250 revision.
        let s2 = engine.status_for(&budget, Date::constant(2027, 6, 3)).await.expect("status");
        assert_eq!(s2.allocated, dec!(250));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_spanning_boundary_aggregates_periods(pool: sqlx::SqlitePool) {
        // Monthly $300 from Jan; $600 from Apr 1. A Q1+Q2-ish window sums both.
        let accounts = AccountService::new(pool.clone());
        let acc = accounts.create().name("Groceries")
            .account_type(AccountType::Expense).kind(AccountKind::DepositAccount)
            .call().await.expect("acc");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc.create().account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(Decimal::from(300_i32), CommodityCode::new("AUD")))
            .period(Period::Monthly).rollover(RolloverPolicy::ResetToZero)
            .call().await.expect("create");
        svc.revise(budget.id(), bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone()).effective_from(Date::constant(2026, 4, 1))
            .target(Amount::new(Decimal::from(600_i32), CommodityCode::new("AUD")))
            .period(Period::Monthly).rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now()).build()).await.expect("revise");
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        // Window Feb 1 .. May 1 = Feb,Mar @300 + Apr @600 = 1200.
        let w = bc_models::BudgetWindow::custom(
            Date::constant(2026, 2, 1), Date::constant(2026, 5, 1), "FebApr");
        let s = engine.status_for_window(&budget, w).await.expect("status");
        assert_eq!(s.allocated, dec!(1200));
    }
}
