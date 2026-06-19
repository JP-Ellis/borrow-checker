//! Tauri command handlers for budget operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

use crate::AppState;
use crate::ipc::IntoIpc as _;
use crate::ipc::IntoModel as _;

// MARK: Overview

/// Returns the budget overview (summary + tree) for a display window.
///
/// # Arguments
///
/// * `period_type` - The display period type (monthly, weekly, etc.).
/// * `period_start` - The display window start date.
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
    state: State<'_, AppState>,
) -> Result<(bc_ipc::BudgetSummary, Vec<bc_ipc::BudgetTreeNode>), bc_ipc::BcError> {
    let period = period_type.into_model();

    let overview = state
        .budget_tree
        .get_overview(&period, period_start)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let nodes: Vec<bc_ipc::BudgetTreeNode> = overview
        .nodes
        .iter()
        .map(crate::ipc::budget_tree_item_into_ipc)
        .collect::<Result<Vec<_>, _>>()?;

    let target_commodity = overview.summary.commodity.as_ref();

    let (total_budgeted, total_spent, total_remaining) = if let Some(tc) = target_commodity {
        let budgeted =
            crate::ipc::decimal_to_amount(overview.summary.total_effective_target, tc.as_str())?;
        let actuals_in_target = overview
            .summary
            .total_actuals
            .iter()
            .find(|a| a.commodity() == tc)
            .map_or(bc_models::Decimal::ZERO, bc_models::Amount::value);
        let spent = crate::ipc::decimal_to_amount(actuals_in_target, tc.as_str())?;
        let remaining_val = overview
            .summary
            .total_effective_target
            .checked_sub(actuals_in_target)
            .unwrap_or(bc_models::Decimal::ZERO);
        let remaining = crate::ipc::decimal_to_amount(remaining_val, tc.as_str())?;
        (Some(budgeted), Some(spent), Some(remaining))
    } else {
        let spent = match overview.summary.total_actuals.as_slice() {
            [single] => Some(single.into_ipc()),
            _ => None,
        };
        (None, spent, None)
    };

    let has_mixed = overview.summary.total_actuals.len() > 1;
    let summary = bc_ipc::BudgetSummary::new(
        total_budgeted,
        total_spent,
        total_remaining,
        has_mixed,
        overview.summary.overspent_count,
    );

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
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::NativePeriodRow>, bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let budget = state
        .budgets
        .get(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let native = state
        .budget_tree
        .native_periods(&budget, display_start, display_end)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let commodity = budget
        .target()
        .map(|t| t.commodity().as_str().to_owned())
        .ok_or_else(|| {
            bc_ipc::BcError::Internal(
                "budget has no target — commodity required for amount conversion".to_owned(),
            )
        })?;

    native
        .iter()
        .map(|n| {
            let label = format_native_period_label(n);
            let effective_target = n
                .effective_target
                .map(|t| crate::ipc::decimal_to_amount(t, &commodity))
                .transpose()?;
            let spent = crate::ipc::decimal_to_amount(n.actuals, &commodity)?;
            Ok(bc_ipc::NativePeriodRow::new(
                label,
                n.overlap.native_start,
                n.overlap.native_end,
                effective_target,
                spent,
            ))
        })
        .collect()
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
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::Transaction>, bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let budget = state
        .budgets
        .get(&bid)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let txns = state
        .transactions
        .list_for_budget(&budget, period_start, period_end)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let account_map: std::collections::HashMap<String, &bc_models::Account> =
        accounts.iter().map(|a| (a.id().to_string(), a)).collect();

    Ok(txns
        .iter()
        .map(|t| crate::ipc::transaction_into_ipc_with_accounts(t, &account_map))
        .collect())
}

// MARK: Budget mutations

/// Updates mutable fields on a budget.
///
/// Pass `name = Some(None)` to clear the name, `name = None` to leave it unchanged.
/// Both `target_minor_units` and `target_currency` must be provided together, or both omitted.
/// Pass `tag_filter = Some(None)` to clear the tag filter, `tag_filter = None` to leave it unchanged.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the budget ID is invalid, the target fields are inconsistent,
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
pub async fn update_budget(
    budget_id: String,
    name: Option<Option<String>>,
    target_minor_units: Option<i64>,
    target_currency: Option<String>,
    rollover: Option<bc_ipc::RolloverPolicy>,
    period: Option<bc_ipc::Period>,
    tag_filter: Option<Option<String>>,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let bid = budget_id
        .parse::<bc_models::BudgetId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid budget_id: {e}")))?;

    let target: Option<Option<bc_models::Amount>> = match (target_minor_units, target_currency) {
        (Some(minor), Some(cur)) => {
            let value = rust_decimal::Decimal::new(minor, 2);
            Some(Some(bc_models::Amount::new(
                value,
                bc_models::CommodityCode::new(cur),
            )))
        }
        (None, None) => None,
        _ => {
            return Err(bc_ipc::BcError::Validation(
                "target_minor_units and target_currency must both be set or both null".to_owned(),
            ));
        }
    };

    let rollover_model = rollover.map(crate::ipc::IntoModel::into_model);
    let period_model = period.map(crate::ipc::IntoModel::into_model);

    let tag: Option<Option<bc_models::TagId>> = match tag_filter {
        None => None,
        Some(None) => Some(None),
        Some(Some(s)) => Some(Some(s.parse::<bc_models::TagId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid tag_filter: {e}"))
        })?)),
    };

    state
        .budgets
        .update(&bid, name, target, rollover_model, period_model, tag)
        .await
        .map(|_| ())
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
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
/// Both `target_minor_units` and `target_currency` must be provided together, or both omitted.
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
    name: Option<String>,
    target_minor_units: Option<i64>,
    target_currency: Option<String>,
    period: bc_ipc::Period,
    rollover: bc_ipc::RolloverPolicy,
    tag_filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let aid = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let target = match (target_minor_units, target_currency) {
        (Some(minor), Some(cur)) => {
            let value = rust_decimal::Decimal::new(minor, 2);
            Some(bc_models::Amount::new(
                value,
                bc_models::CommodityCode::new(cur),
            ))
        }
        (None, None) => None,
        _ => {
            return Err(bc_ipc::BcError::Validation(
                "target_minor_units and target_currency must both be set or both null".to_owned(),
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
        .maybe_name(name)
        .maybe_target(target)
        .period(period.into_model())
        .rollover(rollover.into_model())
        .maybe_tag_filter(tag)
        .call()
        .await
        .map(|_| ())
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
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
