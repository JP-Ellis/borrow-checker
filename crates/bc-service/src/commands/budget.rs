//! Command handlers for budget operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use std::collections::HashMap;

use bc_core::ipc::NativePeriodRowExt as _;
use bc_core::ipc::TransactionExt as _;

use crate::AppState;

// MARK: Overview

/// Returns the budget overview (summary, tree and pace) for a display window.
///
/// # Arguments
///
/// * `period_type` - The display period type (monthly, weekly, etc.).
/// * `period_start` - The display window start date.
/// * `filter` - The global filter, with the date dimension ignored.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the period is invalid or if a service call fails.
pub async fn get_budget_overview(
    state: &AppState,
    args: bc_ipc::commands::GetBudgetOverviewArgs,
) -> Result<bc_ipc::BudgetOverview, bc_ipc::BcError> {
    let bc_ipc::commands::GetBudgetOverviewArgs {
        period_type,
        period_start,
        filter,
        ..
    } = args;
    let period = bc_models::Period::from(period_type);
    let query = budget_query(state, filter).await?;

    let overview = state
        .budget_tree
        .get_overview(
            &period,
            period_start,
            query.as_ref(),
            jiff::Zoned::now().date(),
        )
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::BudgetOverview::from(&overview))
}

// MARK: Native periods

/// Returns native period sub-rows for one budget in a display window.
///
/// # Arguments
///
/// * `budget_id` - The budget to expand.
/// * `display_start` - The display window start date (inclusive).
/// * `display_end` - The display window end date (exclusive).
/// * `filter` - The global filter, with the date dimension ignored.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid, or if a service call fails.
pub async fn get_native_periods(
    state: &AppState,
    args: bc_ipc::commands::GetNativePeriodsArgs,
) -> Result<Vec<bc_ipc::NativePeriodRow>, bc_ipc::BcError> {
    let bc_ipc::commands::GetNativePeriodsArgs {
        budget_id,
        display_start,
        display_end,
        filter,
        ..
    } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;
    let query = budget_query(state, filter).await?;

    let budget = state
        .budgets
        .get(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let native = state
        .budget_tree
        .native_periods(
            &budget,
            display_start,
            display_end,
            query.as_ref(),
            jiff::Zoned::now().date(),
        )
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(native
        .iter()
        .map(|n| bc_ipc::NativePeriodRow::from_native(n, format_native_period_label(n)))
        .collect())
}

/// Builds a human-readable label for a native period overlap row.
fn format_native_period_label(n: &bc_core::NativePeriodStatus) -> String {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "jiff::Span::get_days() on a date difference is always non-negative and bounded"
    )]
    let overlap_days = (n.overlap.overlap_end - n.overlap.overlap_start).get_days();
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "jiff::Span::get_days() on a date difference is always non-negative and bounded"
    )]
    let native_days = (n.overlap.native_end - n.overlap.native_start).get_days();
    if overlap_days == native_days {
        n.overlap.native_start.to_string()
    } else {
        format!(
            "{} ({overlap_days} of {native_days} days)",
            n.overlap.native_start
        )
    }
}

// MARK: Budget row transactions

/// What a budget row counted from one posting, accumulated over its keys.
///
/// An elided leg whose residual spans several commodities yields one key per
/// commodity, so the keys fold into one entry per posting.
#[derive(Default)]
struct Counted {
    /// Label of the budget the posting landed in beneath the row.
    bucket: Option<String>,
    /// Whether any key counts in two budgets neither of whose rows nests the other.
    overlaps: bool,
    /// Valued amounts, in the row's commodity.
    values: Vec<bc_models::Amount>,
    /// Native amounts that could not be valued.
    unvalued: Vec<bc_models::Amount>,
}

