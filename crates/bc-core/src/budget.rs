//! Budget calculation engine: actuals, rollover, and budget status.

use std::collections::HashMap;
use std::collections::HashSet;
use std::pin::pin;

use futures_util::TryStreamExt as _;
use sqlx::SqlitePool;

use crate::legs::LegFilter;
use crate::legs::LegScope;
use crate::legs::resolved_transactions;

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
    /// Source expression behind the target; NULL for a literal.
    target_expr: Option<String>,
    /// JSON-serialised [`bc_models::Period`].
    period: String,
    /// Snake-case rollover policy string.
    rollover: String,
    /// Snake-case budget intent string.
    intent: String,
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
        let intent = crate::db::from_db_str::<bc_models::BudgetIntent>(&row.intent)?;
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
            .maybe_target_expr(row.target_expr)
            .period(period)
            .rollover(rollover)
            .intent(intent)
            .maybe_tag_filter(tag_filter)
            .created_at(created_at)
            .build())
    }
}

/// Re-evaluates `target_expr` and puts the result in the target's value.
///
/// Core is the only authority on an expression's value, so whatever value the
/// caller supplied beside an expression is replaced.
fn resolve_target(
    target: Option<bc_models::Amount>,
    target_expr: Option<&str>,
) -> crate::BcResult<Option<bc_models::Amount>> {
    let Some(expr) = target_expr else {
        return Ok(target);
    };
    let Some(supplied) = target else {
        return Err(crate::BcError::InvalidInput(
            "a target expression needs a target commodity".to_owned(),
        ));
    };
    let value = bc_expr::evaluate(expr)
        .map_err(|e| crate::BcError::InvalidInput(format!("target expression '{expr}': {e}")))?;
    Ok(Some(bc_models::Amount::new(
        value,
        supplied.commodity().clone(),
    )))
}

/// Revisions whose target sign differs from an adjacent revision's.
///
/// Returns each flagged revision with the date of the neighbour it disagrees
/// with, checking the previous revision before the next. Zero and
/// tracking-only targets carry no sign and never flag.
///
/// # Arguments
///
/// * `revisions` - One budget's revisions, ordered by `effective_from`.
///
/// # Returns
///
/// The flagged revisions' IDs, each with its disagreeing neighbour's
/// effective-from date, in input order.
#[must_use]
#[inline]
pub fn sign_flips(
    revisions: &[bc_models::BudgetRevision],
) -> Vec<(bc_models::BudgetRevisionId, jiff::civil::Date)> {
    let sign = |r: &bc_models::BudgetRevision| {
        r.target()
            .map(bc_models::Amount::value)
            .filter(|v| !v.is_zero())
            .map(|v| v.is_sign_negative())
    };
    let mut out = Vec::new();
    for (i, rev) in revisions.iter().enumerate() {
        let Some(own) = sign(rev) else { continue };
        let prev = i.checked_sub(1).and_then(|j| revisions.get(j));
        let next = revisions.get(i.saturating_add(1));
        if let Some(n) = [prev, next]
            .into_iter()
            .flatten()
            .find(|n| sign(n).is_some_and(|s| s != own))
        {
            out.push((rev.id().clone(), n.effective_from()));
        }
    }
    out
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
    /// When `target_expr` is set, the target's value is replaced by the
    /// expression's result; `target` then supplies only the commodity.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `rollover` is `CapAtTarget` and
    /// `target` is `None`, or if `target_expr` is set without a `target` or
    /// fails to evaluate.
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
        #[builder(into)] target_expr: Option<String>,
        intent: bc_models::BudgetIntent,
        period: bc_models::Period,
        rollover: bc_models::RolloverPolicy,
    ) -> crate::BcResult<crate::Warned<(bc_models::Budget, bc_models::BudgetRevision)>> {
        let resolved = resolve_target(target, target_expr.as_deref())?;
        if rollover == bc_models::RolloverPolicy::CapAtTarget && resolved.is_none() {
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
            target: resolved.clone(),
            target_expr: target_expr.clone(),
            intent,
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
        let (t_amt, t_cur) = resolved.as_ref().map_or((None, None), |a| {
            (Some(a.value().to_string()), Some(a.commodity().to_string()))
        });
        sqlx::query(
            "INSERT INTO budget_revisions \
             (id, budget_id, effective_from, name, target_amount, target_currency, \
              target_expr, period, rollover, intent, tag_filter, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(revision_id.to_string())
        .bind(budget_id.to_string())
        .bind(effective_from.to_string())
        .bind(&name)
        .bind(&t_amt)
        .bind(&t_cur)
        .bind(&target_expr)
        .bind(&period_json)
        .bind(&rollover_db)
        .bind(crate::db::to_db_str(intent)?)
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
            .maybe_target(resolved)
            .maybe_target_expr(target_expr)
            .period(period)
            .rollover(rollover)
            .intent(intent)
            .maybe_tag_filter(tag_filter)
            .created_at(now)
            .build();
        // A lone revision has no neighbour to flip against.
        Ok(crate::Warned::clean((budget, revision)))
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
              target_expr, period, rollover, intent, tag_filter, created_at \
             FROM budget_revisions \
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
    /// When the revision carries a target expression, the stored target's value
    /// is the expression's result. The returned revision warns with
    /// [`crate::Warning::BudgetSignFlip`] when its target sign differs from an
    /// adjacent revision's.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `CapAtTarget` rollover has no target.
    /// Returns [`crate::BcError::InvalidInput`] if the target expression has no
    /// target or fails to evaluate.
    /// Returns [`crate::BcError::InvalidInput`] if a different revision already occupies the
    /// same `effective_from` date (amending a revision in place — same id — is always allowed).
    /// Returns [`crate::BcError::NotFound`] if the budget is missing or archived.
    /// Returns [`crate::BcError`] on event or database failure.
    #[inline]
    pub async fn revise(
        &self,
        budget_id: &bc_models::BudgetId,
        requested: bc_models::BudgetRevision,
    ) -> crate::BcResult<crate::Warned<bc_models::BudgetRevision>> {
        drop(self.get(budget_id).await?);
        let target = resolve_target(requested.target().cloned(), requested.target_expr())?;
        let revision = bc_models::BudgetRevision::builder()
            .id(requested.id().clone())
            .budget_id(requested.budget_id().clone())
            .effective_from(requested.effective_from())
            .maybe_name(requested.name())
            .maybe_target(target)
            .maybe_target_expr(requested.target_expr())
            .period(requested.period().clone())
            .rollover(requested.rollover())
            .intent(requested.intent())
            .maybe_tag_filter(requested.tag_filter().cloned())
            .created_at(*requested.created_at())
            .build();
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
            target_expr: revision.target_expr().map(str::to_owned),
            intent: revision.intent(),
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
              target_expr, period, rollover, intent, tag_filter, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               effective_from = excluded.effective_from, name = excluded.name, \
               target_amount = excluded.target_amount, target_currency = excluded.target_currency, \
               target_expr = excluded.target_expr, \
               period = excluded.period, rollover = excluded.rollover, \
               intent = excluded.intent, tag_filter = excluded.tag_filter",
        )
        .bind(revision.id().to_string())
        .bind(budget_id.to_string())
        .bind(revision.effective_from().to_string())
        .bind(revision.name())
        .bind(&t_amt)
        .bind(&t_cur)
        .bind(revision.target_expr())
        .bind(&period_json)
        .bind(crate::db::to_db_str(revision.rollover())?)
        .bind(crate::db::to_db_str(revision.intent())?)
        .bind(revision.tag_filter().map(ToString::to_string))
        .bind(revision.created_at().to_string())
        .execute(&mut *db_tx)
        .await?;
        db_tx.commit().await?;
        tracing::info!(%budget_id, revision_id = %revision.id(), "budget revised");
        let after = self.revisions(budget_id).await?;
        let warnings = sign_flips(&after)
            .into_iter()
            .filter(|(id, _)| id == revision.id())
            .map(|(_, neighbour)| crate::Warning::BudgetSignFlip {
                budget_id: budget_id.clone(),
                effective_from: revision.effective_from(),
                neighbour_effective_from: neighbour,
            })
            .collect();
        Ok(crate::Warned::new(revision, warnings))
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
    /// rate could value in the target commodity, tracking-only postings
    /// outside the window's [`Self::commodity`], and each carry-chain
    /// tracking-only period's non-dominant commodity groups. `available` is
    /// exact only when this is empty.
    pub unvalued: bc_models::Balances,
}

/// Identifies one concrete amount a posting contributes.
///
/// A concrete posting has one key. An elided leg has one key per commodity
/// component of its transaction's residual, so each component can be claimed
/// and valued on its own.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub struct PostingKey {
    /// Raw ID of the posting.
    pub posting_id: String,
    /// Commodity of the amount this key stands for.
    pub commodity: String,
}

/// One posting a budget matched in a window, valued under the revision
/// governing the period it falls in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ValuedPosting {
    /// The posting (or elided-leg component) this value is for.
    pub key: PostingKey,
    /// The account the posting is on.
    pub account_id: bc_models::AccountId,
    /// The transaction date.
    pub date: jiff::civil::Date,
    /// The amount in the budget's commodity, or `None` when it could not be
    /// valued: no FX rate reaches the target commodity, or, under a
    /// tracking-only revision, the amount is outside the window's dominant
    /// commodity. An unvalued amount shows in [`BudgetStatus::unvalued`].
    pub value: Option<bc_models::Decimal>,
    /// The native amount, before valuation.
    pub amount: bc_models::Amount,
}

