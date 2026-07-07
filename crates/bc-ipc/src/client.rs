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
use crate::AuditEntry;
use crate::BackupInfo;
use crate::BackupSettings;
use crate::BcError;
use crate::BudgetRevisionView;
use crate::BudgetSummary;
use crate::BudgetTreeNode;
use crate::CommodityInfo;
use crate::EditTransaction;
use crate::Filter;
use crate::FilteredTransaction;
use crate::NativePeriodRow;
use crate::NewTransaction;
use crate::PluginInfo;
use crate::Reconciliation;
use crate::RolloverPolicy;
use crate::SettingsInfo;
use crate::TagInfo;
use crate::Transaction;
use crate::TransferSuggestion;
use crate::commands;
use crate::commands::ReverseTransactionArgs;
use crate::commands::SearchTransactionsArgs;

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
    /// Start of the date range (inclusive).
    date_from: jiff::civil::Date,
    /// End of the date range (exclusive).
    date_until: jiff::civil::Date,
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

/// Lists transactions for `account_id` within `[date_from, date_until)` from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_transactions(
    account_id: &str,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
) -> Result<Vec<Transaction>, BcError> {
    tauri_sys::core::invoke_result::<Vec<Transaction>, BcError>(
        commands::LIST_TRANSACTIONS,
        ListTransactionsArgs {
            account_id,
            date_from,
            date_until,
        },
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

/// Lists all tags from the backend as id/path pairs.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_tags() -> Result<Vec<TagInfo>, BcError> {
    tauri_sys::core::invoke_result::<Vec<TagInfo>, BcError>(commands::LIST_TAGS, NoArgs {}).await
}

/// Lists registered commodities/currencies.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn list_currencies() -> Result<Vec<CommodityInfo>, BcError> {
    tauri_sys::core::invoke_result::<Vec<CommodityInfo>, BcError>(
        commands::LIST_CURRENCIES,
        NoArgs {},
    )
    .await
}

/// Argument struct for [`create_currency`] / [`update_currency`].
#[derive(Serialize)]
struct CurrencyArgs<'a> {
    /// The commodity to persist.
    info: &'a CommodityInfo,
}

/// Creates a new commodity, returning the stored value.
///
/// # Errors
///
/// Returns [`BcError::Validation`] on a marker conflict, or [`BcError::Internal`]
/// if the invoke fails.
#[inline]
pub async fn create_currency(info: &CommodityInfo) -> Result<CommodityInfo, BcError> {
    tauri_sys::core::invoke_result::<CommodityInfo, BcError>(
        commands::CREATE_CURRENCY,
        CurrencyArgs { info },
    )
    .await
}

/// Updates an existing commodity (its code is immutable).
///
/// # Errors
///
/// Returns [`BcError::Validation`] on a marker conflict or code change, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn update_currency(info: &CommodityInfo) -> Result<(), BcError> {
    tauri_sys::core::invoke_result::<(), BcError>(commands::UPDATE_CURRENCY, CurrencyArgs { info })
        .await
}

/// Argument struct for [`delete_currency`].
#[derive(Serialize)]
struct DeleteCurrencyArgs<'a> {
    /// The commodity id to delete.
    id: &'a str,
}

/// Deletes a commodity, refusing if it is still referenced.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the commodity is referenced, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn delete_currency(id: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result::<(), BcError>(
        commands::DELETE_CURRENCY,
        DeleteCurrencyArgs { id },
    )
    .await
}

/// Argument struct for [`create_tag`].
#[derive(Serialize)]
struct CreateTagArgs<'a> {
    /// The colon-joined tag path to create.
    path: &'a str,
}

/// Creates the full colon-path tag hierarchy, returning the leaf tag ID string.
///
/// Existing ancestors are reused; only missing segments are created.
///
/// # Errors
///
/// Returns a [`BcError`] from the backend if the path is invalid, or
/// [`BcError::Internal`] if the Tauri invoke itself fails.
#[inline]
pub async fn create_tag(path: &str) -> Result<String, BcError> {
    tauri_sys::core::invoke_result::<String, BcError>(commands::CREATE_TAG, CreateTagArgs { path })
        .await
}

/// Argument struct for [`get_account_stats`].
#[derive(Serialize)]
struct GetAccountStatsArgs<'a> {
    /// Account ID to query.
    account_id: &'a str,
    /// Optional commodity code override.
    commodity: Option<&'a str>,
    /// Start of the date range (inclusive).
    date_from: jiff::civil::Date,
    /// End of the date range (exclusive).
    date_until: jiff::civil::Date,
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
    /// Reference date; the most recent bucket contains this date.
    as_of: jiff::civil::Date,
}