/// Sums `values` when they share one commodity; `None` when there are none,
/// they span commodities or the sum overflows.
fn contribution(values: &[&bc_models::Amount]) -> Option<bc_ipc::Amount> {
    let first = values.first()?;
    let mut total = rust_decimal::Decimal::ZERO;
    for amount in values {
        if amount.commodity() != first.commodity() {
            return None;
        }
        total = total.checked_add(amount.value())?;
    }
    Some(bc_ipc::Amount::new(total, first.commodity().as_str()))
}

/// Returns the transactions behind one budget tree row, newest first.
///
/// Each transaction carries the bucket label of its first posting that landed
/// in a budget beneath the row, the postings the row counted (`counted`), the
/// valued sum core contributed to the row's actual (`contribution`), the
/// native amounts that fed no total (`unvalued`), and whether any of its
/// postings counts in two budgets neither of whose rows nests the other.
///
/// # Arguments
///
/// * `row_id` - The tree row's id.
/// * `period_type` - The display period type.
/// * `period_start` - The display window start date.
/// * `filter` - The global filter, with the date dimension ignored.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::NotFound`] when no row has `row_id`, or
/// [`bc_ipc::BcError`] if a service call fails.
pub async fn get_budget_row_transactions(
    state: &AppState,
    args: bc_ipc::commands::GetBudgetRowTransactionsArgs,
) -> Result<Vec<bc_ipc::BudgetRowTransaction>, bc_ipc::BcError> {
    let bc_ipc::commands::GetBudgetRowTransactionsArgs {
        row_id,
        period_type,
        period_start,
        filter,
        ..
    } = args;
    let period = bc_models::Period::from(period_type);
    let query = budget_query(state, filter).await?;

    let postings = state
        .budget_tree
        .row_postings(
            &row_id,
            &period,
            period_start,
            query.as_ref(),
            jiff::Zoned::now().date(),
        )
        .await?;

    let mut by_posting: HashMap<String, Counted> = HashMap::new();
    for (posting, double_counted) in postings {
        let entry = by_posting.entry(posting.key.posting_id).or_default();
        if entry.bucket.is_none() {
            entry.bucket = posting.bucket;
        }
        entry.overlaps |= double_counted;
        match posting.value {
            Some(value) => entry.values.push(value),
            None => entry.unvalued.push(posting.amount),
        }
    }
    let posting_ids: Vec<String> = by_posting.keys().cloned().collect();

    let tx_ids = state
        .transactions
        .ids_for_postings(&posting_ids)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let mut txns = Vec::with_capacity(tx_ids.len());
    for id in &tx_ids {
        txns.push(
            state
                .transactions
                .find_by_id(id)
                .await
                .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?,
        );
    }
    newest_first(&mut txns);

    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let account_map: HashMap<String, &bc_models::Account> =
        accounts.iter().map(|a| (a.id().to_string(), a)).collect();

    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txns
        .iter()
        .map(|t| {
            let (ids, matched): (Vec<String>, Vec<&Counted>) = t
                .postings()
                .iter()
                .filter_map(|p| {
                    let id = p.id().to_string();
                    by_posting.get(&id).map(|counted| (id, counted))
                })
                .unzip();
            let bucket = matched.iter().find_map(|c| c.bucket.clone());
            let double_counted = matched.iter().any(|c| c.overlaps);
            let values: Vec<&bc_models::Amount> =
                matched.iter().flat_map(|c| c.values.iter()).collect();
            let mut unvalued = bc_models::Balances::new();
            for amount in matched.iter().flat_map(|c| c.unvalued.iter()) {
                if unvalued.try_add(amount).is_err() {
                    tracing::warn!("unvalued total overflowed; amount left out of the row");
                }
            }
            bc_ipc::BudgetRowTransaction::new(
                bc_ipc::Transaction::from_model_with_accounts(t, &account_map, &forest),
                bucket,
                double_counted,
                ids,
                contribution(&values),
                unvalued
                    .iter()
                    .map(|(code, value)| bc_ipc::Amount::new(value, code))
                    .collect(),
            )
        })
        .collect())
}

// MARK: Budget mutations

