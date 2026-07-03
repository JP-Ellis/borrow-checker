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
        .list_all()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(list.iter().map(bc_ipc::CommodityInfo::from).collect())
}

/// Creates a new commodity/currency.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id or a marker is invalid,
/// or [`bc_ipc::BcError::Internal`] if the database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_currency(
    info: bc_ipc::CommodityInfo,
    state: State<'_, AppState>,
) -> Result<bc_ipc::CommodityInfo, bc_ipc::BcError> {
    let c = bc_models::Commodity::try_from(info)?;
    let stored = state.commodities.create(&c).await?;
    Ok(bc_ipc::CommodityInfo::from(&stored))
}

/// Updates an existing commodity/currency (its code is immutable).
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id or a marker is invalid,
/// or [`bc_ipc::BcError::Internal`] if the database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn update_currency(
    info: bc_ipc::CommodityInfo,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let c = bc_models::Commodity::try_from(info)?;
    Ok(state.commodities.update(&c).await?)
}

/// Deletes a commodity/currency, refusing if it is still referenced.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id is invalid or the
/// commodity is still referenced, or [`bc_ipc::BcError::Internal`] if the
/// database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_currency(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let cid = id
        .parse::<bc_models::CommodityId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid commodity id '{id}': {e}")))?;
    Ok(state.commodities.delete(&cid).await?)
}
