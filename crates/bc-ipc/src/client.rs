//! WASM-only typed wrappers for Tauri IPC commands.
//!
//! Each function corresponds to a command registered in `bc-app`. The command
//! name strings come from [`crate::commands`] so a rename on either side is a
//! compile error, not a silent runtime mismatch.
//!
//! # Usage
//!
//! ```ignore
//! let accounts = bc_ipc::client::list_accounts().await?;
//! ```

use serde::Serialize;

use crate::AccountNode;
use crate::BcError;
use crate::BudgetSummary;
use crate::BudgetTreeNode;
use crate::NativePeriodRow;
use crate::NewTransaction;
use crate::PluginInfo;
use crate::RolloverPolicy;
use crate::SettingsInfo;
use crate::Transaction;
use crate::commands;

/// Empty args struct for commands that take no parameters.
///
/// Tauri 2 IPC requires args to serialise as a JSON Object (`{}`), not `null`.
/// A unit struct (`struct NoArgs;`) serialises to `null`; an empty record
/// struct (`struct NoArgs {}`) serialises to `{}` as required.
#[derive(Serialize)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "empty braces are intentional: serde serialises `struct S {}` as `{}` but `struct S;` as `null`"
)]
struct NoArgs {}

/// Argument struct for [`list_transactions`]. Must match the Tauri command param name.
#[derive(Serialize)]
struct ListTransactionsArgs<'a> {
    /// The account ID to query transactions for.
    account_id: &'a str,
}

/// Argument struct for [`create_transaction`]. Must match the Tauri command param name.
#[derive(Serialize)]
struct CreateTransactionArgs<'a> {
    /// The new transaction data to create.
    tx: &'a NewTransaction,
}

/// Lists all active accounts from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_accounts() -> Result<Vec<AccountNode>, BcError> {
    tauri_sys::core::invoke_result::<Vec<AccountNode>, BcError>(commands::LIST_ACCOUNTS, NoArgs {})
        .await
}

/// Lists transactions for `account_id` from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_transactions(account_id: &str) -> Result<Vec<Transaction>, BcError> {
    tauri_sys::core::invoke_result::<Vec<Transaction>, BcError>(
        commands::LIST_TRANSACTIONS,
        ListTransactionsArgs { account_id },
    )
    .await
}

/// Creates a new transaction and returns its assigned ID string.
///
/// # Errors
///
/// Returns a [`BcError`] from the backend if validation fails, or
/// [`BcError::Internal`] if the Tauri invoke itself fails.
#[inline]
pub async fn create_transaction(tx: &NewTransaction) -> Result<String, BcError> {
    tauri_sys::core::invoke_result::<String, BcError>(
        commands::CREATE_TRANSACTION,
        CreateTransactionArgs { tx },
    )
    .await
}

/// Argument struct for [`get_account_stats`].
#[derive(Serialize)]
struct GetAccountStatsArgs<'a> {
    /// Account ID to query.
    account_id: &'a str,
    /// Optional commodity code override.
    commodity: Option<&'a str>,
}

/// Argument struct for [`get_account_sparkline`].
#[derive(Serialize)]
struct GetAccountSparklineArgs<'a> {
    /// Account ID to query.
    account_id: &'a str,
    /// Optional commodity code override.
    commodity: Option<&'a str>,
    /// Number of buckets to return.
    count: u32,
    /// Time-bucket granularity.
    period: crate::Period,
}

/// Gets income and expense totals for `account_id` over the last 30 days.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_stats(account_id: &str) -> Result<crate::AccountStats, BcError> {
    tauri_sys::core::invoke_result::<crate::AccountStats, BcError>(
        commands::GET_ACCOUNT_STATS,
        GetAccountStatsArgs {
            account_id,
            commodity: None,
        },
    )
    .await
}

/// Argument struct for [`get_posting_count`].
#[derive(Serialize)]
struct GetPostingCountArgs<'a> {
    /// Account ID to query.
    account_id: &'a str,
}

/// Returns the number of non-voided postings for `account_id`.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_posting_count(account_id: &str) -> Result<u32, BcError> {
    tauri_sys::core::invoke_result::<u32, BcError>(
        commands::GET_POSTING_COUNT,
        GetPostingCountArgs { account_id },
    )
    .await
}

/// Gets period-bucketed cash-flow data for a sparkline.
///
/// # Arguments
///
/// * `account_id` - Account ID to query.
/// * `period` - Time-bucket granularity.
/// * `count` - Number of buckets to return.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_sparkline(
    account_id: &str,
    period: crate::Period,
    count: u32,
) -> Result<Vec<crate::SparkPoint>, BcError> {
    tauri_sys::core::invoke_result::<Vec<crate::SparkPoint>, BcError>(
        commands::GET_ACCOUNT_SPARKLINE,
        GetAccountSparklineArgs {
            account_id,
            commodity: None,
            count,
            period,
        },
    )
    .await
}

/// Lists all installed plugins from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_plugins() -> Result<Vec<PluginInfo>, BcError> {
    tauri_sys::core::invoke_result::<Vec<PluginInfo>, BcError>(commands::LIST_PLUGINS, NoArgs {})
        .await
}

/// Gets the current application settings from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails or the config
/// cannot be loaded.
#[inline]
pub async fn get_settings() -> Result<SettingsInfo, BcError> {
    tauri_sys::core::invoke_result::<SettingsInfo, BcError>(commands::GET_SETTINGS, NoArgs {}).await
}

/// Arg struct for [`get_budget_overview`].
#[derive(Serialize)]
struct GetBudgetOverviewArgs {
    /// Display period granularity.
    period_type: crate::Period,
    /// ISO-8601 date string for the start of the display window.
    period_start: String,
}