/// Lists a budget's revisions, annotated for a display window.
///
/// # Arguments
///
/// * `budget_id` - The budget whose revisions to list.
/// * `display_start` - Display window start (inclusive).
/// * `display_end` - Display window end (exclusive).
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid or a service call fails.
pub async fn list_budget_revisions(
    state: &AppState,
    args: bc_ipc::commands::ListBudgetRevisionsArgs,
) -> Result<Vec<bc_ipc::BudgetRevisionView>, bc_ipc::BcError> {
    let bc_ipc::commands::ListBudgetRevisionsArgs {
        budget_id,
        display_start,
        display_end,
        ..
    } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let revs = state
        .budgets
        .revisions(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let flips = bc_core::sign_flips(&revs);
    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    revs.iter()
        .enumerate()
        .map(|(i, r)| {
            let reign_end = revs
                .get(i.saturating_add(1))
                .map(bc_models::BudgetRevision::effective_from);
            let period_ipc = bc_ipc::Period::from(r.period());
            let target = r.target().map(bc_ipc::Amount::from);
            Ok(bc_ipc::BudgetRevisionView::builder()
                .id(r.id().to_string())
                .effective_from(r.effective_from())
                .maybe_reign_end(reign_end)
                .maybe_name(r.name().map(str::to_owned))
                .maybe_target(target)
                .maybe_target_expr(r.target_expr().map(str::to_owned))
                .intent(r.intent().into())
                .sign_flip(flips.iter().any(|(id, _)| id == r.id()))
                .period(period_ipc.clone())
                .period_label(period_ipc.label())
                .rollover(bc_ipc::RolloverPolicy::from(r.rollover()))
                .maybe_tag_filter(r.tag_filter().map(|id| {
                    let path = forest
                        .path_of(id)
                        .map_or_else(|| id.to_string(), |p| p.to_string());
                    bc_ipc::TagInfo::new(id.to_string(), path)
                }))
                .maybe_window_overlap(crate::ipc::window_overlap(
                    r.effective_from(),
                    reign_end,
                    display_start,
                    display_end,
                ))
                .build())
        })
        .collect()
}

/// Resolves a snap effective date to the next period-grid boundary.
///
/// # Arguments
///
/// * `budget_id` - The budget providing the revision grid.
/// * `date` - The candidate effective date.
/// * `exclude_revision_id` - Revision id to ignore (the one being amended), or `None`.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid or a service call fails.
pub async fn resolve_effective_date(
    state: &AppState,
    args: bc_ipc::commands::ResolveEffectiveDateArgs,
) -> Result<jiff::civil::Date, bc_ipc::BcError> {
    let bc_ipc::commands::ResolveEffectiveDateArgs {
        budget_id,
        date,
        exclude_revision_id,
        ..
    } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;
    let exclude = exclude_revision_id
        .as_deref()
        .map(str::parse::<bc_models::BudgetRevisionId>)
        .transpose()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid revision_id: {e}")))?;

    let revs = state
        .budgets
        .revisions(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_models::snap_to_grid_boundary(
        &revs,
        date,
        exclude.as_ref(),
    ))
}

/// Adds a new revision or amends an existing one.
///
/// `revision_id = None` adds a revision (a fresh id is generated); `Some` amends
/// the revision with that id. `effective_from` must already be exact (the UI
/// resolves snap beforehand). `target` is the text as typed and
/// `target_currency` must be set exactly when it is. `intent = None` keeps the
/// amended revision's intent, else uses the account type's default.
///
/// # Returns
///
/// The write's warnings, rendered for display.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid, the target fields are
/// inconsistent, or the service call fails (including effective-date conflicts).
pub async fn revise_budget(
    state: &AppState,
    args: bc_ipc::commands::ReviseBudgetArgs,
) -> Result<Vec<String>, bc_ipc::BcError> {
    let bc_ipc::commands::ReviseBudgetArgs {
        budget_id,
        revision_id,
        effective_from,
        name,
        target,
        target_currency,
        intent,
        rollover,
        period,
        tag_filter,
        ..
    } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let amended = revision_id
        .as_deref()
        .map(str::parse::<bc_models::BudgetRevisionId>)
        .transpose()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid revision_id: {e}")))?;

    let (target_amount, target_expr) = split_target(target, target_currency)?;

    let tag = tag_filter
        .as_ref()
        .map(|t| {
            t.id.parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag_filter: {e}")))
        })
        .transpose()?;

    let resolved_intent = match intent {
        Some(i) => i.into(),
        None => stored_or_default_intent(state, &bid, amended.as_ref()).await?,
    };

    let revision = bc_models::BudgetRevision::builder()
        .id(amended.unwrap_or_else(bc_models::BudgetRevisionId::new))
        .budget_id(bid.clone())
        .effective_from(effective_from)
        .maybe_name(name)
        .maybe_target(target_amount)
        .maybe_target_expr(target_expr)
        .period(bc_models::Period::from(period))
        .rollover(bc_models::RolloverPolicy::from(rollover))
        .intent(resolved_intent)
        .maybe_tag_filter(tag)
        .created_at(jiff::Timestamp::now())
        .build();

    state
        .budgets
        .revise(&bid, revision)
        .await
        .map(|warned| warned.warnings.iter().map(ToString::to_string).collect())
        .map_err(write_error)
}

/// Resolves an omitted intent for a revision write.
///
/// # Arguments
///
/// * `state` - Shared application state.
/// * `budget_id` - The budget being revised.
/// * `amended` - The revision being amended, or `None` for a new revision.
///
/// # Returns
///
/// The amended revision's stored intent when it exists, else the default for
/// the budget account's type.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::NotFound`] if the budget or its account is
/// missing, or another [`bc_ipc::BcError`] if a service call fails.
async fn stored_or_default_intent(
    state: &AppState,
    budget_id: &bc_models::BudgetId,
    amended: Option<&bc_models::BudgetRevisionId>,
) -> Result<bc_models::BudgetIntent, bc_ipc::BcError> {
    if let Some(rid) = amended
        && let Some(stored) = state
            .budgets
            .revisions(budget_id)
            .await?
            .iter()
            .find(|r| r.id() == rid)
    {
        return Ok(stored.intent());
    }
    let account_id = state.budgets.get(budget_id).await?.account_id().clone();
    Ok(bc_models::BudgetIntent::default_for(
        state.accounts.find_by_id(&account_id).await?.account_type(),
    ))
}

/// Splits target text into an amount and, for an expression, its source.
///
/// # Arguments
///
/// * `raw` - The target as typed: a decimal literal or an expression.
/// * `currency` - The target's commodity code.
///
/// # Returns
///
/// The evaluated target and the expression text (`None` for a literal), or
/// `(None, None)` when both inputs are absent.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if exactly one of `raw` and
/// `currency` is set, or if `raw` does not evaluate.
fn split_target(
    raw: Option<String>,
    currency: Option<String>,
) -> Result<(Option<bc_models::Amount>, Option<String>), bc_ipc::BcError> {
    match (raw, currency) {
        (None, None) => Ok((None, None)),
        (Some(text), Some(cur)) => {
            let (value, expr) = bc_expr::split(&text)
                .map_err(|e| bc_ipc::BcError::Validation(format!("target: {e}")))?;
            Ok((
                Some(bc_models::Amount::new(
                    value,
                    bc_models::CommodityCode::new(cur),
                )),
                expr,
            ))
        }
        _ => Err(bc_ipc::BcError::Validation(
            "target and target_currency must both be set or both null".to_owned(),
        )),
    }
}

/// Maps a budget write's core error to its IPC form.
///
/// `InvalidInput` keeps its bare message; every other variant goes through
/// the shared [`From`] mapping, so a missing budget stays `NotFound`.
fn write_error(e: bc_core::BcError) -> bc_ipc::BcError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "bc_core::BcError is #[non_exhaustive]; only InvalidInput is special-cased"
    )]
    match e {
        bc_core::BcError::InvalidInput(m) => bc_ipc::BcError::Validation(m),
        other => other.into(),
    }
}

