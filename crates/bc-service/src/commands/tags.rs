//! Command handlers for tag lifecycle operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use crate::AppState;

/// Creates a tag hierarchy from a colon-path, returning the leaf tag ID.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed path, or
/// [`bc_ipc::BcError::Internal`] on a service failure.
pub async fn create_tag(
    state: &AppState,
    args: bc_ipc::commands::CreateTagArgs,
) -> Result<String, bc_ipc::BcError> {
    let bc_ipc::commands::CreateTagArgs { path, .. } = args;
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
pub async fn rename_tag(
    state: &AppState,
    args: bc_ipc::commands::RenameTagArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::RenameTagArgs { id, new_name, .. } = args;
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
pub async fn delete_tag(
    state: &AppState,
    args: bc_ipc::commands::DeleteTagArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::DeleteTagArgs { id, .. } = args;
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
pub async fn list_tags(state: &AppState) -> Result<Vec<bc_ipc::TagInfo>, bc_ipc::BcError> {
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