/// Gets windowed income, expense, and balance stats for `account_id`.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_stats(
    account_id: &str,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
) -> Result<crate::AccountStats, BcError> {
    tauri_sys::core::invoke_result::<crate::AccountStats, BcError>(
        commands::GET_ACCOUNT_STATS,
        GetAccountStatsArgs {
            account_id,
            commodity: None,
            date_from,
            date_until,
        },
    )
    .await
}

/// Argument struct for [`account_latest_activity`].
#[derive(Serialize)]
struct AccountLatestActivityArgs<'a> {
    /// Account ID to query.
    account_id: &'a str,
}

/// Returns the most recent transaction date for `account_id`, or `None`.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn account_latest_activity(
    account_id: &str,
) -> Result<Option<jiff::civil::Date>, BcError> {
    tauri_sys::core::invoke_result::<Option<jiff::civil::Date>, BcError>(
        commands::ACCOUNT_LATEST_ACTIVITY,
        AccountLatestActivityArgs { account_id },
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
/// * `as_of` - Reference date; the most recent bucket contains this date.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_sparkline(
    account_id: &str,
    period: crate::Period,
    count: u32,
    as_of: jiff::civil::Date,
) -> Result<Vec<crate::SparkPoint>, BcError> {
    tauri_sys::core::invoke_result::<Vec<crate::SparkPoint>, BcError>(
        commands::GET_ACCOUNT_SPARKLINE,
        GetAccountSparklineArgs {
            account_id,
            commodity: None,
            count,
            period,
            as_of,
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
    /// Start of the display window.
    period_start: jiff::civil::Date,
}

/// Arg struct for [`get_native_periods`].
#[derive(Serialize)]
struct GetNativePeriodsArgs<'a> {
    /// Budget ID to query.
    budget_id: &'a str,
    /// Start of the display window.
    display_start: jiff::civil::Date,
    /// End of the display window.
    display_end: jiff::civil::Date,
}

/// Arg struct for [`get_budget_transactions`].
#[derive(Serialize)]
struct GetBudgetTransactionsArgs<'a> {
    /// Budget ID to query.
    budget_id: &'a str,
    /// Start of the period.
    period_start: jiff::civil::Date,
    /// End of the period.
    period_end: jiff::civil::Date,
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
    /// Effective date for the budget's first revision.
    effective_from: jiff::civil::Date,
    /// Optional display name for the budget.
    name: Option<&'a str>,
    /// Optional target amount as an exact decimal (any precision).
    target: Option<rust_decimal::Decimal>,
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
    /// Start of the accrual spread.
    spread_from: jiff::civil::Date,
    /// End of the accrual spread.
    spread_until: jiff::civil::Date,
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
    period_start: jiff::civil::Date,
) -> Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError> {
    tauri_sys::core::invoke_result(
        commands::GET_BUDGET_OVERVIEW,
        GetBudgetOverviewArgs {
            period_type,
            period_start,
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
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
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
    period_start: jiff::civil::Date,
    period_end: jiff::civil::Date,
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

/// Archives a budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn archive_budget(budget_id: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(commands::ARCHIVE_BUDGET, ArchiveBudgetArgs { budget_id }).await
}

/// Arg struct for [`list_budget_revisions`].
#[derive(Serialize)]
struct ListBudgetRevisionsArgs<'a> {
    /// Budget whose revisions to list.
    budget_id: &'a str,
    /// Display window start (inclusive).
    display_start: jiff::civil::Date,
    /// Display window end (exclusive).
    display_end: jiff::civil::Date,
}

/// Arg struct for [`resolve_effective_date`].
#[derive(Serialize)]
struct ResolveEffectiveDateArgs<'a> {
    /// Budget providing the revision grid.
    budget_id: &'a str,
    /// Candidate effective date to snap.
    date: jiff::civil::Date,
    /// Revision id to exclude (the one being amended), or `None`.
    exclude_revision_id: Option<&'a str>,
}

/// Arg struct for [`revise_budget`].
#[derive(Serialize)]
struct ReviseBudgetArgs<'a> {
    /// Budget to revise.
    budget_id: &'a str,
    /// Existing revision id to amend, or `None` to add a new revision.
    revision_id: Option<&'a str>,
    /// Resolved (exact) effective date.
    effective_from: jiff::civil::Date,
    /// Display name, or `None` for the account-name fallback.
    name: Option<&'a str>,
    /// Target amount as an exact decimal, or `None` for tracking-only.
    target: Option<rust_decimal::Decimal>,
    /// Target currency code, paired with `target`.
    target_currency: Option<&'a str>,
    /// Rollover policy.
    rollover: RolloverPolicy,
    /// Recurrence period.
    period: crate::Period,
    /// Tag filter id, or `None`.
    tag_filter: Option<&'a str>,
}

