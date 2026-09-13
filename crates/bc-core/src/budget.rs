//! Budget calculation engine: actuals, rollover, and budget status.

use std::collections::HashSet;

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
        let id = row.id.parse::<bc_models::BudgetRevisionId>().map_err(|e| {
            crate::BcError::BadData(format!("invalid revision id '{}': {e}", row.id))
        })?;
        let budget_id = row.budget_id.parse::<bc_models::BudgetId>().map_err(|e| {
            crate::BcError::BadData(format!("invalid budget_id '{}': {e}", row.budget_id))
        })?;
        let effective_from = row
            .effective_from
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
    /// Returns [`crate::BcError::InvalidInput`] if a different revision already occupies the
    /// same `effective_from` date (amending a revision in place — same id — is always allowed).
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
        let existing = self.revisions(budget_id).await?;
        if let Some(conflict) = existing
            .iter()
            .find(|r| r.effective_from() == revision.effective_from() && r.id() != revision.id())
        {
            return Err(crate::BcError::InvalidInput(format!(
                "a revision already exists for effective date {} (id: {})",
                conflict.effective_from(),
                conflict.id()
            )));
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
    /// Native amounts that fed no total, summed by commodity: postings no FX
    /// rate could value in the target commodity, and every non-dominant
    /// commodity group of a tracking-only budget, across the window periods
    /// and the carry chain alike. `available` is exact only when this is
    /// empty.
    pub unvalued: bc_models::Balances,
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

/// Builds the dynamic SELECT for [`BudgetStatusEngine::fetch_posting_rows`].
///
/// Assembles the account-subtree (and optional tag-subtree) CTEs plus the date
/// range and every active non-amount filter clause, in the exact order their
/// binds are applied by the caller: account-tree root, optional tag-subtree
/// root, period start/end, text (×2), reconciliation, sorted filter account
/// ids, and tags (×2). The returned string must not be reordered independently
/// of that bind chain.
///
/// # Arguments
///
/// * `tag_filter` - Optional tag whose subtree a counted posting must carry,
///   via either its own tags or its transaction's tags (transaction tags flow
///   down to every posting).
/// * `query` - Optional global transaction query supplying extra clauses.
/// * `filter_accounts` - Pre-resolved account-subtree ids from `query`.
///
/// # Returns
///
/// The finished SQL SELECT string.
fn build_posting_amounts_sql(
    tag_filter: Option<&bc_models::TagId>,
    query: Option<&crate::search::TransactionQuery>,
    filter_accounts: Option<&std::collections::HashSet<bc_models::AccountId>>,
) -> String {
    let mut sql = String::from(
        "WITH RECURSIVE acct_tree(id) AS ( \
           SELECT ? UNION ALL \
           SELECT a.id FROM accounts a \
           INNER JOIN acct_tree ON a.parent_id = acct_tree.id \
         )",
    );
    if tag_filter.is_some() {
        sql.push_str(
            ", tag_subtree(id) AS ( \
               SELECT ? UNION ALL \
               SELECT tg.id FROM tags tg \
               INNER JOIN tag_subtree ON tg.parent_id = tag_subtree.id \
             )",
        );
    }
    sql.push_str(
        " SELECT p.id, t.date AS date, p.amount, p.commodity FROM postings p \
          JOIN transactions t ON t.id = p.transaction_id \
          WHERE p.account_id IN (SELECT id FROM acct_tree) \
            AND t.date >= ? AND t.date < ?",
    );
    if tag_filter.is_some() {
        // A transaction-level tag flows down to every posting, so the budget's
        // own tag filter is satisfied when either this posting or its
        // transaction carries a tag in the filter's subtree. Both branches read
        // the same `tag_subtree` CTE, so no extra bind is introduced.
        sql.push_str(
            " AND (EXISTS ( \
                SELECT 1 FROM posting_tags pt WHERE pt.posting_id = p.id \
                AND pt.tag_id IN (SELECT id FROM tag_subtree)) \
              OR EXISTS ( \
                SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = t.id \
                AND tt.tag_id IN (SELECT id FROM tag_subtree)))",
        );
    }

    if let Some(q) = query {
        if q.text.is_some() {
            sql.push_str(" AND lower(t.description) LIKE ? ESCAPE '\\'");
        }
        if q.reconciliation.is_some() {
            sql.push_str(" AND t.reconciliation = ?");
        }
        if let Some(set) = filter_accounts {
            let ph = crate::transaction::sql_placeholders(set.len());
            sql.push_str(" AND p.account_id IN (");
            sql.push_str(&ph);
            sql.push(')');
        }
        if !q.tags.is_empty() {
            let ph = crate::transaction::sql_placeholders(q.tags.len());
            sql.push_str(
                " AND (EXISTS (SELECT 1 FROM transaction_tags tt \
                        WHERE tt.transaction_id = t.id AND tt.tag_id IN (",
            );
            sql.push_str(&ph);
            sql.push_str(
                ")) OR EXISTS (SELECT 1 FROM posting_tags pt \
                        WHERE pt.posting_id = p.id AND pt.tag_id IN (",
            );
            sql.push_str(&ph);
            sql.push_str(")))");
        }
    }

    sql
}

/// One row of the actuals query.
///
/// `amount` and `commodity` are both `None` for an elided leg, whose value is
/// derived from its transaction's residual rather than read from the row.
#[derive(sqlx::FromRow)]
struct PostingRow {
    /// Raw posting ID.
    id: String,
    /// Transaction date as stored, ISO `YYYY-MM-DD`.
    date: String,
    /// Stored amount, or `None` when elided.
    amount: Option<String>,
    /// Stored commodity, or `None` when elided.
    commodity: Option<String>,
}

/// Turns raw actuals rows into dated concrete amounts, expanding each elided
/// leg into its per-commodity residual.
///
/// # Arguments
///
/// * `rows` - Rows from [`BudgetStatusEngine::fetch_posting_rows`].
/// * `residuals` - Loaded for exactly the subtree and date range `rows` came
///   from. `None` is only valid when no row is elided.
///
/// # Returns
///
/// One `(date, amount)` per concrete row, plus one per commodity component of
/// each attributable elided row, all carrying the row's transaction date. An
/// ambiguous elided row contributes nothing. Because expansion precedes the
/// fold, a later amount filter sees each commodity component of an elided leg
/// as its own amount rather than the leg as a whole.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if a stored amount or date cannot be
/// parsed, if a row is half-NULL, or if an elided row falls outside
/// `residuals`' scope.
fn expand_posting_rows(
    rows: Vec<PostingRow>,
    residuals: Option<&crate::residual::Residuals>,
) -> crate::BcResult<Vec<(jiff::civil::Date, bc_models::Amount)>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let date = row.date.parse::<jiff::civil::Date>().map_err(|e| {
            crate::BcError::BadData(format!(
                "invalid posting date '{}' on '{}': {e}",
                row.date, row.id
            ))
        })?;
        match (row.amount, row.commodity) {
            (Some(amt_str), Some(comm_str)) => {
                let value = amt_str.parse::<bc_models::Decimal>().map_err(|e| {
                    crate::BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
                })?;
                out.push((
                    date,
                    bc_models::Amount::new(value, bc_models::CommodityCode::new(comm_str)),
                ));
            }
            (None, None) => {
                let Some(loaded_residuals) = residuals else {
                    return Err(crate::BcError::BadData(format!(
                        "elided posting '{}' reached the fold without a residual load",
                        row.id
                    )));
                };
                if let Some(balances) = loaded_residuals.residual(&row.id)? {
                    for (code, value) in balances.iter() {
                        out.push((
                            date,
                            bc_models::Amount::new(value, bc_models::CommodityCode::new(code)),
                        ));
                    }
                }
            }
            _ => {
                return Err(crate::BcError::BadData(format!(
                    "posting '{}' stores an amount without a commodity, or vice versa",
                    row.id
                )));
            }
        }
    }
    Ok(out)
}

