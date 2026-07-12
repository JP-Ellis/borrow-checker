//! Tauri command handlers for budget operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use bc_core::ipc::NativePeriodRowExt as _;
use bc_core::ipc::TransactionExt as _;
use tauri::State;

use crate::AppState;

// MARK: Overview

/// Returns the budget overview (summary + tree) for a display window.
///
/// # Arguments
///
/// * `period_type` - The display period type (monthly, weekly, etc.).
/// * `period_start` - The display window start date.
/// * `filter` - The global filter, with the date dimension ignored.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the period is invalid or if a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_budget_overview(
    period_type: bc_ipc::Period,
    period_start: jiff::civil::Date,
    filter: Option<bc_ipc::Filter>,
    state: State<'_, AppState>,
) -> Result<(bc_ipc::BudgetSummary, Vec<bc_ipc::BudgetTreeNode>), bc_ipc::BcError> {
    let period = bc_models::Period::from(period_type);
    let query = budget_query(filter)?;

    let overview = state
        .budget_tree
        .get_overview(&period, period_start, query.as_ref())
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let nodes: Vec<bc_ipc::BudgetTreeNode> = overview
        .nodes
        .iter()
        .map(bc_ipc::BudgetTreeNode::from)
        .collect::<Vec<_>>();

    let summary = bc_ipc::BudgetSummary::from(&overview.summary);

    Ok((summary, nodes))
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
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid, or if a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_native_periods(
    budget_id: String,
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
    filter: Option<bc_ipc::Filter>,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::NativePeriodRow>, bc_ipc::BcError> {
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

// MARK: Budget transactions

/// Returns transactions matching a budget in a date range.
///
/// # Arguments
///
/// * `budget_id` - The budget to query.
/// * `period_start` - The period start date (inclusive).
/// * `period_end` - The period end date (exclusive).
/// * `filter` - The global filter, with the date dimension ignored.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid, or if a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_budget_transactions(
    budget_id: String,
    period_start: jiff::civil::Date,
    period_end: jiff::civil::Date,
    filter: Option<bc_ipc::Filter>,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::Transaction>, bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;
    let query = budget_query(filter)?;

    let budget = state
        .budgets
        .get(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let revs = state
        .budgets
        .revisions(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let gov = bc_core::governing_revision(&revs, period_start);
    let tag_filter = gov.and_then(|r| r.tag_filter());

    let txns = state
        .transactions
        .list_for_budget(
            budget.account_id(),
            tag_filter,
            period_start,
            period_end,
            query.as_ref(),
        )
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let account_map: std::collections::HashMap<String, &bc_models::Account> =
        accounts.iter().map(|a| (a.id().to_string(), a)).collect();

    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txns
        .iter()
        .map(|t| bc_ipc::Transaction::from_model_with_accounts(t, &account_map, &forest))
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
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid or a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_budget_revisions(
    budget_id: String,
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::BudgetRevisionView>, bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let revs = state
        .budgets
        .revisions(&bid)
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
                .period(period_ipc.clone())
                .period_label(period_ipc.label())
                .rollover(bc_ipc::RolloverPolicy::from(r.rollover()))
                .maybe_tag_filter(r.tag_filter().map(ToString::to_string))
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
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid or a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn resolve_effective_date(
    budget_id: String,
    date: jiff::civil::Date,
    exclude_revision_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<jiff::civil::Date, bc_ipc::BcError> {
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
/// resolves snap beforehand). `target` and `target_currency` must be
/// both set or both omitted.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid, the target fields are
/// inconsistent, or the service call fails (including effective-date conflicts).
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri IPC command args map 1-to-1 to the bc-ipc contract"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn revise_budget(
    budget_id: String,
    revision_id: Option<String>,
    effective_from: jiff::civil::Date,
    name: Option<String>,
    target: Option<rust_decimal::Decimal>,
    target_currency: Option<String>,
    rollover: bc_ipc::RolloverPolicy,
    period: bc_ipc::Period,
    tag_filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let rev_id = match revision_id {
        Some(s) => s
            .parse::<bc_models::BudgetRevisionId>()
            .map_err(|e| bc_ipc::BcError::Validation(format!("invalid revision_id: {e}")))?,
        None => bc_models::BudgetRevisionId::new(),
    };

    let target_amount = match (target, target_currency) {
        (Some(value), Some(cur)) => Some(bc_models::Amount::new(
            value,
            bc_models::CommodityCode::new(cur),
        )),
        (None, None) => None,
        _ => {
            return Err(bc_ipc::BcError::Validation(
                "target and target_currency must both be set or both null".to_owned(),
            ));
        }
    };

    let tag = tag_filter
        .as_deref()
        .map(|s| {
            s.parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag_filter: {e}")))
        })
        .transpose()?;

    let revision = bc_models::BudgetRevision::builder()
        .id(rev_id)
        .budget_id(bid.clone())
        .effective_from(effective_from)
        .maybe_name(name)
        .maybe_target(target_amount)
        .period(bc_models::Period::from(period))
        .rollover(bc_models::RolloverPolicy::from(rollover))
        .maybe_tag_filter(tag)
        .created_at(jiff::Timestamp::now())
        .build();

    state
        .budgets
        .revise(&bid, revision)
        .await
        .map(|_| ())
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

/// Removes a revision from a budget.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if an ID is invalid, the revision is the last one,
/// or the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_budget_revision(
    budget_id: String,
    revision_id: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
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
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn archive_budget(
    budget_id: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
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
/// Both `target` and `target_currency` must be provided together, or both omitted.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if any ID is invalid, the target fields are inconsistent,
/// or the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri IPC command args map 1-to-1 to the bc-ipc contract; a wrapper struct would require extra serde round-trips"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_budget(
    account_id: String,
    effective_from: jiff::civil::Date,
    name: Option<String>,
    target: Option<rust_decimal::Decimal>,
    target_currency: Option<String>,
    period: bc_ipc::Period,
    rollover: bc_ipc::RolloverPolicy,
    tag_filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let aid = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let target_amount = match (target, target_currency) {
        (Some(value), Some(cur)) => Some(bc_models::Amount::new(
            value,
            bc_models::CommodityCode::new(cur),
        )),
        (None, None) => None,
        _ => {
            return Err(bc_ipc::BcError::Validation(
                "target and target_currency must both be set or both null".to_owned(),
            ));
        }
    };

    let tag: Option<bc_models::TagId> = tag_filter
        .as_deref()
        .map(|s| {
            s.parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag_filter: {e}")))
        })
        .transpose()?;

    state
        .budgets
        .create()
        .account_id(aid)
        .effective_from(effective_from)
        .maybe_name(name)
        .maybe_target(target_amount)
        .period(bc_models::Period::from(period))
        .rollover(bc_models::RolloverPolicy::from(rollover))
        .maybe_tag_filter(tag)
        .call()
        .await
        .map(|_| ())
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

// MARK: Posting spread

/// Sets the accrual spread date range on a posting.
///
/// # Arguments
///
/// * `posting_id` - The posting to update.
/// * `spread_from` - The first day of the accrual window (inclusive).
/// * `spread_until` - The last day of the accrual window (inclusive).
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the posting ID is invalid, or the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn set_posting_spread(
    posting_id: String,
    spread_from: jiff::civil::Date,
    spread_until: jiff::civil::Date,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
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
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the posting ID is invalid or the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn clear_posting_spread(
    posting_id: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let pid = posting_id
        .parse::<bc_models::PostingId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid posting_id: {e}")))?;

    state
        .transactions
        .clear_posting_spread(&pid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

// MARK: Filter conversion

/// Converts an optional UI [`bc_ipc::Filter`] into a budget-path
/// [`bc_core::search::TransactionQuery`], stripping the date dimension.
///
/// Budgets are period-gridded; the display window is driven solely by
/// `PeriodNav`, so any `date_from`/`date_until` bounds are cleared before
/// the emptiness check and before conversion. Returns `None` for an absent
/// filter, or one that is empty once dates are stripped — including a
/// date-only filter, which is inert on budgets (reproducing the unfiltered
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
    if stripped == bc_ipc::Filter::default() {
        return Ok(None);
    }
    let query = bc_core::search::TransactionQuery::try_from(stripped)?;
    Ok(Some(query))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

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
    fn budget_query_date_only_is_none() {
        // A date-only filter is inert on budgets, so it must collapse to the
        // unfiltered path (None) — dates are stripped before the empty check.
        let mut filter = bc_ipc::Filter::default();
        filter.date_from = Some(jiff::civil::date(2026, 6, 1));
        filter.date_until = Some(jiff::civil::date(2026, 6, 30));
        assert!(super::budget_query(Some(filter)).expect("ok").is_none());
    }
}
