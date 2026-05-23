//! Tauri command handlers for account and transaction operations (stub — implemented in Task 5).
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates state-binding code that triggers this lint on stub fns"
)]
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]

use tauri::State;

use crate::AppState;

/// List all accounts as a tree of nodes.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_accounts(
    _state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::AccountNode>, bc_ipc::BcError> {
    Ok(vec![])
}

/// List transactions for the given account.
///
/// # Arguments
///
/// * `_account_id` - The account ID to filter by.
/// * `_state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_transactions(
    _account_id: String,
    _state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::Transaction>, bc_ipc::BcError> {
    Ok(vec![])
}

/// Create a new transaction.
///
/// # Arguments
///
/// * `_tx` - The new transaction data.
/// * `_state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_transaction(
    _tx: bc_ipc::NewTransaction,
    _state: State<'_, AppState>,
) -> Result<String, bc_ipc::BcError> {
    Ok(String::new())
}