/// One actuals load: a date range under one revision's tag filter.
#[derive(Debug, PartialEq, Eq)]
struct Load<'a> {
    /// Inclusive start.
    start: jiff::civil::Date,
    /// Exclusive end.
    end: jiff::civil::Date,
    /// The reign's tag filter.
    tag_filter: Option<&'a bc_models::TagId>,
    /// Whether the user's transaction query applies to this load.
    with_query: bool,
}

/// Plans the loads a status needs: at most two per reign.
///
/// The carry chain is always loaded without the user query, because rollover
/// is a property of the budget rather than of the current view. The window is
/// loaded with the query. Without a query the two ranges share one SQL shape
/// and merge into a single span per reign, so a status with no query issues
/// exactly one load per reign it touches.
///
/// # Arguments
///
/// * `revisions` - Revisions sorted ascending by `effective_from`.
/// * `chain` - `[chain_start, first_window_period_start)`, or `None` when no
///   carry reaches the window.
/// * `window` - `[window.start, window.end)`.
/// * `has_query` - Whether a user query narrows the window loads.
///
/// # Returns
///
/// Loads in reign order, then range order within a reign, each clipped to its
/// reign. Empty intersections are dropped.
fn plan_loads(
    revisions: &[bc_models::BudgetRevision],
    chain: Option<(jiff::civil::Date, jiff::civil::Date)>,
    window: (jiff::civil::Date, jiff::civil::Date),
    has_query: bool,
) -> Vec<Load<'_>> {
    let (window_start, window_end) = window;
    let ranges: Vec<(jiff::civil::Date, jiff::civil::Date, bool)> = match chain {
        Some((chain_start, _)) if !has_query => vec![(chain_start, window_end, false)],
        Some((chain_start, chain_end)) => vec![
            (chain_start, chain_end, false),
            (window_start, window_end, true),
        ],
        None => vec![(window_start, window_end, has_query)],
    };
    let mut out = Vec::new();
    for (i, rev) in revisions.iter().enumerate() {
        let reign_start = rev.effective_from();
        let reign_end = revisions
            .get(i.saturating_add(1))
            .map(bc_models::BudgetRevision::effective_from);
        for &(from, to, with_query) in &ranges {
            let start = from.max(reign_start);
            let end = reign_end.map_or(to, |re| to.min(re));
            if start < end {
                out.push(Load {
                    start,
                    end,
                    tag_filter: rev.tag_filter(),
                    with_query,
                });
            }
        }
    }
    out
}

/// A half-open date range whose matched amounts fold together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    /// Inclusive start.
    start: jiff::civil::Date,
    /// Exclusive end.
    end: jiff::civil::Date,
}

/// Index of the segment containing `date`, if any.
///
/// `segments` must be sorted and pairwise disjoint; gaps between segments are
/// allowed and dates falling in a gap return `None`.
fn segment_index(segments: &[Segment], date: jiff::civil::Date) -> Option<usize> {
    let idx = segments.partition_point(|s| s.end <= date);
    segments.get(idx).filter(|s| s.start <= date).map(|_| idx)
}

