//! Transaction delete and import-provenance commands.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use bc_core::ipc::RejectedRowExt as _;
use bc_core::ipc::TransactionProvenanceExt as _;
use bc_models::AccountId;

use crate::AppState;

/// Widens a count for the wire.
fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Parses an account id argument.
fn parse_account_id(raw: &str) -> Result<AccountId, bc_ipc::BcError> {
    raw.parse::<AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account id: {e}")))
}

/// Deletes a transaction, keeping or forgetting its import provenance.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable id and
/// [`bc_ipc::BcError::NotFound`] for an unknown transaction.
pub async fn delete_transaction(
    state: &AppState,
    args: bc_ipc::commands::DeleteTransactionArgs,
) -> Result<bc_ipc::DeleteOutcome, bc_ipc::BcError> {
    let bc_ipc::commands::DeleteTransactionArgs {
        id,
        forget_provenance,
        ..
    } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;
    let mode = if forget_provenance {
        bc_core::DeleteMode::ForgetProvenance
    } else {
        bc_core::DeleteMode::KeepProvenance
    };
    let outcome = state.transactions.delete(&tx_id, mode).await?;
    Ok(bc_ipc::DeleteOutcome::new(
        to_u64(outcome.references_kept),
        to_u64(outcome.references_forgotten),
    ))
}

/// Summarises the import provenance a transaction carries, naming each
/// account by its path.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable id.
pub async fn transaction_provenance(
    state: &AppState,
    args: bc_ipc::commands::TransactionProvenanceArgs,
) -> Result<bc_ipc::TransactionProvenance, bc_ipc::BcError> {
    let bc_ipc::commands::TransactionProvenanceArgs { id, .. } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;
    let summary = state.sources.summary(&tx_id).await?;
    let resolver = bc_core::AccountResolver::load(&state.accounts).await?;
    Ok(bc_ipc::TransactionProvenance::from_summary(
        &summary, &resolver,
    ))
}

/// Lists rejected statement rows, optionally for one account, naming each
/// account by its path.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable account id.
pub async fn list_rejected_sources(
    state: &AppState,
    args: bc_ipc::commands::ListRejectedSourcesArgs,
) -> Result<Vec<bc_ipc::RejectedRow>, bc_ipc::BcError> {
    let bc_ipc::commands::ListRejectedSourcesArgs { account, .. } = args;
    let account_id = account.as_deref().map(parse_account_id).transpose()?;
    let rows = state.sources.rejected(account_id.as_ref()).await?;
    let resolver = bc_core::AccountResolver::load(&state.accounts).await?;
    Ok(rows
        .iter()
        .map(|row| bc_ipc::RejectedRow::from_core(row, &resolver))
        .collect())
}

/// Releases rejected statement rows, returning how many references were released.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a target that parses as
/// neither id, and for a reference that still backs a live posting or names
/// a leg of a deleted transaction. Returns [`bc_ipc::BcError::NotFound`] for
/// an unknown reference and for a transaction id with no rejected rows,
/// including a live transaction's id. No target is released on error.
pub async fn release_rejected_sources(
    state: &AppState,
    args: bc_ipc::commands::ReleaseRejectedSourcesArgs,
) -> Result<u64, bc_ipc::BcError> {
    let bc_ipc::commands::ReleaseRejectedSourcesArgs { targets, .. } = args;
    let parsed = targets
        .iter()
        .map(|raw| raw.parse::<bc_core::ReleaseTarget>())
        .collect::<Result<Vec<_>, _>>()?;
    let released = state.sources.release(&parsed).await?;
    Ok(to_u64(released))
}
