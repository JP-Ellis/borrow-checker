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
use crate::NewTransaction;
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
    /// Optional bucket count (default: 6).
    count: Option<u32>,
    /// Optional bucket period (default: Monthly).
    period: Option<&'a crate::SparklinePeriod>,
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

/// Gets period-bucketed cash-flow data for a sparkline, defaulting to 6 monthly buckets.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_sparkline(account_id: &str) -> Result<Vec<crate::SparkPoint>, BcError> {
    tauri_sys::core::invoke_result::<Vec<crate::SparkPoint>, BcError>(
        commands::GET_ACCOUNT_SPARKLINE,
        GetAccountSparklineArgs {
            account_id,
            commodity: None,
            count: None,
            period: None,
        },
    )
    .await
}