/// Segments a status buckets its rows into: the chain periods whole, then the
/// window periods clipped to `window`.
///
/// A row dated in the clipped-off head of the first window period lands in no
/// segment and is dropped.
///
/// # Returns
///
/// The segments in date order and the count of leading chain segments.
fn bucket_segments(
    chain_periods: &[bc_models::ResolvedPeriod<'_>],
    periods: &[bc_models::ResolvedPeriod<'_>],
    window: &bc_models::BudgetWindow,
) -> (Vec<Segment>, usize) {
    let mut segments: Vec<Segment> = chain_periods
        .iter()
        .map(|p| Segment {
            start: p.start,
            end: p.end,
        })
        .collect();
    let chain_len = segments.len();
    segments.extend(periods.iter().map(|p| Segment {
        start: p.start.max(window.start),
        end: p.end.min(window.end),
    }));
    (segments, chain_len)
}

/// The carry `dst` accepts from a `surplus` left by the period before it.
fn apply_rollover_policy(
    dst: &bc_models::BudgetRevision,
    surplus: bc_models::Decimal,
) -> bc_models::Decimal {
    match dst.rollover() {
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
    }
}

/// One bucket's actuals after the fold.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PeriodActuals {
    /// Sum of every valued amount, in `commodity`.
    total: bc_models::Decimal,
    /// Commodity of `total`, if determinable.
    commodity: Option<bc_models::CommodityCode>,
    /// Native amounts counted in no total, by commodity.
    unvalued: bc_models::Balances,
}

/// Adds `amount` to `unvalued`, mapping overflow to `BadData`.
fn add_unvalued(
    unvalued: &mut bc_models::Balances,
    amount: &bc_models::Amount,
) -> crate::BcResult<()> {
    unvalued.try_add(amount).map_err(|e| {
        crate::BcError::BadData(format!(
            "unvalued overflow for '{}': {e}",
            amount.commodity()
        ))
    })
}

/// Folds every entry of `from` into `into`.
fn merge_unvalued(
    into: &mut bc_models::Balances,
    from: &bc_models::Balances,
) -> crate::BcResult<()> {
    for (code, value) in from.iter() {
        add_unvalued(into, &bc_models::Amount::new(value, code))?;
    }
    Ok(())
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
    /// `[window.start, window.end)`. Rollover into the first resolved period is folded
    /// forward from the start of its carry chain, loading each revision reign at most
    /// twice (once for the chain, once for the window when `query` is set).
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
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<BudgetStatus> {
        if window.days() < 0 {
            return Err(crate::BcError::InvalidInput(format!(
                "BudgetWindow has end before start: {} to {}",
                window.start, window.end
            )));
        }
        let svc = BudgetService::new(self.pool.clone());
        let revisions = svc.revisions(budget.id()).await?;
        let account_id = budget.account_id().clone();
        let filter_accounts = match query {
            Some(q) => crate::search::resolve_account_subtrees(&self.pool, &q.accounts).await?,
            None => None,
        };
        let amount_q = query.and_then(|q| q.amount.as_ref());

        let periods = bc_models::periods_overlapping(&revisions, window.start, window.end);
        let first_start = periods.first().map(|p| p.start);
        let chain_start = first_start.and_then(|fs| bc_models::carry_chain_start(&revisions, fs));
        let chain_periods = match (chain_start, first_start) {
            (Some(cs), Some(fs)) => bc_models::periods_overlapping(&revisions, cs, fs),
            _ => Vec::new(),
        };

        let (segments, chain_len) = bucket_segments(&chain_periods, &periods, &window);
        let mut buckets: Vec<Vec<bc_models::Amount>> = vec![Vec::new(); segments.len()];

        let chain = chain_start.zip(first_start);
        for load in plan_loads(
            &revisions,
            chain,
            (window.start, window.end),
            query.is_some(),
        ) {
            let amounts = self
                .fetch_posting_amounts(
                    &account_id,
                    load.start,
                    load.end,
                    load.tag_filter,
                    if load.with_query { query } else { None },
                    filter_accounts.as_ref(),
                )
                .await?;
            for (date, amount) in amounts {
                if let Some(bucket) =
                    segment_index(&segments, date).and_then(|i| buckets.get_mut(i))
                {
                    bucket.push(amount);
                }
            }
        }
        let (chain_buckets, window_buckets) = buckets.split_at(chain_len);

        let mut allocated = bc_models::Decimal::ZERO;
        let mut actuals = bc_models::Decimal::ZERO;
        let mut commodity: Option<bc_models::CommodityCode> = None;
        let mut unvalued = bc_models::Balances::new();
        for (p, bucket) in periods.iter().zip(window_buckets) {
            let seg_start = p.start.max(window.start);
            let seg_end = p.end.min(window.end);
            allocated = allocated
                .checked_add(Self::period_target_prorated(
                    p.revision, p.start, seg_start, seg_end,
                ))
                .ok_or_else(|| crate::BcError::BadData("allocated overflow".into()))?;
            let period = self.fold_actuals(p.revision, bucket, amount_q)?;
            actuals = actuals
                .checked_add(period.total)
                .ok_or_else(|| crate::BcError::BadData("actuals overflow".into()))?;
            merge_unvalued(&mut unvalued, &period.unvalued)?;
            if commodity.is_none() {
                commodity = period.commodity;
            }
        }

        let governing = bc_models::governing_revision(&revisions, window.start).cloned();
        let rollover =
            match first_start.and_then(|fs| bc_models::governing_revision(&revisions, fs)) {
                Some(dst) => {
                    let (carry, chain_unvalued) =
                        self.fold_rollover(dst, &chain_periods, chain_buckets)?;
                    merge_unvalued(&mut unvalued, &chain_unvalued)?;
                    carry
                }
                None => bc_models::Decimal::ZERO,
            };
        #[expect(clippy::arithmetic_side_effects, reason = "decimal budget arithmetic")]
        let available = allocated + rollover - actuals;

        Ok(BudgetStatus {
            budget: budget.clone(),
            window,
            governing,
            allocated,
            commodity,
            actuals,
            rollover,
            available,
            unvalued,
        })
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
                    if e > as_of {
                        break (s, e);
                    }
                    s = e;
                }
            }
            None => {
                return Ok(BudgetStatus {
                    budget: budget.clone(),
                    window: bc_models::BudgetWindow::custom(as_of, as_of, "n/a"),
                    governing: None,
                    allocated: bc_models::Decimal::ZERO,
                    commodity: None,
                    actuals: bc_models::Decimal::ZERO,
                    rollover: bc_models::Decimal::ZERO,
                    available: bc_models::Decimal::ZERO,
                    unvalued: bc_models::Balances::new(),
                });
            }
        };
        let label = format!("{start} \u{2013} {end}");
        self.status_for_window(
            budget,
            bc_models::BudgetWindow::custom(start, end, label),
            None,
        )
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
    /// Fetches raw rows for postings to `account_id` or any descendant account in
    /// `[from, to)`, optionally filtered by tag.
    ///
    /// The dynamic SELECT is assembled by [`build_posting_amounts_sql`], whose
    /// clause order the bind chain below relies on exactly. The caller resolves
    /// `filter_accounts` up front, so the snapshot transaction holds the only
    /// pool connection.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure.
    #[inline]
    async fn fetch_posting_rows(
        conn: &mut sqlx::SqliteConnection,
        account_id: &bc_models::AccountId,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
        tag_filter: Option<&bc_models::TagId>,
        query: Option<&crate::search::TransactionQuery>,
        filter_accounts: Option<&HashSet<bc_models::AccountId>>,
    ) -> crate::BcResult<Vec<PostingRow>> {
        let sql = build_posting_amounts_sql(tag_filter, query, filter_accounts);

        let mut stmt = sqlx::query_as::<_, PostingRow>(sqlx::AssertSqlSafe(sql));
        stmt = stmt.bind(account_id.to_string());
        if let Some(tag) = tag_filter {
            stmt = stmt.bind(tag.to_string());
        }
        stmt = stmt.bind(from.to_string()).bind(to.to_string());
        if let Some(q) = query {
            if let Some(text) = &q.text {
                let needle = format!(
                    "%{}%",
                    crate::search::escape_like(&text.to_ascii_lowercase())
                );
                stmt = stmt.bind(needle);
            }
            if let Some(rec) = q.reconciliation {
                stmt = stmt.bind(crate::db::to_db_str(rec)?);
            }
            if let Some(set) = filter_accounts {
                let mut ids: Vec<&bc_models::AccountId> = set.iter().collect();
                ids.sort_by_key(ToString::to_string);
                for id in ids {
                    stmt = stmt.bind(id.to_string());
                }
            }
            if !q.tags.is_empty() {
                for t in &q.tags {
                    stmt = stmt.bind(t.to_string());
                }
                for t in &q.tags {
                    stmt = stmt.bind(t.to_string());
                }
            }
        }

        stmt.fetch_all(conn).await.map_err(Into::into)
    }

    /// Fetches the concrete amounts of every matched posting in `[from, to)`,
    /// with elided legs resolved to their residuals.
    ///
    /// The row fetch and the residual load run in one read transaction so they
    /// describe the same snapshot: a date amendment landing between them would
    /// leave an elided id outside the loaded scope, which
    /// [`expand_posting_rows`] reports as an error rather than dropping.
    ///
    /// # Arguments
    ///
    /// * `account_id` - Root of the account subtree to fetch postings for.
    /// * `from` - Inclusive start of the load's date range.
    /// * `to` - Exclusive end of the load's date range.
    /// * `tag_filter` - Restricts to postings or transactions tagged within
    ///   this subtree.
    /// * `query` - Additional filters (text, reconciliation, accounts, tags,
    ///   amount).
    /// * `filter_accounts` - Account subtrees from `query`, resolved by the
    ///   caller so the snapshot transaction holds the only pool connection.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure or unparsable stored data.
    #[inline]
    async fn fetch_posting_amounts(
        &self,
        account_id: &bc_models::AccountId,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
        tag_filter: Option<&bc_models::TagId>,
        query: Option<&crate::search::TransactionQuery>,
        filter_accounts: Option<&HashSet<bc_models::AccountId>>,
    ) -> crate::BcResult<Vec<(jiff::civil::Date, bc_models::Amount)>> {
        let mut tx = self.pool.begin().await?;
        let rows = Self::fetch_posting_rows(
            &mut tx,
            account_id,
            from,
            to,
            tag_filter,
            query,
            filter_accounts,
        )
        .await?;
        let residuals = if rows.iter().any(|row| row.amount.is_none()) {
            Some(
                crate::residual::Residuals::for_subtree_in_range(&mut *tx, account_id, from, to)
                    .await?,
            )
        } else {
            None
        };
        // Nothing was written, so the snapshot is released rather than committed.
        tx.rollback().await?;
        expand_posting_rows(rows, residuals.as_ref())
    }

    /// Folds one bucket of posting amounts under `rev`.
    ///
    /// Under a target commodity every amount is converted with the FX
    /// service; an amount no rate can value goes to `unvalued`. Under
    /// tracking-only the amounts are grouped by commodity, the group with
    /// the largest absolute total is the result, and every other group goes
    /// to `unvalued` whole. Each amount is filtered by `amount_q` before
    /// either path (amounts derive from [`expand_posting_rows`], so an
    /// elided leg's components are matched one by one).
    ///
    /// # Arguments
    ///
    /// * `rev` - The revision governing this bucket's period.
    /// * `amounts` - The bucket.
    /// * `amount_q` - The user's amount filter, matched exactly in Rust.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] on decimal overflow.
    fn fold_actuals(
        &self,
        rev: &bc_models::BudgetRevision,
        amounts: &[bc_models::Amount],
        amount_q: Option<&crate::search::AmountQuery>,
    ) -> crate::BcResult<PeriodActuals> {
        let mut unvalued = bc_models::Balances::new();
        let matched = amounts
            .iter()
            .filter(|&posting_amount| amount_q.is_none_or(|aq| aq.matches(Some(posting_amount))));

        if let Some(target) = rev.target().map(|t| t.commodity().clone()) {
            let mut total = bc_models::Decimal::ZERO;
            for posting_amount in matched {
                match self.fx.convert(posting_amount, &target) {
                    Ok(a) => {
                        total = total.checked_add(a.value()).ok_or_else(|| {
                            crate::BcError::BadData("actuals sum overflow".into())
                        })?;
                    }
                    Err(_) => add_unvalued(&mut unvalued, posting_amount)?,
                }
            }
            return Ok(PeriodActuals {
                total,
                commodity: Some(target),
                unvalued,
            });
        }

        let mut groups: std::collections::BTreeMap<String, bc_models::Decimal> =
            std::collections::BTreeMap::new();
        for posting_amount in matched {
            let entry = groups
                .entry(posting_amount.commodity().to_string())
                .or_insert(bc_models::Decimal::ZERO);
            *entry = entry
                .checked_add(posting_amount.value())
                .ok_or_else(|| crate::BcError::BadData("actuals sum overflow".into()))?;
        }
        // A tie in absolute value resolves to the last (highest) commodity
        // code, since `max_by` keeps the later of two equal elements and
        // `BTreeMap::iter` yields entries in code order.
        let Some(dominant) = groups
            .iter()
            .max_by(|(_, a), (_, b)| {
                a.abs()
                    .partial_cmp(&b.abs())
                    .unwrap_or(core::cmp::Ordering::Equal)
            })
            .map(|(code, _)| code.clone())
        else {
            return Ok(PeriodActuals {
                total: bc_models::Decimal::ZERO,
                commodity: None,
                unvalued,
            });
        };
        let mut total = bc_models::Decimal::ZERO;
        for (code, value) in groups {
            if code == dominant {
                total = value;
            } else {
                add_unvalued(&mut unvalued, &bc_models::Amount::new(value, code))?;
            }
        }
        Ok(PeriodActuals {
            total,
            commodity: Some(bc_models::CommodityCode::new(dominant)),
            unvalued,
        })
    }

    /// Target pro-rated to the day count of `[seg_start, seg_end)`.
    ///
    /// The denominator is the revision's natural period length anchored at
    /// `period_start` (the period's true start), so the ratio stays correct when
    /// the segment is clipped to a window edge or truncated to a boundary stub.
    /// `period_start` must be the resolved period's start; `seg_start`/`seg_end`
    /// the (possibly clipped) portion being measured.
    #[inline]
    fn period_target_prorated(
        rev: &bc_models::BudgetRevision,
        period_start: jiff::civil::Date,
        seg_start: jiff::civil::Date,
        seg_end: jiff::civil::Date,
    ) -> bc_models::Decimal {
        let Some(target) = rev.target() else {
            return bc_models::Decimal::ZERO;
        };
        let natural_end = rev.period().advance(period_start);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date Span; realistic ranges"
        )]
        let period_days = i64::from((natural_end - period_start).get_days());
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date Span; realistic ranges"
        )]
        let actual_days = i64::from((seg_end - seg_start).get_days());
        if period_days <= 0 {
            return target.value();
        }
        #[expect(clippy::arithmetic_side_effects, reason = "guarded by period_days > 0")]
        let ratio = bc_models::Decimal::from(actual_days) / bc_models::Decimal::from(period_days);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "decimal mul bounded by target"
        )]
        let v = (target.value() * ratio).round_dp(2);
        v
    }

    /// Rollover carried into the period governed by `dst`, and every native
    /// amount in the chain that fed no total.
    ///
    /// `chain` is every period from [`bc_models::carry_chain_start`] up to the
    /// destination period, chronological, and `buckets` holds each one's
    /// amounts. The fold runs forward: each period's surplus, plus what it
    /// received, passes through the next period's policy. Carry occurs only
    /// when BOTH sides of a boundary carry; `carry_chain_start` has already
    /// cut the chain where that fails, so the fold only applies the
    /// destination policy at each step. `CapAtTarget` clamps on the
    /// destination side. Stub periods are pro-rated by day count.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] if folding a chain bucket's
    /// actuals overflows (see [`Self::fold_actuals`]).
    fn fold_rollover(
        &self,
        dst: &bc_models::BudgetRevision,
        chain: &[bc_models::ResolvedPeriod<'_>],
        buckets: &[Vec<bc_models::Amount>],
    ) -> crate::BcResult<(bc_models::Decimal, bc_models::Balances)> {
        let mut carry = bc_models::Decimal::ZERO;
        let mut unvalued = bc_models::Balances::new();
        for (k, (period, bucket)) in chain.iter().zip(buckets).enumerate() {
            let allocated = Self::period_target_prorated(
                period.revision,
                period.start,
                period.start,
                period.end,
            );
            let spent = self.fold_actuals(period.revision, bucket, None)?;
            merge_unvalued(&mut unvalued, &spent.unvalued)?;
            #[expect(clippy::arithmetic_side_effects, reason = "decimal budget arithmetic")]
            let surplus = allocated + carry - spent.total;
            let next = chain.get(k.saturating_add(1)).map_or(dst, |n| n.revision);
            carry = apply_rollover_policy(next, surplus);
        }
        Ok((carry, unvalued))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod budget_service_tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::RolloverPolicy;
    use bc_models::Transaction;
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
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, rev) = svc
            .create()
            .account_id(acc.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
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
        let acc = accounts
            .create()
            .name("Salary")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        let future = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .target(Amount::new(
                Decimal::from(9000_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        svc.revise(budget.id(), future).await.expect("revise");
        let all = svc.revisions(budget.id()).await.expect("revisions");
        assert_eq!(all.len(), 2);
        #[expect(
            clippy::indexing_slicing,
            reason = "index known valid: asserted len == 2 above"
        )]
        {
            assert_eq!(all[0].effective_from(), Date::constant(2026, 1, 1));
            assert_eq!(all[1].effective_from(), Date::constant(2027, 1, 1));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cannot_remove_last_revision(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, rev) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        let err = svc.remove_revision(budget.id(), rev.id()).await;
        assert!(
            matches!(err, Err(crate::BcError::InvalidInput(_))),
            "got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revise_capattarget_without_target_rejected(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Fun")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        let bad = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2026, 6, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::CapAtTarget)
            .created_at(Timestamp::now())
            .build();
        assert!(matches!(
            svc.revise(budget.id(), bad).await,
            Err(crate::BcError::InvalidInput(_))
        ));
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
        let budget_acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc");
        let offset = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("offset");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(budget_acc.clone())
            .effective_from(Date::constant(2030, 7, 1))
            .target(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(src_policy)
            .call()
            .await
            .expect("create");
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2030, 8, 1))
                .target(Amount::new(
                    Decimal::from(100_i32),
                    CommodityCode::new("AUD"),
                ))
                .period(Period::Monthly)
                .rollover(dst_policy)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");

        let txns = TransactionService::new(pool.clone());
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2030, 7, 15))
                .description("Shop")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(budget_acc)
                        .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(offset)
                        .amount(Amount::new(dec!(-60), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("tx");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let status = engine
            .status_for(&budget, Date::constant(2030, 8, 15))
            .await
            .expect("status");
        assert_eq!(status.rollover, expected);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_carry_into_carry_preserves_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::CarryForward,
            RolloverPolicy::CarryForward,
            dec!(40),
        )
        .await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_carry_into_reset_drops_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::CarryForward,
            RolloverPolicy::ResetToZero,
            dec!(0),
        )
        .await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollover_reset_into_carry_drops_surplus(pool: sqlx::SqlitePool) {
        rollover_across_boundary_case(
            pool,
            RolloverPolicy::ResetToZero,
            RolloverPolicy::CarryForward,
            dec!(0),
        )
        .await;
    }

    /// A daily budget carries its surplus across every day between two
    /// dates, so the rollover on day three is day one's surplus plus the
    /// whole of day two's untouched target.
    #[sqlx::test(migrations = "./migrations")]
    async fn daily_rollover_carries_across_days(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let budget_acc = accounts
            .create()
            .name("Interest")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc");
        let offset = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("offset");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(budget_acc.clone())
            .effective_from(Date::constant(2030, 7, 1))
            .target(Amount::new(
                Decimal::from(10_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Daily)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create");

        let txns = TransactionService::new(pool.clone());
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2030, 7, 1))
                .description("Interest")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(budget_acc)
                        .amount(Amount::new(dec!(4), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(offset)
                        .amount(Amount::new(dec!(-4), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("tx");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let status = engine
            .status_for(&budget, Date::constant(2030, 7, 3))
            .await
            .expect("status");
        assert_eq!(status.rollover, dec!(16));
        assert_eq!(status.available, dec!(26));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn future_revision_dormant_until_effective(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(
                Decimal::from(200_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2027, 1, 1))
                .target(Amount::new(
                    Decimal::from(250_i32),
                    CommodityCode::new("AUD"),
                ))
                .period(Period::Weekly)
                .rollover(RolloverPolicy::ResetToZero)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        // A week in 2026 uses the $200 revision.
        let s = engine
            .status_for(&budget, Date::constant(2026, 6, 3))
            .await
            .expect("status");
        assert_eq!(s.allocated, dec!(200));
        assert_eq!(
            s.governing
                .as_ref()
                .expect("governing revision set")
                .effective_from(),
            Date::constant(2026, 1, 1)
        );
        // A week in 2027 uses the $250 revision.
        let s2 = engine
            .status_for(&budget, Date::constant(2027, 6, 3))
            .await
            .expect("status");
        assert_eq!(s2.allocated, dec!(250));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_spanning_boundary_aggregates_periods(pool: sqlx::SqlitePool) {
        // Monthly $300 from Jan; $600 from Apr 1. A Q1+Q2-ish window sums both.
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(
                Decimal::from(300_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2026, 4, 1))
                .target(Amount::new(
                    Decimal::from(600_i32),
                    CommodityCode::new("AUD"),
                ))
                .period(Period::Monthly)
                .rollover(RolloverPolicy::ResetToZero)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        // Window Feb 1 .. May 1 = Feb,Mar @300 + Apr @600 = 1200.
        let w = bc_models::BudgetWindow::custom(
            Date::constant(2026, 2, 1),
            Date::constant(2026, 5, 1),
            "FebApr",
        );
        let s = engine
            .status_for_window(&budget, w, None)
            .await
            .expect("status");
        assert_eq!(s.allocated, dec!(1200));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_clipped_mid_period_prorates_on_true_period_length(pool: sqlx::SqlitePool) {
        // Monthly $310 from Jan 1 2026 (period Jan 1 .. Feb 1 = 31 days). A window
        // that starts mid-period (Jan 29 .. Feb 1 = 3 days) must pro-rate against
        // the period's true length (31), not the natural length anchored at the
        // clipped start: 310 * 3/31 = 30.00.
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("acc");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(
                Decimal::from(310_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let w = bc_models::BudgetWindow::custom(
            Date::constant(2026, 1, 29),
            Date::constant(2026, 2, 1),
            "tail",
        );
        let s = engine
            .status_for_window(&budget, w, None)
            .await
            .expect("status");
        assert_eq!(s.allocated, dec!(30));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revise_duplicate_effective_from_on_different_id_is_rejected(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Rent")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");

        let svc = BudgetService::new(pool.clone());
        // Create budget with revision A effective 2026-01-01.
        let (budget, _rev_a) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");

        // Add revision B effective 2027-01-01.
        let rev_b = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        svc.revise(budget.id(), rev_b)
            .await
            .expect("add revision B");

        // Attempt to revise with a THIRD revision (fresh id) using the same
        // effective_from as revision B — must be rejected.
        let rev_c = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        let result = svc.revise(budget.id(), rev_c).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "duplicate effective_from with a different id should be InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_for_window_zero_day_window_returns_zero_status(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let acc = accounts
            .create()
            .name("Utilities")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");

        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acc)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");

        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let zero_window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 6, 15),
            Date::constant(2026, 6, 15),
            "zero",
        );
        let status = engine
            .status_for_window(&budget, zero_window, None)
            .await
            .expect("zero-day window should not error");
        assert_eq!(status.allocated, Decimal::ZERO);
        assert_eq!(status.actuals, Decimal::ZERO);
        assert_eq!(status.available, Decimal::ZERO);
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod elided_actuals_tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::Budget;
    use bc_models::CommodityCode;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use bc_models::TagId;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::BudgetService;
    use super::BudgetStatusEngine;
    use super::PostingRow;
    use super::expand_posting_rows;
    use crate::BcError;
    use crate::account::Service as AccountService;
    use crate::fx::noop_fx;
    use crate::residual::Residuals;

    /// One leg of a fixture transaction: posting id, account, and `None` for an elided amount.
    type Leg<'a> = (&'a str, &'a AccountId, Option<(&'a str, &'a str)>);

    /// Creates an account, optionally under `parent`.
    async fn account(
        pool: &SqlitePool,
        name: &str,
        account_type: AccountType,
        parent: Option<&AccountId>,
    ) -> AccountId {
        AccountService::new(pool.clone())
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .maybe_parent_id(parent)
            .call()
            .await
            .expect("create account")
    }

    /// A monthly 200 AUD budget on `account` from 2026-01-01, optionally tag-filtered.
    async fn monthly_budget(
        pool: &SqlitePool,
        account: &AccountId,
        tag_filter: Option<&TagId>,
    ) -> Budget {
        budget_with_target(
            pool,
            account,
            tag_filter,
            Some(Amount::new(dec!(200), CommodityCode::new("AUD"))),
        )
        .await
    }

    /// Creates a monthly budget on `account`; `None` for `target` makes it tracking-only.
    async fn budget_with_target(
        pool: &SqlitePool,
        account: &AccountId,
        tag_filter: Option<&TagId>,
        target: Option<Amount>,
    ) -> Budget {
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(account.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .maybe_target(target)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .maybe_tag_filter(tag_filter.cloned())
            .call()
            .await
            .expect("create budget");
        budget
    }

    /// A daily 10 AUD budget on `account` from 2026-01-01, under `rollover`.
    async fn daily_carry_budget(
        pool: &SqlitePool,
        account: &AccountId,
        rollover: RolloverPolicy,
    ) -> Budget {
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(account.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(10), CommodityCode::new("AUD")))
            .period(Period::Daily)
            .rollover(rollover)
            .call()
            .await
            .expect("create budget");
        budget
    }

    /// Inserts a transaction and its legs by raw SQL, so an ambiguous
    /// two-elided-leg transaction can be staged without the service's guard.
    async fn insert_tx(pool: &SqlitePool, tx_id: &str, date: &str, legs: &[Leg<'_>]) {
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, 'fixture', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .bind(tx_id)
        .bind(date)
        .execute(pool)
        .await
        .expect("insert transaction");
        for (position, (posting_id, account_id, amount)) in legs.iter().enumerate() {
            let (amt, comm) = amount.map_or((None, None), |(a, c)| (Some(a), Some(c)));
            sqlx::query(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(*posting_id)
            .bind(tx_id)
            .bind(account_id.to_string())
            .bind(amt)
            .bind(comm)
            .bind(i64::try_from(position).expect("position fits i64"))
            .execute(pool)
            .await
            .expect("insert posting");
        }
    }

    /// Status of `budget` for March 2026 under `noop_fx`, with no user query.
    async fn march_actuals(pool: &SqlitePool, budget: &Budget) -> super::BudgetStatus {
        BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for(budget, Date::constant(2026, 3, 15))
            .await
            .expect("status")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_on_the_budgeted_account_counts_its_residual(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_e1",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-50.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(50.00));
        assert_eq!(status.available, dec!(150.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_on_a_child_account_counts_toward_the_parent_budget(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let cafes = account(&pool, "Cafes", AccountType::Expense, Some(&food)).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_c1",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-4.50", "AUD"))),
                ("p_cafes", &cafes, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(4.50));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_elided_legs_contribute_nothing_without_error(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let fun = account(&pool, "Fun", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_amb",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-50.00", "AUD"))),
                ("p_food", &food, None),
                ("p_fun", &fun, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concrete_and_elided_legs_sum_together(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_concrete",
            "2026-03-02",
            &[
                ("p_food_a", &food, Some(("20.00", "AUD"))),
                ("p_bank_a", &bank, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_elided",
            "2026-03-20",
            &[
                ("p_bank_b", &bank, Some(("-30.00", "AUD"))),
                ("p_food_b", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(50.00));
    }

    /// Under `noop_fx` the USD component has no rate; PR 3 (#504) surfaces it.
    #[sqlx::test(migrations = "./migrations")]
    async fn multi_commodity_residual_counts_the_target_component(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let wallet = account(&pool, "Wallet", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_mc",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-40.00", "AUD"))),
                ("p_wallet", &wallet, Some(("-10.00", "USD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(40.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tracking_only_budget_reports_the_dominant_residual_component(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let wallet = account(&pool, "Wallet", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = budget_with_target(&pool, &food, None, None).await;
        insert_tx(
            &pool,
            "tx_track",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-40.00", "AUD"))),
                ("p_wallet", &wallet, Some(("-95.00", "USD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(95.00));
        assert_eq!(status.commodity, Some(CommodityCode::new("USD")));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_filter_narrows_rows_inside_the_residual_scope(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let cafes = account(&pool, "Cafes", AccountType::Expense, Some(&food)).await;
        let groceries = account(&pool, "Groceries", AccountType::Expense, Some(&food)).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_cafe",
            "2026-03-05",
            &[
                ("p_bank_c", &bank, Some(("-7.00", "AUD"))),
                ("p_cafes", &cafes, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_groc",
            "2026-03-06",
            &[
                ("p_bank_g", &bank, Some(("-60.00", "AUD"))),
                ("p_groceries", &groceries, None),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery {
            accounts: vec![cafes.clone()],
            ..Default::default()
        };
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
            "March",
        );

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, Some(&query))
            .await
            .expect("status");

        assert_eq!(status.actuals, dec!(7.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_outside_the_period_is_not_counted(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_feb",
            "2026-02-28",
            &[
                ("p_bank", &bank, Some(("-50.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(0));
    }

    /// Inserts a tag row.
    async fn insert_tag(pool: &SqlitePool, tag: &TagId, name: &str) {
        sqlx::query(
            "INSERT INTO tags (id, name, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(tag.to_string())
        .bind(name)
        .execute(pool)
        .await
        .expect("insert tag");
    }

    /// A raw actuals row dated 2026-03-05, concrete when `amount` is `Some`.
    fn row(id: &str, amount: Option<(&str, &str)>) -> PostingRow {
        let (value, commodity) = amount.map_or((None, None), |(a, c)| (Some(a), Some(c)));
        PostingRow {
            id: id.into(),
            date: "2026-03-05".into(),
            amount: value.map(Into::into),
            commodity: commodity.map(Into::into),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_tag_admits_an_elided_leg(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let organic = TagId::new();
        insert_tag(&pool, &organic, "organic").await;
        let budget = monthly_budget(&pool, &food, Some(&organic)).await;
        insert_tx(
            &pool,
            "tx_tag",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-25.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;
        sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES ('tx_tag', ?)")
            .bind(organic.to_string())
            .execute(&pool)
            .await
            .expect("tag transaction");

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(25.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sibling_posting_tag_does_not_admit_an_elided_leg(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let organic = TagId::new();
        insert_tag(&pool, &organic, "organic").await;
        let budget = monthly_budget(&pool, &food, Some(&organic)).await;
        insert_tx(
            &pool,
            "tx_sib",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-25.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES ('p_bank', ?)")
            .bind(organic.to_string())
            .execute(&pool)
            .await
            .expect("tag sibling posting");

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amount_filter_matches_the_derived_amount(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_small",
            "2026-03-05",
            &[
                ("p_bank_s", &bank, Some(("-5.00", "AUD"))),
                ("p_food_s", &food, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_large",
            "2026-03-06",
            &[
                ("p_bank_l", &bank, Some(("-80.00", "AUD"))),
                ("p_food_l", &food, None),
            ],
        )
        .await;
        let amount = crate::search::AmountQuery {
            min: Some(dec!(50)),
            ..Default::default()
        };
        let query = crate::search::TransactionQuery {
            amount: Some(amount),
            ..Default::default()
        };
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
            "March",
        );

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, Some(&query))
            .await
            .expect("status");

        assert_eq!(status.actuals, dec!(80.00));
    }

    #[test]
    fn half_null_row_is_bad_data() {
        let amount_without_commodity = expand_posting_rows(
            vec![PostingRow {
                id: "p1".into(),
                date: "2026-03-05".into(),
                amount: Some("1.00".into()),
                commodity: None,
            }],
            None,
        );
        let err = amount_without_commodity.expect_err("half-null row must be rejected");
        assert!(matches!(err, BcError::BadData(_)), "{err:?}");

        let commodity_without_amount = expand_posting_rows(
            vec![PostingRow {
                id: "p1".into(),
                date: "2026-03-05".into(),
                amount: None,
                commodity: Some("AUD".into()),
            }],
            None,
        );
        let other_err = commodity_without_amount.expect_err("half-null row must be rejected");
        assert!(matches!(other_err, BcError::BadData(_)), "{other_err:?}");
    }

    #[test]
    fn elided_row_without_loaded_residuals_is_bad_data() {
        let result = expand_posting_rows(vec![row("p1", None)], None);
        let err = result.expect_err("elided row with no residual load must be rejected");
        assert!(matches!(err, BcError::BadData(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_row_outside_residual_scope_is_bad_data(pool: SqlitePool) {
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let residuals = Residuals::for_subtree_in_range(
            &pool,
            &food,
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
        )
        .await
        .expect("load residuals");

        let result = expand_posting_rows(vec![row("p1", None)], Some(&residuals));

        let err = result.expect_err("posting outside the loaded scope must be rejected");
        assert!(matches!(err, BcError::BadData(_)), "{err:?}");
    }

    #[test]
    fn dated_rows_parse_their_dates() {
        let amounts = expand_posting_rows(vec![row("p1", Some(("12.50", "AUD")))], None)
            .expect("concrete row expands");
        assert_eq!(
            amounts,
            vec![(
                Date::constant(2026, 3, 5),
                Amount::new(dec!(12.50), CommodityCode::new("AUD")),
            )]
        );
    }

    #[test]
    fn unparsable_date_is_bad_data() {
        let result = expand_posting_rows(
            vec![PostingRow {
                id: "p1".into(),
                date: "not-a-date".into(),
                amount: Some("1.00".into()),
                commodity: Some("AUD".into()),
            }],
            None,
        );
        let err = result.expect_err("bad date must be rejected");
        assert!(matches!(err, BcError::BadData(_)), "{err:?}");
    }

    /// 304 daily periods precede 1 November; all carry. One elided leg of
    /// 7.00 on 5 January is the only spend, so the rollover into November 1
    /// is 304 × 10 − 7.
    #[sqlx::test(migrations = "./migrations")]
    async fn daily_budget_carries_an_elided_leg_from_three_hundred_days_back(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = daily_carry_budget(&pool, &food, RolloverPolicy::CarryForward).await;
        insert_tx(
            &pool,
            "tx_jan",
            "2026-01-05",
            &[
                ("p_bank", &bank, Some(("-7.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for(&budget, Date::constant(2026, 11, 1))
            .await
            .expect("status");

        assert_eq!(status.allocated, dec!(10.00));
        assert_eq!(status.actuals, dec!(0));
        assert_eq!(status.rollover, dec!(3033.00));
        assert_eq!(status.available, dec!(3043.00));
    }

    /// The user's account filter narrows the window's actuals but never the
    /// carry chain: rollover is a property of the budget, not of the view.
    #[sqlx::test(migrations = "./migrations")]
    async fn account_filter_narrows_window_actuals_but_not_rollover(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let cafe = account(&pool, "Cafe", AccountType::Expense, Some(&food)).await;
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(200), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create budget");
        // February: 30 on Food, 20 on Cafe → surplus 150; January untouched → 200.
        insert_tx(
            &pool,
            "tx_feb_food",
            "2026-02-10",
            &[
                ("p_b1", &bank, Some(("-30.00", "AUD"))),
                ("p_f1", &food, Some(("30.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_feb_cafe",
            "2026-02-11",
            &[
                ("p_b2", &bank, Some(("-20.00", "AUD"))),
                ("p_c2", &cafe, Some(("20.00", "AUD"))),
            ],
        )
        .await;
        // March: 40 on Food, 15 on Cafe.
        insert_tx(
            &pool,
            "tx_mar_food",
            "2026-03-03",
            &[
                ("p_b3", &bank, Some(("-40.00", "AUD"))),
                ("p_f3", &food, Some(("40.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_mar_cafe",
            "2026-03-04",
            &[
                ("p_b4", &bank, Some(("-15.00", "AUD"))),
                ("p_c4", &cafe, Some(("15.00", "AUD"))),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery {
            accounts: vec![cafe.clone()],
            ..Default::default()
        };
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
            "March",
        );

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, Some(&query))
            .await
            .expect("status");

        assert_eq!(status.actuals, dec!(15.00));
        assert_eq!(status.rollover, dec!(350.00));
        assert_eq!(status.available, dec!(535.00));
    }

    /// Tags a transaction.
    async fn tag_tx(pool: &SqlitePool, tx_id: &str, tag: &TagId) {
        sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
            .bind(tx_id)
            .bind(tag.to_string())
            .execute(pool)
            .await
            .expect("tag transaction");
    }

    /// A carry chain spanning two reigns with different tag filters, read
    /// through a window with a user query: the chain loads under each reign's
    /// own tag filter and never the query, while the window loads under both.
    #[sqlx::test(migrations = "./migrations")]
    async fn chain_across_tag_filtered_reigns_with_a_window_query(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let cafe = account(&pool, "Cafe", AccountType::Expense, Some(&food)).await;
        let home = TagId::new();
        let work = TagId::new();
        insert_tag(&pool, &home, "home").await;
        insert_tag(&pool, &work, "work").await;
        let svc = BudgetService::new(pool.clone());
        let target = || Amount::new(dec!(100), CommodityCode::new("AUD"));
        // January is governed by a `home`-filtered reign, February onward by a
        // `work`-filtered one; both carry.
        let (budget, _) = svc
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(target())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .tag_filter(home.clone())
            .call()
            .await
            .expect("create budget");
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2026, 2, 1))
                .target(target())
                .period(Period::Monthly)
                .rollover(RolloverPolicy::CarryForward)
                .tag_filter(work.clone())
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");
        // (tx, date, account, amount, tag). January: only `home` counts → 30,
        // surplus 70. February: only `work` counts → 40, surplus 100 + 70 − 40
        // = 130. March under the Cafe query: only the `work`-tagged Cafe
        // posting counts → 25.
        let fixtures: [(&str, &str, &AccountId, &str, &TagId); 6] = [
            ("tx_jan_home", "2026-01-10", &food, "30.00", &home),
            ("tx_jan_work", "2026-01-11", &food, "20.00", &work),
            ("tx_feb_work", "2026-02-10", &food, "40.00", &work),
            ("tx_feb_home", "2026-02-11", &food, "10.00", &home),
            ("tx_mar_cafe_work", "2026-03-03", &cafe, "25.00", &work),
            ("tx_mar_food_work", "2026-03-04", &food, "5.00", &work),
        ];
        for (tx_id, date, acct, amount, tag) in fixtures {
            let negated = format!("-{amount}");
            let p_bank = format!("{tx_id}_bank");
            let p_spend = format!("{tx_id}_spend");
            insert_tx(
                &pool,
                tx_id,
                date,
                &[
                    (&p_bank, &bank, Some((&negated, "AUD"))),
                    (&p_spend, acct, Some((amount, "AUD"))),
                ],
            )
            .await;
            tag_tx(&pool, tx_id, tag).await;
        }
        let query = crate::search::TransactionQuery {
            accounts: vec![cafe.clone()],
            ..Default::default()
        };
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
            "March",
        );

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, Some(&query))
            .await
            .expect("status");

        assert_eq!(status.actuals, dec!(25.00));
        assert_eq!(status.rollover, dec!(130.00));
        assert_eq!(status.available, dec!(205.00));
    }

    /// An uncapped carry would reach 30.00 after three empty days; `CapAtTarget`
    /// clamps the carry to the 10.00 target at every destination, so it stays
    /// at 10.00 rather than accumulating.
    #[sqlx::test(migrations = "./migrations")]
    async fn cap_at_target_caps_the_carry_at_every_step(pool: SqlitePool) {
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = daily_carry_budget(&pool, &food, RolloverPolicy::CapAtTarget).await;

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for(&budget, Date::constant(2026, 1, 4))
            .await
            .expect("status");

        assert_eq!(status.allocated, dec!(10.00));
        assert_eq!(status.actuals, dec!(0));
        assert_eq!(status.rollover, dec!(10.00));
        assert_eq!(status.available, dec!(20.00));
    }

    /// A posting dated before the window's clipped-off period head is dropped
    /// by `bucket_segments`: it lands in neither the chain (which ends at the
    /// window period's true start) nor the window segment (clipped to the
    /// window itself), so it must not surface in either actuals or rollover.
    #[sqlx::test(migrations = "./migrations")]
    async fn posting_in_clipped_off_period_head_is_not_counted(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(200), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create budget");
        insert_tx(
            &pool,
            "tx_feb_head",
            "2026-02-10",
            &[
                ("p_bank", &bank, Some(("-50.00", "AUD"))),
                ("p_food", &food, None),
            ],
        )
        .await;
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 2, 26),
            Date::constant(2026, 3, 1),
            "tail of February",
        );

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, None)
            .await
            .expect("status");

        // January is a full chain period (target 200, no spend), so it carries
        // whole into February; the February posting predates the window and
        // falls in the clipped-off head of February's own period, so it
        // reaches neither the window's actuals nor the chain's rollover.
        assert_eq!(status.actuals, dec!(0));
        assert_eq!(status.rollover, dec!(200.00));
        // 3 of 28 days of February's 200.00 target: 200 * 3 / 28 = 21.428571...
        assert_eq!(status.allocated, dec!(21.43));
        assert_eq!(status.available, dec!(221.43));
    }

    /// A posting in a commodity `noop_fx` cannot convert to the target is
    /// excluded from `actuals` and reported in `unvalued`.
    #[sqlx::test(migrations = "./migrations")]
    async fn foreign_posting_under_a_target_commodity_is_unvalued(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_usd",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-10.00", "USD"))),
                ("p_food", &food, Some(("10.00", "USD"))),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(0));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(10.00)));
        assert_eq!(status.unvalued.len(), 1);
    }

    /// An unvalued posting in a carry-chain period surfaces on the status
    /// whose rollover it fed, with the rollover itself unaffected.
    #[sqlx::test(migrations = "./migrations")]
    async fn unvalued_posting_in_the_chain_surfaces_on_the_status(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = daily_carry_budget(&pool, &food, RolloverPolicy::CarryForward).await;
        insert_tx(
            &pool,
            "tx_usd",
            "2026-01-05",
            &[
                ("p_bank", &bank, Some(("-7.00", "USD"))),
                ("p_food", &food, Some(("7.00", "USD"))),
            ],
        )
        .await;

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for(&budget, Date::constant(2026, 1, 10))
            .await
            .expect("status");

        // Nine empty chain days at 10.00 each; the USD leg counts in none.
        assert_eq!(status.rollover, dec!(90.00));
        assert_eq!(status.actuals, dec!(0));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(7.00)));
    }

    /// A tracking-only budget reports the dominant commodity as `actuals`
    /// and every other commodity group as `unvalued`.
    #[sqlx::test(migrations = "./migrations")]
    async fn tracking_only_reports_non_dominant_groups_as_unvalued(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create budget");
        insert_tx(
            &pool,
            "tx_aud",
            "2026-03-05",
            &[
                ("p_bank_a", &bank, Some(("-40.00", "AUD"))),
                ("p_food_a", &food, Some(("40.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_usd",
            "2026-03-06",
            &[
                ("p_bank_u", &bank, Some(("-10.00", "USD"))),
                ("p_food_u", &food, Some(("10.00", "USD"))),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(40.00));
        assert_eq!(status.commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(10.00)));
        assert_eq!(status.unvalued.len(), 1);
    }

    /// An elided leg whose residual spans two commodities lands the target
    /// component in `actuals` and the other in `unvalued`.
    #[sqlx::test(migrations = "./migrations")]
    async fn multi_commodity_residual_reports_the_foreign_component_as_unvalued(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let wallet = account(&pool, "Wallet", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_mc",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-40.00", "AUD"))),
                ("p_wallet", &wallet, Some(("-10.00", "USD"))),
                ("p_food", &food, None),
            ],
        )
        .await;

        let status = march_actuals(&pool, &budget).await;

        assert_eq!(status.actuals, dec!(40.00));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(10.00)));
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod load_plan_tests {
    use bc_models::BudgetId;
    use bc_models::BudgetRevision;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use bc_models::TagId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::Load;
    use super::Segment;
    use super::plan_loads;
    use super::segment_index;

    /// A daily carry-forward revision effective from `eff`, optionally tag-filtered.
    fn daily_rev(eff: Date, tag: Option<&TagId>) -> BudgetRevision {
        BudgetRevision::builder()
            .budget_id(BudgetId::new())
            .effective_from(eff)
            .period(Period::Daily)
            .rollover(RolloverPolicy::CarryForward)
            .maybe_tag_filter(tag.cloned())
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn no_query_merges_chain_and_window_into_one_load_per_reign() {
        // 400 daily periods across two reigns: one load per reign, never per day.
        let tag = TagId::new();
        let revs = vec![
            daily_rev(Date::constant(2026, 1, 1), None),
            daily_rev(Date::constant(2026, 7, 1), Some(&tag)),
        ];
        let loads = plan_loads(
            &revs,
            Some((Date::constant(2026, 1, 1), Date::constant(2027, 2, 4))),
            (Date::constant(2027, 2, 4), Date::constant(2027, 2, 5)),
            false,
        );
        assert_eq!(
            loads,
            vec![
                Load {
                    start: Date::constant(2026, 1, 1),
                    end: Date::constant(2026, 7, 1),
                    tag_filter: None,
                    with_query: false,
                },
                Load {
                    start: Date::constant(2026, 7, 1),
                    end: Date::constant(2027, 2, 5),
                    tag_filter: Some(&tag),
                    with_query: false,
                },
            ]
        );
    }

    #[test]
    fn query_splits_the_reign_spanning_the_window_start() {
        // Reign 1 holds the whole chain and the first half of the window, so
        // it needs an unfiltered chain load and a filtered window load; reign 2
        // only overlaps the window.
        let revs = vec![
            daily_rev(Date::constant(2026, 1, 1), None),
            daily_rev(Date::constant(2026, 7, 1), None),
        ];
        let loads = plan_loads(
            &revs,
            Some((Date::constant(2026, 1, 1), Date::constant(2026, 6, 15))),
            (Date::constant(2026, 6, 15), Date::constant(2026, 7, 15)),
            true,
        );
        assert_eq!(
            loads,
            vec![
                Load {
                    start: Date::constant(2026, 1, 1),
                    end: Date::constant(2026, 6, 15),
                    tag_filter: None,
                    with_query: false,
                },
                Load {
                    start: Date::constant(2026, 6, 15),
                    end: Date::constant(2026, 7, 1),
                    tag_filter: None,
                    with_query: true,
                },
                Load {
                    start: Date::constant(2026, 7, 1),
                    end: Date::constant(2026, 7, 15),
                    tag_filter: None,
                    with_query: true,
                },
            ]
        );
    }

    #[test]
    fn no_chain_plans_the_window_only() {
        let revs = vec![daily_rev(Date::constant(2026, 1, 1), None)];
        let loads = plan_loads(
            &revs,
            None,
            (Date::constant(2026, 3, 1), Date::constant(2026, 4, 1)),
            true,
        );
        assert_eq!(
            loads,
            vec![Load {
                start: Date::constant(2026, 3, 1),
                end: Date::constant(2026, 4, 1),
                tag_filter: None,
                with_query: true,
            }]
        );
    }

    #[test]
    fn reigns_outside_both_ranges_plan_nothing() {
        // Reign 2 starts after the window ends; reign 1 covers everything.
        let revs = vec![
            daily_rev(Date::constant(2026, 1, 1), None),
            daily_rev(Date::constant(2027, 1, 1), None),
        ];
        let loads = plan_loads(
            &revs,
            None,
            (Date::constant(2026, 3, 1), Date::constant(2026, 4, 1)),
            false,
        );
        assert_eq!(loads.len(), 1);
    }

    #[test]
    fn empty_window_plans_nothing() {
        let revs = vec![daily_rev(Date::constant(2026, 1, 1), None)];
        let loads = plan_loads(
            &revs,
            None,
            (Date::constant(2026, 3, 1), Date::constant(2026, 3, 1)),
            false,
        );
        assert_eq!(loads, Vec::<Load<'_>>::new());
    }

    #[test]
    fn segment_index_finds_the_half_open_range_holding_a_date() {
        let segments = [
            Segment {
                start: Date::constant(2026, 1, 1),
                end: Date::constant(2026, 2, 1),
            },
            Segment {
                start: Date::constant(2026, 3, 1),
                end: Date::constant(2026, 3, 15),
            },
        ];
        assert_eq!(
            segment_index(&segments, Date::constant(2026, 1, 1)),
            Some(0)
        );
        assert_eq!(
            segment_index(&segments, Date::constant(2026, 1, 31)),
            Some(0)
        );
        assert_eq!(segment_index(&segments, Date::constant(2026, 2, 1)), None);
        assert_eq!(segment_index(&segments, Date::constant(2026, 2, 15)), None);
        assert_eq!(
            segment_index(&segments, Date::constant(2026, 3, 14)),
            Some(1)
        );
        assert_eq!(segment_index(&segments, Date::constant(2026, 3, 15)), None);
        assert_eq!(segment_index(&[], Date::constant(2026, 3, 1)), None);
    }
}