/// Arg struct for [`remove_budget_revision`].
#[derive(Serialize)]
struct RemoveBudgetRevisionArgs<'a> {
    /// Budget owning the revision.
    budget_id: &'a str,
    /// Revision to remove.
    revision_id: &'a str,
}

/// Creates a new budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
#[expect(
    clippy::too_many_arguments,
    reason = "budget creation requires all revision fields; a builder struct would add more boilerplate than clarity here"
)]
pub async fn create_budget(
    account_id: &str,
    effective_from: jiff::civil::Date,
    name: Option<&str>,
    target: Option<rust_decimal::Decimal>,
    target_currency: Option<&str>,
    period: crate::Period,
    rollover: RolloverPolicy,
    tag_filter: Option<&str>,
) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::CREATE_BUDGET,
        CreateBudgetArgs {
            account_id,
            effective_from,
            name,
            target,
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
    spread_from: jiff::civil::Date,
    spread_until: jiff::civil::Date,
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

/// Lists a budget's revisions for a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn list_budget_revisions(
    budget_id: &str,
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
) -> Result<Vec<BudgetRevisionView>, BcError> {
    tauri_sys::core::invoke_result(
        commands::LIST_BUDGET_REVISIONS,
        ListBudgetRevisionsArgs {
            budget_id,
            display_start,
            display_end,
        },
    )
    .await
}

/// Resolves a snap effective date to the next grid boundary.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn resolve_effective_date(
    budget_id: &str,
    date: jiff::civil::Date,
    exclude_revision_id: Option<&str>,
) -> Result<jiff::civil::Date, BcError> {
    tauri_sys::core::invoke_result(
        commands::RESOLVE_EFFECTIVE_DATE,
        ResolveEffectiveDateArgs {
            budget_id,
            date,
            exclude_revision_id,
        },
    )
    .await
}

/// Adds or amends a budget revision.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[expect(
    clippy::too_many_arguments,
    reason = "IPC wrapper mirrors the Tauri command's flat argument list"
)]
#[inline]
pub async fn revise_budget(
    budget_id: &str,
    revision_id: Option<&str>,
    effective_from: jiff::civil::Date,
    name: Option<&str>,
    target: Option<rust_decimal::Decimal>,
    target_currency: Option<&str>,
    rollover: RolloverPolicy,
    period: crate::Period,
    tag_filter: Option<&str>,
) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::REVISE_BUDGET,
        ReviseBudgetArgs {
            budget_id,
            revision_id,
            effective_from,
            name,
            target,
            target_currency,
            rollover,
            period,
            tag_filter,
        },
    )
    .await
}

/// Removes a budget revision.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn remove_budget_revision(budget_id: &str, revision_id: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::REMOVE_BUDGET_REVISION,
        RemoveBudgetRevisionArgs {
            budget_id,
            revision_id,
        },
    )
    .await
}

/// Reverses a transaction by id, returning the new reversal transaction's id.
///
/// # Errors
///
/// Returns [`BcError`] if the id is invalid or the transaction does not exist.
#[inline]
pub async fn reverse_transaction(id: &str) -> Result<String, BcError> {
    tauri_sys::core::invoke_result::<String, BcError>(
        commands::REVERSE_TRANSACTION,
        ReverseTransactionArgs { id },
    )
    .await
}

/// Arg struct for [`edit_transaction`]. Must match the Tauri command param name.
#[derive(Serialize)]
struct EditTransactionArgs<'a> {
    /// The desired transaction state.
    tx: &'a EditTransaction,
}

/// Arg struct for [`get_transaction_audit`]. Must match the Tauri command param name.
#[derive(Serialize)]
struct GetTransactionAuditArgs<'a> {
    /// The transaction ID whose audit trail to load.
    id: &'a str,
}

/// Applies a desired transaction state via the backend edit command.
///
/// # Arguments
///
/// * `tx` - The desired transaction state.
///
/// # Errors
///
/// Returns [`BcError`] if the backend rejects or fails the edit.
#[inline]
pub async fn edit_transaction(tx: &EditTransaction) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(commands::EDIT_TRANSACTION, EditTransactionArgs { tx }).await
}

/// Arg struct for [`set_reconciliation`]. Must match the Tauri command param names.
#[derive(Serialize)]
struct SetReconciliationArgs<'a> {
    /// The transaction ID to update.
    id: &'a str,
    /// The desired reconciliation state.
    reconciliation: Reconciliation,
}

/// Sets a transaction's reconciliation state.
///
/// # Arguments
///
/// * `id` - The transaction ID.
/// * `state` - The desired reconciliation state.
///
/// # Errors
///
/// Returns [`BcError`] if the backend rejects (e.g. reconciling an
/// unbalanced transaction) or fails the update.
#[inline]
pub async fn set_reconciliation(id: &str, state: Reconciliation) -> Result<(), BcError> {
    tauri_sys::core::invoke_result(
        commands::SET_RECONCILIATION,
        SetReconciliationArgs {
            id,
            reconciliation: state,
        },
    )
    .await
}

