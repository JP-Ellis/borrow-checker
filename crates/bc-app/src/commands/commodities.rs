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
    Ok(list.iter().map(from_commodity).collect())
}

/// Builds an IPC `CommodityInfo` from a `bc_models::Commodity`.
///
/// The reverse of [`to_commodity`]. Kept as a free function rather than a
/// `From` impl because neither type is local to this crate (orphan rule) and
/// `bc-ipc` deliberately does not depend on `bc-models`.
fn from_commodity(c: &bc_models::Commodity) -> bc_ipc::CommodityInfo {
    bc_ipc::CommodityInfo::new(
        c.id().to_string(),
        c.code().to_owned(),
        c.symbol().map(ToOwned::to_owned),
        c.aliases().to_vec(),
        c.decimals(),
        c.is_iso(),
        c.symbol_after(),
    )
}

/// Maps a `CommodityService` error to the IPC error surfaced to the UI.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "bc_core::BcError is #[non_exhaustive] and covers many unrelated failure modes (database, migration, serialisation, etc.) that all collapse to Internal; enumerating them individually adds no value"
)]
fn map_err(e: bc_core::BcError) -> bc_ipc::BcError {
    match e {
        bc_core::BcError::MarkerConflict { marker, existing } => {
            bc_ipc::BcError::Validation(format!("'{marker}' already maps to {existing}"))
        }
        bc_core::BcError::CommodityInUse(msg) => {
            bc_ipc::BcError::Validation(format!("cannot delete: {msg}"))
        }
        bc_core::BcError::InvalidInput(msg) => bc_ipc::BcError::Validation(msg),
        bc_core::BcError::NotFound(id) => bc_ipc::BcError::NotFound(id),
        other => bc_ipc::BcError::Internal(other.to_string()),
    }
}

/// Builds a `bc_models::Commodity` from an IPC `CommodityInfo`. A blank id yields
/// a fresh commodity (create); a populated id round-trips (update).
fn to_commodity(info: bc_ipc::CommodityInfo) -> Result<bc_models::Commodity, bc_ipc::BcError> {
    let id = if info.id.is_empty() {
        None
    } else {
        Some(
            info.id
                .parse::<bc_models::CommodityId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid commodity id: {e}")))?,
        )
    };
    Ok(bc_models::Commodity::builder()
        .code(info.code)
        .aliases(info.aliases)
        .decimals(info.decimals)
        .is_iso(info.is_iso)
        .symbol_after(info.symbol_after)
        .maybe_symbol(info.symbol)
        .maybe_id(id)
        .build())
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
    let c = to_commodity(info)?;
    let stored = state.commodities.create(&c).await.map_err(map_err)?;
    Ok(from_commodity(&stored))
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
    let c = to_commodity(info)?;
    state.commodities.update(&c).await.map_err(map_err)
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
    state.commodities.delete(&cid).await.map_err(map_err)
}
