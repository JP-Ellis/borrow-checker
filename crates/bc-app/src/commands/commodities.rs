//! Tauri command handlers for commodity/currency queries.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

use crate::AppState;

/// Lists all active commodities/currencies for the UI.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the database query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_currencies(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::CommodityInfo>, bc_ipc::BcError> {
    let list = state
        .commodities
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(list
        .into_iter()
        .map(|c| {
            bc_ipc::CommodityInfo::new(
                c.id().to_string(),
                c.code().to_owned(),
                c.symbol().map(ToOwned::to_owned),
                c.aliases().to_vec(),
            )
        })
        .collect())
}