/// Loads the audit trail for a transaction.
///
/// # Arguments
///
/// * `id` - The transaction's ID.
///
/// # Errors
///
/// Returns [`BcError`] if the backend lookup fails.
#[inline]
pub async fn get_transaction_audit(id: &str) -> Result<Vec<AuditEntry>, BcError> {
    tauri_sys::core::invoke_result(
        commands::GET_TRANSACTION_AUDIT,
        GetTransactionAuditArgs { id },
    )
    .await
}

/// Snapshots the database to the managed backup directory.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn backup_database() -> Result<BackupInfo, BcError> {
    tauri_sys::core::invoke_result::<BackupInfo, BcError>(commands::BACKUP_DATABASE, NoArgs {})
        .await
}

/// Lists existing backups, newest-first.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn list_backups() -> Result<Vec<BackupInfo>, BcError> {
    tauri_sys::core::invoke_result::<Vec<BackupInfo>, BcError>(commands::LIST_BACKUPS, NoArgs {})
        .await
}

/// Argument struct for [`restore_database`].
#[derive(Serialize)]
struct RestoreArgs<'a> {
    /// Path to the backup to restore.
    path: &'a str,
}

/// Restores the database from `path`; the backend relaunches the app on success.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the file is not a valid backup, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn restore_database(path: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result::<(), BcError>(commands::RESTORE_DATABASE, RestoreArgs { path })
        .await
}

/// Reads the current backup settings.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn get_backup_settings() -> Result<BackupSettings, BcError> {
    tauri_sys::core::invoke_result::<BackupSettings, BcError>(
        commands::GET_BACKUP_SETTINGS,
        NoArgs {},
    )
    .await
}

/// Argument struct for [`update_backup_settings`].
#[derive(Serialize)]
struct UpdateBackupSettingsArgs<'a> {
    /// The settings to persist.
    settings: &'a BackupSettings,
}

/// Persists updated backup settings to the config file.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn update_backup_settings(settings: &BackupSettings) -> Result<(), BcError> {
    tauri_sys::core::invoke_result::<(), BcError>(
        commands::UPDATE_BACKUP_SETTINGS,
        UpdateBackupSettingsArgs { settings },
    )
    .await
}

/// Proposes candidate transfer pairs for review.
///
/// # Errors
///
/// Returns [`BcError`] if the backend query fails.
#[inline]
pub async fn suggest_transfers() -> Result<Vec<TransferSuggestion>, BcError> {
    tauri_sys::core::invoke_result::<Vec<TransferSuggestion>, BcError>(
        commands::SUGGEST_TRANSFERS,
        NoArgs {},
    )
    .await
}

/// Argument struct for [`merge_transactions`]. Field names must match the
/// `merge_transactions` Tauri command parameters.
#[derive(Serialize)]
struct MergeArgs<'a> {
    /// The surviving (debit) transaction id.
    survivor: &'a str,
    /// The absorbed (credit) transaction id.
    absorbed: &'a str,
}

/// Merges `absorbed` into `survivor` (survivor is the debit leg).
///
/// # Errors
///
/// Returns [`BcError::Validation`] if an id is invalid or the pair is not
/// mergeable, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn merge_transactions(survivor: &str, absorbed: &str) -> Result<(), BcError> {
    tauri_sys::core::invoke_result::<(), BcError>(
        commands::MERGE_TRANSACTIONS,
        MergeArgs { survivor, absorbed },
    )
    .await
}

/// Argument struct for [`unmerge_transaction`]. Field name must match the
/// `unmerge_transaction` Tauri command parameter.
#[derive(Serialize)]
struct UnmergeArgs<'a> {
    /// The transaction whose most recent merge is reversed.
    transaction: &'a str,
}

/// Reverses the most recent merge on `transaction`, returning the restored id.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the id is invalid or there is no merge to
/// reverse, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn unmerge_transaction(transaction: &str) -> Result<String, BcError> {
    tauri_sys::core::invoke_result::<String, BcError>(
        commands::UNMERGE_TRANSACTION,
        UnmergeArgs { transaction },
    )
    .await
}

/// Runs a structured transaction search on the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn search_transactions(filter: &Filter) -> Result<Vec<FilteredTransaction>, BcError> {
    tauri_sys::core::invoke_result::<Vec<FilteredTransaction>, BcError>(
        commands::SEARCH_TRANSACTIONS,
        SearchTransactionsArgs { filter },
    )
    .await
}
