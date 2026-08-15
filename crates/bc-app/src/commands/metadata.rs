//! Tauri command handlers for the global metadata key registry.
//!
//! Keys enter the registry implicitly, on the first write of a value under
//! them, so there is no create command here — only the three operations a user
//! performs afterwards.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few
//! lints on the `State<'_, AppState>` parameter; these are suppressed
//! module-wide since item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

use crate::AppState;

/// Parses a metadata key, naming the offending text on failure.
///
/// # Arguments
///
/// * `key` - The raw key as it arrived over IPC.
///
/// # Returns
///
/// The validated, lowercased key.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] when the key breaks the charset,
/// leading-character or length rules.
fn parse_key(key: &str) -> Result<bc_models::MetaKey, bc_ipc::BcError> {
    bc_models::MetaKey::new(key)
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid metadata key '{key}': {e}")))
}

/// Lists every registered metadata key with its type, ordered by key.
///
/// The response is a whole-registry snapshot: key counts stay small, and the
/// registry changes only through a first write or the two commands below.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_metadata_keys(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::MetaKeyDefDto>, bc_ipc::BcError> {
    let defs = state
        .metadata
        .list()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(defs.iter().map(bc_ipc::MetaKeyDefDto::from).collect())
}

/// Changes a key's registered type, returning the type it had before.
///
/// Every stored value under the key is re-asserted against the new type:
/// widening to `text` is a relabel, and narrowing flags whatever will not
/// parse. Returning the previous type spares the caller a second query.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an invalid or unregistered key,
/// or [`bc_ipc::BcError::Internal`] on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn retype_metadata_key(
    key: String,
    ty: bc_ipc::MetaTypeDto,
    state: State<'_, AppState>,
) -> Result<bc_ipc::MetaTypeDto, bc_ipc::BcError> {
    let parsed = parse_key(&key)?;
    let from = state
        .metadata
        .retype(&parsed, bc_models::MetaType::from(ty))
        .await?;
    Ok(bc_ipc::MetaTypeDto::from(from))
}

/// Renames a key, carrying its entries with it.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an invalid name, an unregistered
/// source, or a target that is already registered; [`bc_ipc::BcError::Internal`]
/// on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn rename_metadata_key(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let parsed_from = parse_key(&from)?;
    let parsed_to = parse_key(&to)?;
    Ok(state.metadata.rename(&parsed_from, &parsed_to).await?)
}