/// Removes a revision from a budget.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid, the revision is the last one,
/// or the service call fails.
pub async fn remove_budget_revision(
    state: &AppState,
    args: bc_ipc::commands::RemoveBudgetRevisionArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::RemoveBudgetRevisionArgs {
        budget_id,
        revision_id,
        ..
    } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;
    let rid = revision_id
        .parse::<bc_models::BudgetRevisionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid revision_id: {e}")))?;

    state
        .budgets
        .remove_revision(&bid, &rid)
        .await
        .map_err(|e| {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "bc_core::BcError is #[non_exhaustive]; all non-validation errors map to Internal"
            )]
            match e {
                bc_core::BcError::InvalidInput(m) => bc_ipc::BcError::Validation(m),
                other => bc_ipc::BcError::Internal(other.to_string()),
            }
        })
}

/// Archives a budget (soft-deletes it).
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid or the service call fails.
pub async fn archive_budget(
    state: &AppState,
    args: bc_ipc::commands::ArchiveBudgetArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::ArchiveBudgetArgs { budget_id, .. } = args;
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    state
        .budgets
        .archive(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Creates a new budget on an account.
///
/// `target` is the text as typed; both `target` and `target_currency` must be
/// provided together, or both omitted. `intent = None` uses the account type's
/// default.
///
/// # Returns
///
/// The write's warnings, rendered for display.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if any ID is invalid, the target fields are inconsistent,
/// or the service call fails.
pub async fn create_budget(
    state: &AppState,
    args: bc_ipc::commands::CreateBudgetArgs,
) -> Result<Vec<String>, bc_ipc::BcError> {
    let bc_ipc::commands::CreateBudgetArgs {
        account_id,
        effective_from,
        name,
        target,
        target_currency,
        intent,
        period,
        rollover,
        tag_filter,
        ..
    } = args;
    let aid = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let (target_amount, target_expr) = split_target(target, target_currency)?;

    let tag: Option<bc_models::TagId> = tag_filter
        .as_ref()
        .map(|t| {
            t.id.parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag_filter: {e}")))
        })
        .transpose()?;

    let resolved_intent = match intent {
        Some(i) => i.into(),
        None => bc_models::BudgetIntent::default_for(
            state.accounts.find_by_id(&aid).await?.account_type(),
        ),
    };

    state
        .budgets
        .create()
        .account_id(aid)
        .effective_from(effective_from)
        .maybe_name(name)
        .maybe_target(target_amount)
        .maybe_target_expr(target_expr)
        .period(bc_models::Period::from(period))
        .rollover(bc_models::RolloverPolicy::from(rollover))
        .intent(resolved_intent)
        .maybe_tag_filter(tag)
        .call()
        .await
        .map(|warned| warned.warnings.iter().map(ToString::to_string).collect())
        .map_err(write_error)
}

// MARK: Posting spread

/// Sets the accrual spread date range on a posting.
///
/// # Arguments
///
/// * `posting_id` - The posting to update.
/// * `spread_from` - The first day of the accrual window (inclusive).
/// * `spread_until` - The last day of the accrual window (inclusive).
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the posting ID is invalid, or the service call fails.
pub async fn set_posting_spread(
    state: &AppState,
    args: bc_ipc::commands::SetPostingSpreadArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::SetPostingSpreadArgs {
        posting_id,
        spread_from,
        spread_until,
        ..
    } = args;
    let pid = posting_id
        .parse::<bc_models::PostingId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid posting_id: {e}")))?;

    state
        .transactions
        .set_posting_spread(&pid, spread_from, spread_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Clears the accrual spread from a posting.
///
/// # Arguments
///
/// * `posting_id` - The posting to update.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the posting ID is invalid or the service call fails.
pub async fn clear_posting_spread(
    state: &AppState,
    args: bc_ipc::commands::ClearPostingSpreadArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::ClearPostingSpreadArgs { posting_id, .. } = args;
    let pid = posting_id
        .parse::<bc_models::PostingId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid posting_id: {e}")))?;

    state
        .transactions
        .clear_posting_spread(&pid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Sorts `txns` by date, newest first, breaking same-date ties by id
/// (newest first, as ids are time-ordered) so the order is stable across
/// calls.
fn newest_first(txns: &mut [bc_models::Transaction]) {
    txns.sort_by_cached_key(|t| core::cmp::Reverse((t.date(), t.id().to_string())));
}

// MARK: Filter conversion

/// Resolves an optional UI [`bc_ipc::Filter`] into the query a budget
/// evaluates: [`bc_core::search::TransactionQuery::for_budget`] drops the dates
/// and the top-level date and balance-status terms. Returns `None` for an
/// absent filter, or one with nothing left once they are dropped, so the
/// budget takes its unfiltered path.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Query`] for text that does not parse or resolve.
async fn budget_query(
    state: &AppState,
    filter: Option<bc_ipc::Filter>,
) -> Result<Option<bc_core::search::TransactionQuery>, bc_ipc::BcError> {
    let Some(given) = filter else {
        return Ok(None);
    };
    Ok(state
        .transactions
        .query_from_filter(given)
        .await?
        .for_budget())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use tempfile::TempDir;

    use super::*;

    fn transaction(date: jiff::civil::Date) -> bc_models::Transaction {
        bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date)
            .description("fixture")
            .postings(Vec::new())
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .build()
    }

    #[test]
    fn newest_first_breaks_date_ties_by_id() {
        let older = transaction(jiff::civil::date(2026, 6, 1));
        let mut same_day = [
            transaction(jiff::civil::date(2026, 6, 10)),
            transaction(jiff::civil::date(2026, 6, 10)),
        ];
        same_day.sort_by_key(|t| t.id().to_string());
        let [low, high] = same_day;
        let expected = vec![high.id().clone(), low.id().clone(), older.id().clone()];
        let mut txns = vec![older, low, high];

        newest_first(&mut txns);

        let ids: Vec<bc_models::TransactionId> = txns.iter().map(|t| t.id().clone()).collect();
        assert_eq!(ids, expected);
    }

    #[rstest]
    #[case(Some("250"), Some("AUD"), Some(dec!(250)), None)]
    #[case(Some("(30.00 / 4)"), Some("AUD"), Some(dec!(7.5)), Some("(30.00 / 4)"))]
    #[case(None, None, None, None)]
    fn split_target_cases(
        #[case] raw: Option<&str>,
        #[case] cur: Option<&str>,
        #[case] value: Option<Decimal>,
        #[case] expr: Option<&str>,
    ) {
        let (amount, e) = split_target(raw.map(str::to_owned), cur.map(str::to_owned)).expect("ok");
        assert_eq!(amount.map(|a| a.value()), value);
        assert_eq!(e.as_deref(), expr);
    }

    #[test]
    fn split_target_reports_bad_expression() {
        let err = split_target(Some("1 / 0".into()), Some("AUD".into())).expect_err("bad");
        assert!(matches!(err, bc_ipc::BcError::Validation(_)));
    }

    // MARK: Row transaction fixtures

    /// A fresh state on a database inside `dir`, with config lookup isolated.
    async fn open_state(dir: &TempDir) -> AppState {
        // SAFETY: nextest runs each test in its own process, before any thread
        // reads the environment.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config")) }
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        std::fs::create_dir_all(dir.path().join("backups")).expect("mkdir backups");
        AppState::open(&settings).await.expect("open")
    }

    async fn account(
        state: &AppState,
        name: &str,
        ty: bc_models::AccountType,
        parent: Option<&bc_models::AccountId>,
    ) -> bc_models::AccountId {
        state
            .accounts
            .create()
            .name(name)
            .account_type(ty)
            .kind(bc_models::AccountKind::DepositAccount)
            .maybe_parent_id(parent)
            .call()
            .await
            .expect(name)
    }

    fn aud(v: Decimal) -> bc_models::Amount {
        bc_models::Amount::new(v, bc_models::CommodityCode::new("AUD"))
    }

    async fn monthly_aud_budget(
        state: &AppState,
        on: &bc_models::AccountId,
        target: Decimal,
    ) -> bc_models::Budget {
        let (budget, _) = state
            .budgets
            .create()
            .account_id(on.clone())
            .effective_from(jiff::civil::date(2026, 1, 1))
            .target(aud(target))
            .period(bc_models::Period::Monthly)
            .rollover(bc_models::RolloverPolicy::ResetToZero)
            .intent(bc_models::BudgetIntent::Limit)
            .call()
            .await
            .expect("budget")
            .value;
        budget
    }

    /// A June transaction from `legs`: `(posting id, account, amount)`, `None` elided.
    async fn june_tx(
        state: &AppState,
        legs: Vec<(
            bc_models::PostingId,
            &bc_models::AccountId,
            Option<bc_models::Amount>,
        )>,
    ) {
        let postings = legs
            .into_iter()
            .map(|(id, acct, amount)| {
                bc_models::Posting::builder()
                    .id(id)
                    .account_id(acct.clone())
                    .maybe_amount(amount)
                    .build()
            })
            .collect();
        state
            .transactions
            .create(
                bc_models::Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(jiff::civil::date(2026, 6, 3))
                    .description("Shop")
                    .postings(postings)
                    .reconciliation(bc_models::Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
    }

    async fn rows(state: &AppState, row_id: String) -> Vec<bc_ipc::BudgetRowTransaction> {
        let args = serde_json::from_value(serde_json::json!({
            "row_id": row_id,
            "period_type": { "type": "monthly" },
            "period_start": "2026-06-01",
            "filter": null,
        }))
        .expect("args");
        get_budget_row_transactions(state, args)
            .await
            .expect("rows")
    }

    #[tokio::test]
    async fn child_account_posting_is_counted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let food = account(&state, "Food", bc_models::AccountType::Expense, None).await;
        let groceries = account(
            &state,
            "Groceries",
            bc_models::AccountType::Expense,
            Some(&food),
        )
        .await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let budget = monthly_aud_budget(&state, &food, dec!(200)).await;
        let grocery_leg = bc_models::PostingId::new();
        june_tx(
            &state,
            vec![
                (grocery_leg.clone(), &groceries, Some(aud(dec!(40)))),
                (bc_models::PostingId::new(), &bank, Some(aud(dec!(-40)))),
            ],
        )
        .await;

        let list = rows(&state, budget.id().to_string()).await;
        let row = list.first().expect("one transaction");
        assert_eq!(row.counted, vec![grocery_leg.to_string()]);
        assert_eq!(row.contribution, Some(bc_ipc::Amount::new(dec!(40), "AUD")));
        assert_eq!(row.unvalued, Vec::<bc_ipc::Amount>::new());
    }

    #[tokio::test]
    async fn multi_commodity_elided_leg_is_counted_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let food = account(&state, "Food", bc_models::AccountType::Expense, None).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let budget = monthly_aud_budget(&state, &food, dec!(200)).await;
        let food_leg = bc_models::PostingId::new();
        june_tx(
            &state,
            vec![
                (bc_models::PostingId::new(), &bank, Some(aud(dec!(-40)))),
                (
                    bc_models::PostingId::new(),
                    &bank,
                    Some(bc_models::Amount::new(
                        dec!(-10),
                        bc_models::CommodityCode::new("USD"),
                    )),
                ),
                (food_leg.clone(), &food, None),
            ],
        )
        .await;

        let list = rows(&state, budget.id().to_string()).await;
        let row = list.first().expect("one transaction");
        assert_eq!(row.counted, vec![food_leg.to_string()]);
        assert_eq!(row.contribution, Some(bc_ipc::Amount::new(dec!(40), "AUD")));
        assert_eq!(row.unvalued, vec![bc_ipc::Amount::new(dec!(10), "USD")]);
    }

    /// Finds the node with `id` anywhere in `nodes`.
    fn find_node<'a>(
        nodes: &'a [bc_ipc::BudgetTreeNode],
        id: &str,
    ) -> Option<&'a bc_ipc::BudgetTreeNode> {
        nodes.iter().find_map(|n| {
            if n.id == id {
                Some(n)
            } else {
                find_node(&n.children, id)
            }
        })
    }

    #[tokio::test]
    async fn panel_contributions_sum_to_the_rows_actual() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let food = account(&state, "Food", bc_models::AccountType::Expense, None).await;
        let groceries = account(
            &state,
            "Groceries",
            bc_models::AccountType::Expense,
            Some(&food),
        )
        .await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let budget = monthly_aud_budget(&state, &food, dec!(200)).await;
        june_tx(
            &state,
            vec![
                (bc_models::PostingId::new(), &groceries, Some(aud(dec!(40)))),
                (bc_models::PostingId::new(), &bank, Some(aud(dec!(-40)))),
            ],
        )
        .await;
        june_tx(
            &state,
            vec![
                (bc_models::PostingId::new(), &bank, Some(aud(dec!(-35)))),
                (
                    bc_models::PostingId::new(),
                    &bank,
                    Some(bc_models::Amount::new(
                        dec!(-10),
                        bc_models::CommodityCode::new("USD"),
                    )),
                ),
                (bc_models::PostingId::new(), &food, None),
            ],
        )
        .await;

        let args = serde_json::from_value(serde_json::json!({
            "period_type": { "type": "monthly" },
            "period_start": "2026-06-01",
            "filter": null,
        }))
        .expect("args");
        let overview = get_budget_overview(&state, args).await.expect("overview");
        let node = find_node(&overview.nodes, &budget.id().to_string()).expect("budget row");
        let actual = node.actual.clone().expect("actual");
        assert_eq!(actual, bc_ipc::Amount::new(dec!(75), "AUD"));

        let list = rows(&state, budget.id().to_string()).await;
        let total: Decimal = list
            .iter()
            .filter_map(|r| r.contribution.as_ref())
            .map(|c| c.value)
            .sum();
        assert_eq!(total, actual.value);
        let panel_unvalued: Vec<bc_ipc::Amount> =
            list.iter().flat_map(|r| r.unvalued.clone()).collect();
        assert_eq!(panel_unvalued, vec![bc_ipc::Amount::new(dec!(10), "USD")]);
        assert_eq!(node.unvalued, panel_unvalued);
    }

    #[tokio::test]
    async fn unallocated_row_counts_only_its_own_postings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let food = account(&state, "Food", bc_models::AccountType::Expense, None).await;
        let groceries = account(
            &state,
            "Groceries",
            bc_models::AccountType::Expense,
            Some(&food),
        )
        .await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let envelope = monthly_aud_budget(&state, &food, dec!(200)).await;
        monthly_aud_budget(&state, &groceries, dec!(100)).await;
        let food_leg = bc_models::PostingId::new();
        june_tx(
            &state,
            vec![
                (food_leg.clone(), &food, Some(aud(dec!(15)))),
                (bc_models::PostingId::new(), &groceries, Some(aud(dec!(25)))),
                (bc_models::PostingId::new(), &bank, Some(aud(dec!(-40)))),
            ],
        )
        .await;

        let list = rows(&state, format!("unalloc:{}", envelope.id())).await;
        let row = list.first().expect("one transaction");
        assert_eq!(row.counted, vec![food_leg.to_string()]);
        assert_eq!(row.contribution, Some(bc_ipc::Amount::new(dec!(15), "AUD")));
    }
}