/// Every posting a budget matched in a window, valued under one commodity
/// decision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WindowValuation {
    /// The valued postings, in no particular order.
    pub postings: Vec<ValuedPosting>,
    /// The window's commodity: the target commodity of the first period (in
    /// order) that has a target, else the dominant commodity over the
    /// tracking-only postings. Tracking-only periods value against it, so a
    /// posting in another commodity is unvalued. A targeted period values into
    /// its own target's commodity, which differs from this one only when the
    /// target commodity changes inside the window. `None` only when no period
    /// has a target and no posting exists.
    pub commodity: Option<bc_models::CommodityCode>,
    /// Native amounts counted in no total, by commodity.
    pub unvalued: bc_models::Balances,
}

/// One concrete amount from the actuals query: its key, account, transaction
/// date and native amount.
type ExpandedPosting = (
    PostingKey,
    bc_models::AccountId,
    jiff::civil::Date,
    bc_models::Amount,
);

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
pub(crate) fn add_unvalued(
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

/// Rejects a window whose end precedes its start.
///
/// # Errors
///
/// Returns [`crate::BcError::InvalidInput`] if `window.end < window.start`.
fn check_window(window: &bc_models::BudgetWindow) -> crate::BcResult<()> {
    if window.days() < 0 {
        return Err(crate::BcError::InvalidInput(format!(
            "BudgetWindow has end before start: {} to {}",
            window.start, window.end
        )));
    }
    Ok(())
}

/// Sums `amounts` per commodity, keyed by commodity code.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] on decimal overflow.
fn commodity_groups<'a>(
    amounts: impl IntoIterator<Item = &'a bc_models::Amount>,
) -> crate::BcResult<std::collections::BTreeMap<String, bc_models::Decimal>> {
    let mut groups: std::collections::BTreeMap<String, bc_models::Decimal> =
        std::collections::BTreeMap::new();
    for amount in amounts {
        let entry = groups
            .entry(amount.commodity().to_string())
            .or_insert(bc_models::Decimal::ZERO);
        *entry = entry
            .checked_add(amount.value())
            .ok_or_else(|| crate::BcError::BadData("actuals sum overflow".into()))?;
    }
    Ok(groups)
}

