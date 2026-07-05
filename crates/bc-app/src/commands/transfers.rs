//! Tauri command handlers for transfer resolution (merge/unmerge/suggest).
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use core::str::FromStr as _;

use tauri::State;

use crate::AppState;

/// Merges `absorbed` into `survivor`.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if either id fails to parse,
/// or [`bc_ipc::BcError`] if the pair is not mergeable or the database
/// write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn merge_transactions(
    survivor: String,
    absorbed: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let survivor_id = bc_models::TransactionId::from_str(&survivor).map_err(|e| {
        bc_ipc::BcError::Validation(format!("invalid transaction id '{survivor}': {e}"))
    })?;
    let absorbed_id = bc_models::TransactionId::from_str(&absorbed).map_err(|e| {
        bc_ipc::BcError::Validation(format!("invalid transaction id '{absorbed}': {e}"))
    })?;
    state.transfers.merge(&survivor_id, &absorbed_id).await?;
    Ok(())
}

/// Reverses the most recent merge on `transaction`, returning the restored id.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id fails to parse,
/// or [`bc_ipc::BcError`] if no merge record exists or the database
/// write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn unmerge_transaction(
    transaction: String,
    state: State<'_, AppState>,
) -> Result<String, bc_ipc::BcError> {
    let tx_id = bc_models::TransactionId::from_str(&transaction).map_err(|e| {
        bc_ipc::BcError::Validation(format!("invalid transaction id '{transaction}': {e}"))
    })?;
    let restored = state.transfers.unmerge(&tx_id).await?;
    Ok(restored.to_string())
}

/// Returns proposed transfer pairs for review.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the database query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn suggest_transfers(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::TransferSuggestion>, bc_ipc::BcError> {
    let suggestions = state.transfers.suggest_transfers().await?;
    Ok(suggestions
        .iter()
        .map(bc_ipc::TransferSuggestion::from)
        .collect())
}
