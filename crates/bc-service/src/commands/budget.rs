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
    let query = budget_query(filter)?;

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
    let query = budget_query(filter)?;

    let budget = state
        .budgets
        .get(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let native = state
        .budget_tree
        .native_periods(&budget, display_start, display_end, query.as_ref())
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let revs = state
        .budgets
        .revisions(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let gov = bc_core::governing_revision(&revs, display_start);
    let commodity = gov
        .and_then(|r| r.target())
        .map(|t| t.commodity().as_str().to_owned())
        .ok_or_else(|| {
            bc_ipc::BcError::Internal(
                "budget has no target for the selected period — commodity required for amount conversion"
                    .to_owned(),
            )
        })?;

    Ok(native
        .iter()
        .map(|n| {
            let label = format_native_period_label(n);
            bc_ipc::NativePeriodRow::from_native(n, label, commodity.as_str())
        })
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

/// Returns the transactions behind one budget tree row, newest first.
///
/// Each transaction carries the bucket label of its first posting that landed
/// in a budget beneath the row, and whether any of its postings counts in two
/// budgets neither of whose rows nests the other.
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
    let query = budget_query(filter)?;

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

    // Posting id -> (first bucket label seen, any key double-counted).
    let mut by_posting: HashMap<String, (Option<String>, bool)> = HashMap::new();
    for (key, bucket, double_counted) in postings {
        let entry = by_posting.entry(key.posting_id).or_default();
        if entry.0.is_none() {
            entry.0 = bucket;
        }
        entry.1 |= double_counted;
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
            let matched: Vec<&(Option<String>, bool)> = t
                .postings()
                .iter()
                .filter_map(|p| by_posting.get(&p.id().to_string()))
                .collect();
            let bucket = matched.iter().find_map(|(bucket, _)| bucket.clone());
            let double_counted = matched.iter().any(|(_, dc)| *dc);
            bc_ipc::BudgetRowTransaction::new(
                bc_ipc::Transaction::from_model_with_accounts(t, &account_map, &forest),
                bucket,
                double_counted,
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

/// Converts an optional UI [`bc_ipc::Filter`] into a budget-path
/// [`bc_core::search::TransactionQuery`], stripping the date and balance
/// dimensions.
///
/// Budgets are period-gridded; the display window is driven solely by
/// `PeriodNav`, so any `date_from`/`date_until` bounds are cleared before
/// the emptiness check and before conversion. Budget actuals assume double
/// entry, which an unbalanced transaction violates, so `balance` is cleared
/// too. Returns `None` for an absent filter, or one that is empty once
/// dates and balance are stripped — including a date-only or balance-only
/// filter, either of which is inert on budgets (reproducing the unfiltered
/// path).
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if an account/tag id fails to parse.
fn budget_query(
    filter: Option<bc_ipc::Filter>,
) -> Result<Option<bc_core::search::TransactionQuery>, bc_ipc::BcError> {
    let Some(mut stripped) = filter else {
        return Ok(None);
    };
    stripped.date_from = None;
    stripped.date_until = None;
    stripped.balance = None;
    if stripped == bc_ipc::Filter::default() {
        return Ok(None);
    }
    let query = bc_core::search::TransactionQuery::try_from(stripped)?;
    Ok(Some(query))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

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

    #[test]
    fn budget_query_strips_date_bounds() {
        let mut filter = bc_ipc::Filter::default();
        filter.date_from = Some(jiff::civil::date(2026, 6, 10));
        filter.date_until = Some(jiff::civil::date(2026, 6, 20));
        filter.text = Some("coffee".to_owned());
        let q = super::budget_query(Some(filter))
            .expect("convert")
            .expect("some");
        assert_eq!(q.date_from, None);
        assert_eq!(q.date_until, None);
        assert_eq!(q.text.as_deref(), Some("coffee"));
    }

    #[test]
    fn budget_query_none_for_empty() {
        assert!(super::budget_query(None).expect("ok").is_none());
        assert!(
            super::budget_query(Some(bc_ipc::Filter::default()))
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn budget_query_none_for_balance_only() {
        let mut filter = bc_ipc::Filter::default();
        filter.balance = Some(bc_ipc::BalanceStatus::Unbalanced);
        assert!(super::budget_query(Some(filter)).expect("ok").is_none());
    }

    #[test]
    fn budget_query_date_only_is_none() {
        // A date-only filter is inert on budgets, so it must collapse to the
        // unfiltered path (None) — dates are stripped before the empty check.
        let mut filter = bc_ipc::Filter::default();
        filter.date_from = Some(jiff::civil::date(2026, 6, 1));
        filter.date_until = Some(jiff::civil::date(2026, 6, 30));
        assert!(super::budget_query(Some(filter)).expect("ok").is_none());
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
}
