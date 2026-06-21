//! Tauri command handlers for tag lifecycle operations.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

use crate::AppState;

/// Creates a tag hierarchy from a colon-path, returning the leaf tag ID.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed path, or
/// [`bc_ipc::BcError::Internal`] on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_tag(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, bc_ipc::BcError> {
    let parsed = path
        .parse::<bc_models::TagPath>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag path '{path}': {e}")))?;
    let id = state
        .tags
        .create_path(&parsed)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(id.to_string())
}

/// Renames a tag's leaf segment.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a bad ID or a sibling collision,
/// [`bc_ipc::BcError::Internal`] on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn rename_tag(
    id: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let tag_id = id
        .parse::<bc_models::TagId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag id '{id}': {e}")))?;
    state
        .tags
        .rename(&tag_id, &new_name)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Deletes a tag and its subtree (cascading memberships).
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a bad ID, [`bc_ipc::BcError::Internal`]
/// on a service failure (including the tag being a budget filter).
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_tag(id: String, state: State<'_, AppState>) -> Result<(), bc_ipc::BcError> {
    let tag_id = id
        .parse::<bc_models::TagId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag id '{id}': {e}")))?;
    state
        .tags
        .delete(&tag_id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Lists every tag as `(id, resolved-path)` pairs for typeahead/selection UIs.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] on a service failure.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_tags(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::TagInfo>, bc_ipc::BcError> {
    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let tags = state
        .tags
        .list()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(tags
        .iter()
        .filter_map(|t| {
            forest
                .path_of(t.id())
                .map(|p| bc_ipc::TagInfo::new(t.id().to_string(), p.to_string()))
        })
        .collect())
}
