//! Command handlers for query text.

use crate::AppState;

/// Returns every fact query text resolves against, read fresh from the
/// database.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the catalog cannot be loaded.
#[expect(
    clippy::module_name_repetitions,
    reason = "the function is named after its command, `query_catalog`"
)]
pub async fn query_catalog(state: &AppState) -> Result<bc_ipc::QueryCatalog, bc_ipc::BcError> {
    Ok(state.transactions.query_catalog().await?)
}

/// Returns the stored values of a text metadata key that contain a needle,
/// prefix matches first, then the most used.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the values cannot be read.
pub async fn metadata_values(
    state: &AppState,
    args: bc_ipc::commands::MetadataValuesArgs,
) -> Result<Vec<bc_ipc::MetaValueCount>, bc_ipc::BcError> {
    let bc_ipc::commands::MetadataValuesArgs {
        key, needle, limit, ..
    } = args;
    let values = state
        .transactions
        .metadata_values(&key, &needle, limit)
        .await?;
    Ok(values
        .into_iter()
        .map(|(value, count)| bc_ipc::MetaValueCount::new(value, count))
        .collect())
}
