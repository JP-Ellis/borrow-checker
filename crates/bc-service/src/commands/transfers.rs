//! Command handlers for transfer resolution (merge/unmerge/suggest).
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use core::str::FromStr as _;

use crate::AppState;

/// Merges `absorbed` into `survivor`.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if either id fails to parse,
/// or [`bc_ipc::BcError`] if the pair is not mergeable or the database
/// write fails.
pub async fn merge_transactions(
    state: &AppState,
    args: bc_ipc::commands::MergeTransactionsArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::MergeTransactionsArgs {
        survivor, absorbed, ..
    } = args;
    let survivor_id = bc_models::TransactionId::from_str(&survivor).map_err(|e| {
        bc_ipc::BcError::Validation(format!("invalid transaction id '{survivor}': {e}"))
    })?;
    let absorbed_id = bc_models::TransactionId::from_str(&absorbed).map_err(|e| {
        bc_ipc::BcError::Validation(format!("invalid transaction id '{absorbed}': {e}"))
    })?;
    let warned = state.transfers.merge(&survivor_id, &absorbed_id).await?;
    for warning in &warned.warnings {
        tracing::warn!(%warning, "merge produced a warning");
    }
    Ok(())
}

/// Reverses the most recent merge on `transaction`, returning the restored id.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id fails to parse,
/// or [`bc_ipc::BcError`] if no merge record exists or the database
/// write fails.
pub async fn unmerge_transaction(
    state: &AppState,
    args: bc_ipc::commands::UnmergeTransactionArgs,
) -> Result<String, bc_ipc::BcError> {
    let bc_ipc::commands::UnmergeTransactionArgs { transaction, .. } = args;
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
pub async fn suggest_transfers(
    state: &AppState,
) -> Result<Vec<bc_ipc::TransferSuggestion>, bc_ipc::BcError> {
    let suggestions = state.transfers.suggest_transfers().await?;
    Ok(suggestions
        .iter()
        .map(bc_ipc::TransferSuggestion::from)
        .collect())
}