/// Arg struct for [`get_native_periods`].
#[derive(Serialize)]
struct GetNativePeriodsArgs<'a> {
    /// Budget ID to query.
    budget_id: &'a str,
    /// ISO-8601 date string for the start of the display window.
    display_start: &'a str,
    /// ISO-8601 date string for the end of the display window.
    display_end: &'a str,
}

/// Arg struct for [`get_budget_transactions`].
#[derive(Serialize)]
struct GetBudgetTransactionsArgs<'a> {
    /// Budget ID to query.
    budget_id: &'a str,
    /// ISO-8601 date string for the start of the period.
    period_start: &'a str,
    /// ISO-8601 date string for the end of the period.
    period_end: &'a str,
}

/// Arg struct for [`update_budget`].
#[derive(Serialize)]
struct UpdateBudgetArgs<'a> {
    /// Budget ID to update.
    budget_id: &'a str,
    /// New name: `Some(Some(s))` sets name, `Some(None)` clears it, `None` leaves it unchanged.
    #[expect(
        clippy::option_option,
        reason = "outer Some = patch; inner None = clear the field"
    )]
    name: Option<Option<String>>,
    /// New target amount in minor currency units, or `None` to leave unchanged.
    target_minor_units: Option<i64>,
    /// New target currency code, or `None` to leave unchanged.
    target_currency: Option<&'a str>,
    /// New rollover policy, or `None` to leave unchanged.
    rollover: Option<RolloverPolicy>,
}

/// Arg struct for [`archive_budget`].
#[derive(Serialize)]
struct ArchiveBudgetArgs<'a> {
    /// Budget ID to archive.
    budget_id: &'a str,
}

/// Arg struct for [`create_budget`].
#[derive(Serialize)]
struct CreateBudgetArgs<'a> {
    /// Account ID to attach the budget to.
    account_id: &'a str,
    /// Optional display name for the budget.
    name: Option<&'a str>,
    /// Optional target amount in minor currency units.
    target_minor_units: Option<i64>,
    /// Optional target currency code.
    target_currency: Option<&'a str>,
    /// Budget period granularity.
    period: crate::Period,
    /// Rollover policy for unused budget amounts.
    rollover: RolloverPolicy,
    /// Optional tag filter expression.
    tag_filter: Option<&'a str>,
}

/// Arg struct for [`set_posting_spread`].
#[derive(Serialize)]
struct SetPostingSpreadArgs<'a> {
    /// Posting ID to update.
    posting_id: &'a str,
    /// ISO-8601 date string for the start of the accrual spread.
    spread_from: &'a str,
    /// ISO-8601 date string for the end of the accrual spread.
    spread_until: &'a str,
}

/// Arg struct for [`clear_posting_spread`].
#[derive(Serialize)]
struct ClearPostingSpreadArgs<'a> {
    /// Posting ID whose spread should be removed.
    posting_id: &'a str,
}

/// Gets the budget overview (summary + tree) for a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_budget_overview(
    period_type: crate::Period,
    period_start: &str,
) -> Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError> {
    tauri_sys::core::invoke_result(
        commands::GET_BUDGET_OVERVIEW,
        GetBudgetOverviewArgs {
            period_type,
            period_start: period_start.to_owned(),
        },
    )
    .await
}

/// Gets native period sub-rows for one budget in a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_native_periods(
    budget_id: &str,
    display_start: &str,
    display_end: &str,
) -> Result<Vec<NativePeriodRow>, BcError> {
    tauri_sys::core::invoke_result(
        commands::GET_NATIVE_PERIODS,
        GetNativePeriodsArgs {
            budget_id,
            display_start,
            display_end,
        },
    )
    .await
}

/// Gets transactions matched by a budget in a date range.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_budget_transactions(
    budget_id: &str,
    period_start: &str,
    period_end: &str,
) -> Result<Vec<Transaction>, BcError> {
    tauri_sys::core::invoke_result(
        commands::GET_BUDGET_TRANSACTIONS,
        GetBudgetTransactionsArgs {
            budget_id,
            period_start,
            period_end,
        },
    )
    .await
}

/// Updates mutable fields on a budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn update_budget(
    budget_id: &str,
    name: Option<Option<String>>,
    target_minor_units: Option<i64>,
    target_currency: Option<&str>,
    rollover: Option<RolloverPolicy>,
) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::UPDATE_BUDGET,
        UpdateBudgetArgs {
            budget_id,
            name,
            target_minor_units,
            target_currency,
            rollover,
        },
    )
    .await
}

/// Archives a budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn archive_budget(budget_id: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(commands::ARCHIVE_BUDGET, ArchiveBudgetArgs { budget_id }).await
}

/// Creates a new budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn create_budget(
    account_id: &str,
    name: Option<&str>,
    target_minor_units: Option<i64>,
    target_currency: Option<&str>,
    period: crate::Period,
    rollover: RolloverPolicy,
    tag_filter: Option<&str>,
) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::CREATE_BUDGET,
        CreateBudgetArgs {
            account_id,
            name,
            target_minor_units,
            target_currency,
            period,
            rollover,
            tag_filter,
        },
    )
    .await
}

/// Sets the accrual spread on a posting.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn set_posting_spread(
    posting_id: &str,
    spread_from: &str,
    spread_until: &str,
) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::SET_POSTING_SPREAD,
        SetPostingSpreadArgs {
            posting_id,
            spread_from,
            spread_until,
        },
    )
    .await
}

/// Clears the accrual spread from a posting.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn clear_posting_spread(posting_id: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::CLEAR_POSTING_SPREAD,
        ClearPostingSpreadArgs { posting_id },
    )
    .await
}
