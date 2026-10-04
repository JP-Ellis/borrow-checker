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