/// The commodity whose group total is largest in absolute value, or `None`
/// when `groups` is empty.
///
/// A tie in absolute value resolves to the last (highest) commodity code,
/// since `max_by` keeps the later of two equal elements and
/// `BTreeMap::iter` yields entries in code order.
fn dominant_commodity(
    groups: &std::collections::BTreeMap<String, bc_models::Decimal>,
) -> Option<bc_models::CommodityCode> {
    groups
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.abs()
                .partial_cmp(&b.abs())
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .map(|(code, _)| bc_models::CommodityCode::new(code.clone()))
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
        check_window(&window)?;
        let svc = BudgetService::new(self.pool.clone());
        let revisions = svc.revisions(budget.id()).await?;
        let account_id = budget.account_id().clone();
        let keys = self
            .query_keys(&account_id, window.start, window.end, query)
            .await?;

        let periods = bc_models::periods_overlapping(&revisions, window.start, window.end);
        let first_start = periods.first().map(|p| p.start);
        let chain_start = first_start.and_then(|fs| bc_models::carry_chain_start(&revisions, fs));
        let chain_periods = match (chain_start, first_start) {
            (Some(cs), Some(fs)) => bc_models::periods_overlapping(&revisions, cs, fs),
            _ => Vec::new(),
        };

        let (segments, chain_len) = bucket_segments(&chain_periods, &periods, &window);
        let mut buckets: Vec<Vec<ExpandedPosting>> = vec![Vec::new(); segments.len()];

        let chain = chain_start.zip(first_start);
        for load in plan_loads(
            &revisions,
            chain,
            (window.start, window.end),
            query.is_some(),
        ) {
            let postings = self
                .fetch_postings(
                    &account_id,
                    load.start,
                    load.end,
                    load.tag_filter,
                    if load.with_query { keys.as_ref() } else { None },
                )
                .await?;
            for posting in postings {
                if let Some(bucket) =
                    segment_index(&segments, posting.2).and_then(|i| buckets.get_mut(i))
                {
                    bucket.push(posting);
                }
            }
        }
        let (chain_buckets, window_buckets) = buckets.split_at(chain_len);

        let valuation = self.value_window(&periods, window_buckets.to_vec())?;
        let actuals = valuation
            .postings
            .iter()
            .filter_map(|p| p.value)
            .try_fold(bc_models::Decimal::ZERO, bc_models::Decimal::checked_add)
            .ok_or_else(|| crate::BcError::BadData("actuals overflow".into()))?;
        let mut unvalued = valuation.unvalued;
        let commodity = valuation.commodity;

        let allocated = prorated_target(&revisions, window.start, window.end)?
            .unwrap_or(bc_models::Decimal::ZERO);

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

    /// Every posting `budget` matches inside `window`, valued as
    /// [`Self::status_for_window`] values its actuals.
    ///
    /// No carry chain is loaded. Every posting is valued against the window's
    /// one commodity (see [`WindowValuation::commodity`]), so the values sum
    /// to the status's `actuals`. An amount that cannot be valued is kept with
    /// `value: None`, so a caller still knows the budget matched it.
    ///
    /// # Arguments
    ///
    /// * `budget` - The budget whose matches to list.
    /// * `window` - The viewing window.
    /// * `query` - Optional user query narrowing the postings, amount filter
    ///   included.
    ///
    /// # Returns
    ///
    /// The valued postings, in no particular order, the commodity of their
    /// values (the same commodity [`BudgetStatus::commodity`] reports for
    /// this window and query) and the native amounts counted in no total.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `window.end < window.start`.
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn window_postings(
        &self,
        budget: &bc_models::Budget,
        window: &bc_models::BudgetWindow,
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<WindowValuation> {
        check_window(window)?;
        let revisions = BudgetService::new(self.pool.clone())
            .revisions(budget.id())
            .await?;
        let keys = self
            .query_keys(budget.account_id(), window.start, window.end, query)
            .await?;

        let periods = bc_models::periods_overlapping(&revisions, window.start, window.end);
        let (segments, _) = bucket_segments(&[], &periods, window);
        let mut buckets: Vec<Vec<ExpandedPosting>> = vec![Vec::new(); segments.len()];
        for load in plan_loads(
            &revisions,
            None,
            (window.start, window.end),
            query.is_some(),
        ) {
            let postings = self
                .fetch_postings(
                    budget.account_id(),
                    load.start,
                    load.end,
                    load.tag_filter,
                    if load.with_query { keys.as_ref() } else { None },
                )
                .await?;
            for posting in postings {
                if let Some(bucket) =
                    segment_index(&segments, posting.2).and_then(|i| buckets.get_mut(i))
                {
                    bucket.push(posting);
                }
            }
        }
        self.value_window(&periods, buckets)
    }

    /// Values every window posting under one commodity decision.
    ///
    /// `buckets` holds each period's postings, already narrowed to the query's
    /// keys. Under a target a posting values into its period's target
    /// commodity. The window commodity is the first target's commodity, else
    /// the tracking-only dominant; tracking-only periods value against it.
    /// Values share one commodity unless two revisions in the window target
    /// different ones.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] on decimal overflow.
    fn value_window(
        &self,
        periods: &[bc_models::ResolvedPeriod<'_>],
        buckets: Vec<Vec<ExpandedPosting>>,
    ) -> crate::BcResult<WindowValuation> {
        let mut commodity = periods
            .iter()
            .find_map(|p| p.revision.target().map(|t| t.commodity().clone()));
        if commodity.is_none() {
            let tracking =
                commodity_groups(buckets.iter().flat_map(|b| b.iter().map(|(_, _, _, a)| a)))?;
            commodity = dominant_commodity(&tracking);
        }
        let mut postings = Vec::new();
        let mut unvalued = bc_models::Balances::new();
        for (p, bucket) in periods.iter().zip(buckets) {
            for (key, account_id, date, amount) in bucket {
                let value = self.value_in(p.revision, commodity.as_ref(), &amount);
                if value.is_none() {
                    add_unvalued(&mut unvalued, &amount)?;
                }
                postings.push(ValuedPosting {
                    key,
                    account_id,
                    date,
                    value,
                    amount,
                });
            }
        }
        Ok(WindowValuation {
            postings,
            commodity,
            unvalued,
        })
    }

    /// Every posting under `root` inside `window` that passes `query`, as
    /// native amounts.
    ///
    /// Elided legs expand to their residual components, and the query admits
    /// each component on its own, exactly as it does for a budget's actuals.
    ///
    /// # Arguments
    ///
    /// * `root` - Root of the account subtree to list.
    /// * `window` - The viewing window.
    /// * `query` - Optional user query narrowing the postings, amount filter
    ///   included.
    ///
    /// # Returns
    ///
    /// One `(key, account, amount)` per concrete amount, in no particular
    /// order.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if `window.end < window.start`.
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn subtree_postings(
        &self,
        root: &bc_models::AccountId,
        window: &bc_models::BudgetWindow,
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<Vec<(PostingKey, bc_models::AccountId, bc_models::Amount)>> {
        check_window(window)?;
        let keys = self
            .query_keys(root, window.start, window.end, query)
            .await?;
        let postings = self
            .fetch_postings(root, window.start, window.end, None, keys.as_ref())
            .await?;
        Ok(postings
            .into_iter()
            .map(|(key, account_id, _, amount)| (key, account_id, amount))
            .collect())
    }

    /// The `(posting, commodity)` keys under `root` dated in `[from, until)`
    /// that `query` admits; `None` without a query or without an expression.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure.
    async fn query_keys(
        &self,
        root: &bc_models::AccountId,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<Option<HashSet<PostingKey>>> {
        let Some(q) = query else {
            return Ok(None);
        };
        let Some(subtree) =
            crate::search::resolve_account_subtrees(&self.pool, core::slice::from_ref(root))
                .await?
        else {
            return Ok(None);
        };
        let scope: Vec<bc_models::AccountId> = subtree.into_iter().collect();
        crate::transaction::Service::new(self.pool.clone())
            .matching_components(q, &scope, from, until)
            .await
    }

    // TODO: apply spread fields to period attribution (planned follow-on)
    /// Fetches every matched posting in `[from, to)` as concrete amounts,
    /// with elided legs resolved to their residuals.
    ///
    /// The leg stream and the posting-id lookup run in one read transaction,
    /// so every rowid the stream yields maps to a posting in the same
    /// snapshot.
    ///
    /// # Arguments
    ///
    /// * `account_id` - Root of the account subtree to fetch postings for.
    /// * `from` - Inclusive start of the load's date range.
    /// * `to` - Exclusive end of the load's date range.
    /// * `tag_filter` - Restricts to postings or transactions tagged within
    ///   this subtree.
    /// * `keys` - When set, only the amounts whose `(posting, commodity)` key
    ///   it holds are kept: the components a query admits, resolved by the
    ///   caller so the snapshot transaction holds the only pool connection.
    ///
    /// # Returns
    ///
    /// One entry per concrete leg, plus one per commodity component of each
    /// attributable elided leg. An ambiguous elided leg contributes nothing.
    /// Because expansion precedes the key filter, that filter sees each
    /// commodity component of an elided leg as its own amount.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure or unparsable stored data.
    #[inline]
    async fn fetch_postings(
        &self,
        account_id: &bc_models::AccountId,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
        tag_filter: Option<&bc_models::TagId>,
        keys: Option<&HashSet<PostingKey>>,
    ) -> crate::BcResult<Vec<ExpandedPosting>> {
        let mut tx = self.pool.begin().await?;
        let filter = LegFilter::new(LegScope::Subtree(account_id))
            .window(from, to)
            .tag(tag_filter);
        // `(rowid, account, date, amount)` per yielded leg with a commodity.
        let mut legs: Vec<(
            i64,
            bc_models::AccountId,
            jiff::civil::Date,
            bc_models::Amount,
        )> = Vec::new();
        {
            let mut stream = pin!(resolved_transactions(&mut *tx, &filter)?);
            while let Some(resolved) = stream.try_next().await? {
                for leg in resolved.legs() {
                    let Some(commodity) = leg.commodity() else {
                        continue;
                    };
                    let account = leg
                        .account()
                        .as_str()
                        .parse::<bc_models::AccountId>()
                        .map_err(|e| {
                            crate::BcError::BadData(format!(
                                "invalid account_id '{}': {e}",
                                leg.account().as_str()
                            ))
                        })?;
                    legs.push((
                        leg.posting(),
                        account,
                        resolved.date(),
                        bc_models::Amount::new(
                            leg.value(),
                            bc_models::CommodityCode::new(commodity.as_str()),
                        ),
                    ));
                }
            }
        }
        let rowids: Vec<i64> = legs.iter().map(|(rowid, ..)| *rowid).collect();
        let ids = Self::posting_ids_for_rowids(&mut tx, &rowids).await?;
        // Nothing was written, so the snapshot is released rather than committed.
        tx.rollback().await?;

        let mut postings = Vec::with_capacity(legs.len());
        for (rowid, account, date, amount) in legs {
            let posting_id = ids.get(&rowid).cloned().ok_or_else(|| {
                crate::BcError::BadData(format!(
                    "posting rowid {rowid} vanished inside one snapshot"
                ))
            })?;
            let key = PostingKey {
                posting_id,
                commodity: amount.commodity().as_str().to_owned(),
            };
            postings.push((key, account, date, amount));
        }
        if let Some(wanted) = keys {
            postings.retain(|(key, ..)| wanted.contains(key));
        }
        Ok(postings)
    }

    /// Maps posting rowids to posting ids within one snapshot.
    ///
    /// # Arguments
    ///
    /// * `conn` - The connection holding the snapshot the rowids came from.
    /// * `rowids` - Rowids to resolve; duplicates are harmless.
    ///
    /// # Returns
    ///
    /// Each found rowid mapped to its posting id.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure or if the rowid list
    /// cannot be serialised.
    async fn posting_ids_for_rowids(
        conn: &mut sqlx::SqliteConnection,
        rowids: &[i64],
    ) -> crate::BcResult<HashMap<i64, String>> {
        if rowids.is_empty() {
            return Ok(HashMap::new());
        }
        let json = serde_json::to_string(rowids)
            .map_err(|e| crate::BcError::BadData(format!("rowid list serialisation: {e}")))?;
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT rowid, id FROM postings WHERE rowid IN (SELECT value FROM json_each(?))",
        )
        .bind(json)
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Folds one bucket of posting amounts under `rev`.
    ///
    /// Each amount is valued with [`Self::value_in`]. Amounts derive from
    /// [`Self::fetch_postings`], already narrowed to a query's keys, so an
    /// elided leg's components were admitted one by one. Under a target
    /// commodity an amount no rate can value goes to `unvalued`. Under
    /// tracking-only the group with the largest absolute total is the
    /// result, and every other group goes to `unvalued` whole.
    ///
    /// # Arguments
    ///
    /// * `rev` - The revision governing this bucket's period.
    /// * `amounts` - The bucket.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::BadData`] on decimal overflow.
    fn fold_actuals(
        &self,
        rev: &bc_models::BudgetRevision,
        amounts: &[bc_models::Amount],
    ) -> crate::BcResult<PeriodActuals> {
        let mut unvalued = bc_models::Balances::new();
        let tracking_groups = match rev.target() {
            Some(_) => None,
            None => Some(commodity_groups(amounts)?),
        };
        let dominant = tracking_groups.as_ref().and_then(dominant_commodity);
        let commodity = rev
            .target()
            .map(|t| t.commodity().clone())
            .or_else(|| dominant.clone());

        let mut total = bc_models::Decimal::ZERO;
        for posting_amount in amounts {
            match self.value_in(rev, dominant.as_ref(), posting_amount) {
                Some(value) => {
                    total = total
                        .checked_add(value)
                        .ok_or_else(|| crate::BcError::BadData("actuals sum overflow".into()))?;
                }
                // A tracking-only miss is reported per group below.
                None if tracking_groups.is_some() => {}
                None => add_unvalued(&mut unvalued, posting_amount)?,
            }
        }
        for (code, value) in tracking_groups.into_iter().flatten() {
            if dominant.as_ref().is_none_or(|d| d.as_str() != code) {
                add_unvalued(&mut unvalued, &bc_models::Amount::new(value, code))?;
            }
        }
        Ok(PeriodActuals {
            total,
            commodity,
            unvalued,
        })
    }

    /// Values `amount` in the commodity `rev` budgets in.
    ///
    /// # Arguments
    ///
    /// * `rev` - The revision governing the amount's period.
    /// * `dominant` - The commodity a tracking-only revision values in: the
    ///   window commodity for a window posting, the bucket's
    ///   [`dominant_commodity`] for a carry-chain posting. Read only when `rev`
    ///   is tracking-only.
    /// * `amount` - The native amount.
    ///
    /// # Returns
    ///
    /// Under a target commodity, `amount` converted by the FX service, or
    /// `None` when no rate reaches the target. Under tracking-only, the
    /// amount's own value when its commodity is `dominant`, else `None`.
    fn value_in(
        &self,
        rev: &bc_models::BudgetRevision,
        dominant: Option<&bc_models::CommodityCode>,
        amount: &bc_models::Amount,
    ) -> Option<bc_models::Decimal> {
        match rev.target() {
            Some(target) => self
                .fx
                .convert(amount, target.commodity())
                .ok()
                .map(|a| a.value()),
            None => (dominant == Some(amount.commodity())).then(|| amount.value()),
        }
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
        buckets: &[Vec<ExpandedPosting>],
    ) -> crate::BcResult<(bc_models::Decimal, bc_models::Balances)> {
        let mut carry = bc_models::Decimal::ZERO;
        let mut unvalued = bc_models::Balances::new();
        for (k, (period, bucket)) in chain.iter().zip(buckets).enumerate() {
            let allocated = period_share(period.revision, period.start, period.start, period.end);
            let amounts: Vec<bc_models::Amount> =
                bucket.iter().map(|(_, _, _, a)| a.clone()).collect();
            let spent = self.fold_actuals(period.revision, &amounts)?;
            merge_unvalued(&mut unvalued, &spent.unvalued)?;
            #[expect(clippy::arithmetic_side_effects, reason = "decimal budget arithmetic")]
            let surplus = allocated + carry - spent.total;
            let next = chain.get(k.saturating_add(1)).map_or(dst, |n| n.revision);
            carry = apply_rollover_policy(next, surplus);
        }
        Ok((carry, unvalued))
    }
}

/// Target pro-rated to the day count of `[seg_start, seg_end)`.
///
/// The denominator is the revision's natural period length anchored at
/// `period_start` (the period's true start), so the ratio stays correct when
/// the segment is clipped to a window edge or truncated to a boundary stub.
/// `period_start` must be the resolved period's start; `seg_start`/`seg_end`
/// the (possibly clipped) portion being measured.
#[inline]
fn period_share(
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

/// Target for `[start, end)`: each overlapping period's share, pro-rated on
/// its natural length and rounded to cents, summed. `None` when no
/// overlapping period has a target.
///
/// Rounding per period makes the shares of adjacent sub-windows sum exactly
/// to the share of their union.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] on overflow.
pub(crate) fn prorated_target(
    revs: &[bc_models::BudgetRevision],
    start: jiff::civil::Date,
    end: jiff::civil::Date,
) -> crate::BcResult<Option<bc_models::Decimal>> {
    let periods = bc_models::periods_overlapping(revs, start, end);
    if periods.iter().all(|p| p.revision.target().is_none()) {
        return Ok(None);
    }
    periods
        .iter()
        .map(|p| period_share(p.revision, p.start, p.start.max(start), p.end.min(end)))
        .try_fold(bc_models::Decimal::ZERO, bc_models::Decimal::checked_add)
        .map(Some)
        .ok_or_else(|| crate::BcError::BadData("target overflow".into()))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod budget_service_tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::BudgetIntent;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;

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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget")
            .value;

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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        let future = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .target(Amount::new(
                Decimal::from(9000_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        let bad = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2026, 6, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::CapAtTarget)
            .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
                .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;

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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
                .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
                .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;

        // Add revision B effective 2027-01-01.
        let rev_b = bc_models::BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2027, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;

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
    use bc_models::BudgetId;
    use bc_models::BudgetIntent;
    use bc_models::BudgetRevision;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Period;
    use bc_models::PostingId;
    use bc_models::RolloverPolicy;
    use bc_models::TagId;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::BudgetService;
    use super::BudgetStatusEngine;
    use super::PostingKey;
    use super::ValuedPosting;
    use super::sign_flips;
    use crate::BcError;
    use crate::Warning;
    use crate::account::Service as AccountService;
    use crate::fx::noop_fx;
    use crate::search::build;

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
            .intent(BudgetIntent::Limit)
            .maybe_tag_filter(tag_filter.cloned())
            .call()
            .await
            .expect("create budget")
            .value;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget")
            .value;
        budget
    }

    /// A fresh, valid transaction id for a raw-SQL fixture the query engine
    /// hydrates.
    fn tx_id() -> String {
        TransactionId::new().to_string()
    }

    /// A fresh, valid posting id for a raw-SQL fixture the query engine
    /// hydrates.
    fn posting_id() -> String {
        PostingId::new().to_string()
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
            &tx_id(),
            "2026-03-05",
            &[
                (&posting_id(), &bank, Some(("-7.00", "AUD"))),
                (&posting_id(), &cafes, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-06",
            &[
                (&posting_id(), &bank, Some(("-60.00", "AUD"))),
                (&posting_id(), &groceries, None),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery::new(
            Some(build::accounts(core::slice::from_ref(&cafes)).expect("accounts")),
            None,
            None,
        );
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
            &tx_id(),
            "2026-03-05",
            &[
                (&posting_id(), &bank, Some(("-5.00", "AUD"))),
                (&posting_id(), &food, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-06",
            &[
                (&posting_id(), &bank, Some(("-80.00", "AUD"))),
                (&posting_id(), &food, None),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery::new(
            Some(build::amount(Some(dec!(50)), None, None)),
            None,
            None,
        );
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

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_window_fetches_nothing(pool: SqlitePool) {
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());

        let postings = engine
            .fetch_postings(
                &food,
                Date::constant(2026, 1, 1),
                Date::constant(2026, 2, 1),
                None,
                None,
            )
            .await
            .expect("fetch");

        assert!(postings.is_empty(), "{postings:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn malformed_account_id_in_the_subtree_fails_with_bad_data(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        // A child of Food whose stored id does not parse as an `AccountId`.
        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, kind, parent_id, created_at) \
             VALUES ('acc_bad', 'Bad', 'expense', 'deposit_account', ?, \
             '2026-01-01T00:00:00Z')",
        )
        .bind(food.to_string())
        .execute(&pool)
        .await
        .expect("insert malformed account");
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES ('tx_1', '2026-03-05', 'Test', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert transaction");
        for (id, account, amount, position) in [
            ("p_bank", bank.to_string(), "-5.00", 0_i64),
            ("p_bad", "acc_bad".to_owned(), "5.00", 1),
        ] {
            sqlx::query(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, \
                 position) VALUES (?, 'tx_1', ?, ?, 'AUD', ?)",
            )
            .bind(id)
            .bind(account)
            .bind(amount)
            .bind(position)
            .execute(&pool)
            .await
            .expect("insert posting");
        }
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());

        let err = engine
            .fetch_postings(
                &food,
                Date::constant(2026, 3, 1),
                Date::constant(2026, 4, 1),
                None,
                None,
            )
            .await
            .expect_err("a malformed account id must fail");

        assert!(
            matches!(&err, BcError::BadData(m) if m.contains("invalid account_id 'acc_bad'")),
            "got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn fetched_postings_carry_their_posting_ids_accounts_and_dates(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let cafes = account(&pool, "Cafes", AccountType::Expense, Some(&food)).await;
        insert_tx(
            &pool,
            "tx_mar",
            "2026-03-05",
            &[
                ("p_bank", &bank, Some(("-20.00", "AUD"))),
                ("p_food", &food, Some(("12.50", "AUD"))),
                ("p_cafes", &cafes, None),
            ],
        )
        .await;
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());

        let mut postings = engine
            .fetch_postings(
                &food,
                Date::constant(2026, 3, 1),
                Date::constant(2026, 4, 1),
                None,
                None,
            )
            .await
            .expect("fetch");
        postings.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(
            postings,
            vec![
                (
                    key("p_cafes", "AUD"),
                    cafes,
                    Date::constant(2026, 3, 5),
                    Amount::new(dec!(7.50), CommodityCode::new("AUD")),
                ),
                (
                    key("p_food", "AUD"),
                    food,
                    Date::constant(2026, 3, 5),
                    Amount::new(dec!(12.50), CommodityCode::new("AUD")),
                ),
            ]
        );
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget")
            .value;
        // February: 30 on Food, 20 on Cafe → surplus 150; January untouched → 200.
        insert_tx(
            &pool,
            &tx_id(),
            "2026-02-10",
            &[
                (&posting_id(), &bank, Some(("-30.00", "AUD"))),
                (&posting_id(), &food, Some(("30.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-02-11",
            &[
                (&posting_id(), &bank, Some(("-20.00", "AUD"))),
                (&posting_id(), &cafe, Some(("20.00", "AUD"))),
            ],
        )
        .await;
        // March: 40 on Food, 15 on Cafe.
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-03",
            &[
                (&posting_id(), &bank, Some(("-40.00", "AUD"))),
                (&posting_id(), &food, Some(("40.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-04",
            &[
                (&posting_id(), &bank, Some(("-15.00", "AUD"))),
                (&posting_id(), &cafe, Some(("15.00", "AUD"))),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery::new(
            Some(build::accounts(core::slice::from_ref(&cafe)).expect("accounts")),
            None,
            None,
        );
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
            .intent(BudgetIntent::Limit)
            .tag_filter(home.clone())
            .call()
            .await
            .expect("create budget")
            .value;
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2026, 2, 1))
                .target(target())
                .period(Period::Monthly)
                .rollover(RolloverPolicy::CarryForward)
                .intent(BudgetIntent::Limit)
                .tag_filter(work.clone())
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");
        // (date, account, amount, tag). January: only `home` counts → 30,
        // surplus 70. February: only `work` counts → 40, surplus 100 + 70 − 40
        // = 130. March under the Cafe query: only the `work`-tagged Cafe
        // posting counts → 25.
        let fixtures: [(&str, &AccountId, &str, &TagId); 6] = [
            ("2026-01-10", &food, "30.00", &home),
            ("2026-01-11", &food, "20.00", &work),
            ("2026-02-10", &food, "40.00", &work),
            ("2026-02-11", &food, "10.00", &home),
            ("2026-03-03", &cafe, "25.00", &work),
            ("2026-03-04", &food, "5.00", &work),
        ];
        for (date, acct, amount, tag) in fixtures {
            let negated = format!("-{amount}");
            let tx = tx_id();
            insert_tx(
                &pool,
                &tx,
                date,
                &[
                    (&posting_id(), &bank, Some((&negated, "AUD"))),
                    (&posting_id(), acct, Some((amount, "AUD"))),
                ],
            )
            .await;
            tag_tx(&pool, &tx, tag).await;
        }
        let query = crate::search::TransactionQuery::new(
            Some(build::accounts(core::slice::from_ref(&cafe)).expect("accounts")),
            None,
            None,
        );
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget")
            .value;
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
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget")
            .value;
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

    // MARK: Valued postings

    /// March 2026 as a custom window.
    fn march() -> bc_models::BudgetWindow {
        bc_models::BudgetWindow::custom(
            Date::constant(2026, 3, 1),
            Date::constant(2026, 4, 1),
            "March",
        )
    }

    /// A [`PostingKey`] for `posting_id` in `commodity`.
    fn key(posting_id: &str, commodity: &str) -> PostingKey {
        PostingKey {
            posting_id: posting_id.into(),
            commodity: commodity.into(),
        }
    }

    /// `window_postings` for `budget` over March under `noop_fx`, sorted by key.
    async fn march_postings(
        pool: &SqlitePool,
        budget: &Budget,
    ) -> (Vec<ValuedPosting>, Option<CommodityCode>) {
        let valuation = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .window_postings(budget, &march(), None)
            .await
            .expect("window postings");
        let mut postings = valuation.postings;
        postings.sort_by(|a, b| a.key.cmp(&b.key));
        (postings, valuation.commodity)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_postings_sum_to_status_actuals(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let expenses = account(&pool, "Expenses", AccountType::Expense, None).await;
        let food = account(&pool, "Food", AccountType::Expense, Some(&expenses)).await;
        let groceries = account(&pool, "Groceries", AccountType::Expense, Some(&food)).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_food",
            "2026-03-04",
            &[
                ("p_bank_f", &bank, Some(("-30.00", "AUD"))),
                ("p_food", &food, Some(("30.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_groc",
            "2026-03-12",
            &[
                ("p_bank_g", &bank, Some(("-45.50", "AUD"))),
                ("p_groc", &groceries, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_feb",
            "2026-02-27",
            &[
                ("p_bank_feb", &bank, Some(("-9.00", "AUD"))),
                ("p_food_feb", &food, None),
            ],
        )
        .await;

        let (postings, commodity) = march_postings(&pool, &budget).await;
        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, march(), None)
            .await
            .expect("status");

        let sum: Decimal = postings.iter().filter_map(|p| p.value).sum();
        assert_eq!(sum, status.actuals);
        assert_eq!(sum, dec!(75.50));
        assert_eq!(commodity, status.commodity);
        assert_eq!(
            postings,
            vec![
                ValuedPosting {
                    key: key("p_food", "AUD"),
                    account_id: food.clone(),
                    date: Date::constant(2026, 3, 4),
                    value: Some(dec!(30.00)),
                    amount: Amount::new(dec!(30.00), CommodityCode::new("AUD")),
                },
                ValuedPosting {
                    key: key("p_groc", "AUD"),
                    account_id: groceries.clone(),
                    date: Date::constant(2026, 3, 12),
                    value: Some(dec!(45.50)),
                    amount: Amount::new(dec!(45.50), CommodityCode::new("AUD")),
                },
            ]
        );
        assert!(
            postings
                .iter()
                .all(|p| p.account_id == food || p.account_id == groceries),
            "{postings:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_postings_keep_unvaluable_postings(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let expenses = account(&pool, "Expenses", AccountType::Expense, None).await;
        let food = account(&pool, "Food", AccountType::Expense, Some(&expenses)).await;
        let budget = monthly_budget(&pool, &food, None).await;
        insert_tx(
            &pool,
            "tx_aud",
            "2026-03-05",
            &[
                ("p_bank_a", &bank, Some(("-20.00", "AUD"))),
                ("p_food_a", &food, Some(("20.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_xyz",
            "2026-03-06",
            &[
                ("p_bank_x", &bank, Some(("-3.00", "XYZ"))),
                ("p_food_x", &food, Some(("3.00", "XYZ"))),
            ],
        )
        .await;

        let (postings, commodity) = march_postings(&pool, &budget).await;

        assert_eq!(commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(
            postings,
            vec![
                ValuedPosting {
                    key: key("p_food_a", "AUD"),
                    account_id: food.clone(),
                    date: Date::constant(2026, 3, 5),
                    value: Some(dec!(20.00)),
                    amount: Amount::new(dec!(20.00), CommodityCode::new("AUD")),
                },
                ValuedPosting {
                    key: key("p_food_x", "XYZ"),
                    account_id: food.clone(),
                    date: Date::constant(2026, 3, 6),
                    value: None,
                    amount: Amount::new(dec!(3.00), CommodityCode::new("XYZ")),
                },
            ]
        );
    }

    /// Under tracking-only, only the bucket's dominant commodity is valued.
    #[sqlx::test(migrations = "./migrations")]
    async fn window_postings_value_the_dominant_commodity_when_tracking_only(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = budget_with_target(&pool, &food, None, None).await;
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

        let (postings, commodity) = march_postings(&pool, &budget).await;
        let status = march_actuals(&pool, &budget).await;

        assert_eq!(commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(commodity, status.commodity);
        assert_eq!(
            postings,
            vec![
                ValuedPosting {
                    key: key("p_food_a", "AUD"),
                    account_id: food.clone(),
                    date: Date::constant(2026, 3, 5),
                    value: Some(dec!(40.00)),
                    amount: Amount::new(dec!(40.00), CommodityCode::new("AUD")),
                },
                ValuedPosting {
                    key: key("p_food_u", "USD"),
                    account_id: food.clone(),
                    date: Date::constant(2026, 3, 6),
                    value: None,
                    amount: Amount::new(dec!(10.00), CommodityCode::new("USD")),
                },
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn subtree_postings_expand_elided_legs(pool: SqlitePool) {
        let assets = account(&pool, "Assets", AccountType::Asset, None).await;
        let bank = account(&pool, "Bank", AccountType::Asset, Some(&assets)).await;
        let expenses = account(&pool, "Expenses", AccountType::Expense, None).await;
        let widgets = account(&pool, "Widgets", AccountType::Expense, Some(&expenses)).await;
        insert_tx(
            &pool,
            "tx_widget",
            "2026-03-10",
            &[
                ("p_bank", &bank, Some(("-40.00", "AUD"))),
                ("p_widgets", &widgets, None),
            ],
        )
        .await;

        let postings = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .subtree_postings(&expenses, &march(), None)
            .await
            .expect("subtree postings");

        assert_eq!(
            postings,
            vec![(
                key("p_widgets", "AUD"),
                widgets,
                Amount::new(dec!(40.00), CommodityCode::new("AUD")),
            )]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn subtree_postings_apply_the_amount_filter(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let expenses = account(&pool, "Expenses", AccountType::Expense, None).await;
        let widgets = account(&pool, "Widgets", AccountType::Expense, Some(&expenses)).await;
        let widgets_large = posting_id();
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-05",
            &[
                (&posting_id(), &bank, Some(("-5.00", "AUD"))),
                (&posting_id(), &widgets, None),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-06",
            &[
                (&posting_id(), &bank, Some(("-80.00", "AUD"))),
                (&widgets_large, &widgets, Some(("80.00", "AUD"))),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery::new(
            Some(build::amount(Some(dec!(50)), None, None)),
            None,
            None,
        );

        let postings = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .subtree_postings(&expenses, &march(), Some(&query))
            .await
            .expect("subtree postings");

        assert_eq!(
            postings,
            vec![(
                key(&widgets_large, "AUD"),
                widgets,
                Amount::new(dec!(80.00), CommodityCode::new("AUD")),
            )]
        );
    }

    // MARK: Target expressions and sign flips

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::arithmetic_side_effects, reason = "decimal test expectation")]
    async fn expression_target_is_evaluated_and_stored(pool: SqlitePool) {
        let acct = account(&pool, "Widgets", AccountType::Expense, None).await;
        let svc = BudgetService::new(pool.clone());
        let created = svc
            .create()
            .account_id(acct)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(0), CommodityCode::new("AUD")))
            .target_expr("(100,000 * 6.00 / 100 / 365)")
            .intent(BudgetIntent::Estimate)
            .period(Period::Daily)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
        let (budget, _) = created.value;
        let revs = svc.revisions(budget.id()).await.expect("revs");
        let rev = revs.first().expect("one revision");
        assert_eq!(rev.target_expr(), Some("(100,000 * 6.00 / 100 / 365)"));
        assert_eq!(
            rev.target().map(Amount::value),
            Some(dec!(100000) * dec!(6.00) / dec!(100) / dec!(365))
        );
        assert_eq!(rev.intent(), BudgetIntent::Estimate);
    }

    /// Creates a budget whose target is `expr`, returning the service's error.
    async fn create_with_expr(pool: &SqlitePool, expr: &str) -> BcError {
        let acct = account(pool, "Widgets", AccountType::Expense, None).await;
        BudgetService::new(pool.clone())
            .create()
            .account_id(acct)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(0), CommodityCode::new("AUD")))
            .target_expr(expr)
            .intent(BudgetIntent::Limit)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect_err("rejected")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn division_by_zero_expression_is_invalid_input(pool: SqlitePool) {
        let err = create_with_expr(&pool, "1 / 0").await;
        assert!(matches!(err, BcError::InvalidInput(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unparsable_expression_is_invalid_input(pool: SqlitePool) {
        let err = create_with_expr(&pool, "(1 + ").await;
        assert!(matches!(err, BcError::InvalidInput(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amending_to_a_literal_clears_the_expression(pool: SqlitePool) {
        let acct = account(&pool, "Widgets", AccountType::Expense, None).await;
        let svc = BudgetService::new(pool.clone());
        let (budget, rev) = svc
            .create()
            .account_id(acct)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(0), CommodityCode::new("AUD")))
            .target_expr("(30.00 / 4)")
            .intent(BudgetIntent::Limit)
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create")
            .value;
        assert_eq!(rev.target().map(Amount::value), Some(dec!(7.5)));

        let literal = BudgetRevision::builder()
            .id(rev.id().clone())
            .budget_id(budget.id().clone())
            .effective_from(rev.effective_from())
            .target(Amount::new(dec!(9.00), CommodityCode::new("AUD")))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .created_at(*rev.created_at())
            .build();
        svc.revise(budget.id(), literal).await.expect("revise");

        let revs = svc.revisions(budget.id()).await.expect("revs");
        assert_eq!(revs.len(), 1, "amended in place");
        let stored = revs.first().expect("one revision");
        assert_eq!(stored.id(), rev.id());
        assert_eq!(stored.target().map(Amount::value), Some(dec!(9.00)));
        assert_eq!(stored.target_expr(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sign_flip_between_revisions_warns(pool: SqlitePool) {
        let acct = account(&pool, "Salary", AccountType::Income, None).await;
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(acct)
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(-500), CommodityCode::new("AUD")))
            .intent(BudgetIntent::Goal)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create")
            .value;

        let flipped = BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(Date::constant(2026, 2, 1))
            .target(Amount::new(dec!(500), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Goal)
            .created_at(Timestamp::now())
            .build();
        let revised = svc
            .revise(budget.id(), flipped)
            .await
            .expect("a sign flip saves");

        assert_eq!(
            revised.warnings,
            vec![Warning::BudgetSignFlip {
                budget_id: budget.id().clone(),
                effective_from: Date::constant(2026, 2, 1),
                neighbour_effective_from: Date::constant(2026, 1, 1),
            }]
        );
        assert_eq!(
            svc.revisions(budget.id()).await.expect("revs").len(),
            2,
            "the flipped revision is stored"
        );
    }

    #[rstest]
    #[case::same_sign(&[Some(dec!(100)), Some(dec!(200))], 0)]
    #[case::flip(&[Some(dec!(-100)), Some(dec!(100))], 2)]
    #[case::middle_flags_both_neighbours(&[Some(dec!(100)), Some(dec!(-100)), Some(dec!(100))], 3)]
    #[case::zero_never_flips(&[Some(dec!(-100)), Some(dec!(0)), Some(dec!(100))], 0)]
    #[case::tracking_only_never_flips(&[Some(dec!(-100)), None, Some(dec!(100))], 0)]
    fn sign_flips_detects_adjacent_changes(
        #[case] targets: &[Option<Decimal>],
        #[case] flagged: usize,
    ) {
        let budget_id = BudgetId::new();
        let revs: Vec<BudgetRevision> = (1_i8..)
            .zip(targets)
            .map(|(month, target)| {
                BudgetRevision::builder()
                    .budget_id(budget_id.clone())
                    .effective_from(Date::constant(2026, month, 1))
                    .maybe_target(target.map(|v| Amount::new(v, CommodityCode::new("AUD"))))
                    .period(Period::Monthly)
                    .rollover(RolloverPolicy::ResetToZero)
                    .intent(BudgetIntent::Limit)
                    .created_at(Timestamp::now())
                    .build()
            })
            .collect();
        assert_eq!(sign_flips(&revs).len(), flagged);
    }

    #[test]
    fn sign_flips_reports_the_previous_neighbour_first() {
        let budget_id = BudgetId::new();
        let revs: Vec<BudgetRevision> = [(1, dec!(100)), (2, dec!(-100)), (3, dec!(100))]
            .into_iter()
            .map(|(month, value)| {
                BudgetRevision::builder()
                    .budget_id(budget_id.clone())
                    .effective_from(Date::constant(2026, month, 1))
                    .target(Amount::new(value, CommodityCode::new("AUD")))
                    .period(Period::Monthly)
                    .rollover(RolloverPolicy::ResetToZero)
                    .intent(BudgetIntent::Limit)
                    .created_at(Timestamp::now())
                    .build()
            })
            .collect();
        // The middle revision disagrees with both sides and reports January.
        let expected: Vec<_> = revs
            .iter()
            .zip([2, 1, 2])
            .map(|(r, month)| (r.id().clone(), Date::constant(2026, month, 1)))
            .collect();
        assert_eq!(sign_flips(&revs), expected);
    }

    // MARK: Window-wide dominant commodity

    /// A tracking-only window picks one dominant commodity across its periods.
    #[sqlx::test(migrations = "./migrations")]
    async fn tracking_only_window_picks_one_dominant_across_periods(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = budget_with_target(&pool, &food, None, None).await;
        insert_tx(
            &pool,
            "tx_jan",
            "2026-01-10",
            &[
                ("p_bank_j", &bank, Some(("-90.00", "AUD"))),
                ("p_food_j", &food, Some(("90.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_feb",
            "2026-02-10",
            &[
                ("p_bank_f", &bank, Some(("-30.00", "USD"))),
                ("p_food_f", &food, Some(("30.00", "USD"))),
            ],
        )
        .await;
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 1, 1),
            Date::constant(2026, 3, 1),
            "Jan-Feb",
        );
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());

        let status = engine
            .status_for_window(&budget, window.clone(), None)
            .await
            .expect("status");
        let valuation = engine
            .window_postings(&budget, &window, None)
            .await
            .expect("postings");

        assert_eq!(status.actuals, dec!(90.00));
        assert_eq!(status.commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(30.00)));
        assert_eq!(valuation.commodity, Some(CommodityCode::new("AUD")));
        let total: Decimal = valuation.postings.iter().filter_map(|p| p.value).sum();
        assert_eq!(total, status.actuals);
        assert_eq!(valuation.unvalued.get("USD"), Some(dec!(30.00)));
    }

    /// The amount filter applies before the dominant commodity is chosen.
    #[sqlx::test(migrations = "./migrations")]
    async fn amount_filter_applies_before_dominant_choice(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = budget_with_target(&pool, &food, None, None).await;
        let food_aud = posting_id();
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-05",
            &[
                (&posting_id(), &bank, Some(("-40.00", "AUD"))),
                (&food_aud, &food, Some(("40.00", "AUD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            &tx_id(),
            "2026-03-06",
            &[
                (&posting_id(), &bank, Some(("-500.00", "USD"))),
                (&posting_id(), &food, Some(("500.00", "USD"))),
            ],
        )
        .await;
        let query = crate::search::TransactionQuery::new(
            Some(build::amount(None, Some(dec!(99.99)), None)),
            None,
            None,
        );
        let engine = BudgetStatusEngine::new(pool.clone(), noop_fx());
        let status = engine
            .status_for_window(&budget, march(), Some(&query))
            .await
            .expect("status");
        assert_eq!(status.commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(status.actuals, dec!(40.00));

        // The tree and native sub-rows read `window_postings`, which filters in
        // its own load loop.
        let valuation = engine
            .window_postings(&budget, &march(), Some(&query))
            .await
            .expect("postings");
        assert_eq!(valuation.commodity, Some(CommodityCode::new("AUD")));
        let values: Vec<_> = valuation
            .postings
            .iter()
            .map(|p| (p.key.posting_id.as_str(), p.value))
            .collect();
        assert_eq!(values, vec![(food_aud.as_str(), Some(dec!(40.00)))]);
        assert!(valuation.unvalued.is_empty());
    }

    /// A window spanning a tracking-only and a targeted revision values every
    /// posting in the later target's commodity, leaving tracking-only spend in
    /// another commodity unvalued.
    #[sqlx::test(migrations = "./migrations")]
    async fn mixed_tracking_and_targeted_window_values_in_the_target_commodity(pool: SqlitePool) {
        let bank = account(&pool, "Bank", AccountType::Asset, None).await;
        let food = account(&pool, "Food", AccountType::Expense, None).await;
        let budget = budget_with_target(&pool, &food, None, None).await;
        BudgetService::new(pool.clone())
            .revise(
                budget.id(),
                BudgetRevision::builder()
                    .budget_id(budget.id().clone())
                    .effective_from(Date::constant(2026, 2, 1))
                    .target(Amount::new(dec!(200), CommodityCode::new("AUD")))
                    .period(Period::Monthly)
                    .rollover(RolloverPolicy::ResetToZero)
                    .intent(BudgetIntent::Limit)
                    .created_at(Timestamp::now())
                    .build(),
            )
            .await
            .expect("revise");
        insert_tx(
            &pool,
            "tx_jan",
            "2026-01-10",
            &[
                ("p_bank_j", &bank, Some(("-20.00", "USD"))),
                ("p_food_j", &food, Some(("20.00", "USD"))),
            ],
        )
        .await;
        insert_tx(
            &pool,
            "tx_feb",
            "2026-02-10",
            &[
                ("p_bank_f", &bank, Some(("-50.00", "AUD"))),
                ("p_food_f", &food, Some(("50.00", "AUD"))),
            ],
        )
        .await;
        let window = bc_models::BudgetWindow::custom(
            Date::constant(2026, 1, 1),
            Date::constant(2026, 3, 1),
            "Jan-Feb",
        );
        let valuation = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .window_postings(&budget, &window, None)
            .await
            .expect("postings");
        let by_id = |id: &str| {
            valuation
                .postings
                .iter()
                .find(|p| p.key.posting_id == id)
                .expect(id)
        };
        // February's target fixes the window commodity at AUD.
        assert_eq!(valuation.commodity, Some(CommodityCode::new("AUD")));
        // January is tracking-only: its USD posting is off-commodity.
        assert_eq!(by_id("p_food_j").value, None);
        assert_eq!(valuation.unvalued.get("USD"), Some(dec!(20.00)));
        assert_eq!(valuation.unvalued.len(), 1);
        // February targets AUD.
        assert_eq!(by_id("p_food_f").value, Some(dec!(50.00)));
        let total: Decimal = valuation.postings.iter().filter_map(|p| p.value).sum();
        assert_eq!(total, dec!(50.00));

        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(&budget, window, None)
            .await
            .expect("status");
        assert_eq!(status.commodity, Some(CommodityCode::new("AUD")));
        assert_eq!(status.actuals, dec!(50.00));
        assert_eq!(status.unvalued.get("USD"), Some(dec!(20.00)));
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod load_plan_tests {
    use bc_models::BudgetId;
    use bc_models::BudgetIntent;
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
            .intent(BudgetIntent::Limit)
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
