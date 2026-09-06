//! Core error types.

use bc_models::AccountId;
use bc_models::TransactionId;

/// The result type used throughout `bc-core`.
pub type BcResult<T> = Result<T, BcError>;

/// Errors produced by the BorrowChecker core engine.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum BcError {
    /// An entity with the given ID was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// The account has already been archived and cannot be archived again.
    #[error("account already archived: {0}")]
    AlreadyArchived(AccountId),
    /// The account has already been closed and cannot be closed again.
    #[error("account already closed: {0}")]
    AlreadyClosed(AccountId),
    /// The account is not closed and cannot be reopened.
    #[error("account is not closed: {0}")]
    NotClosed(AccountId),
    /// An operation is not valid for the given account kind.
    #[error("invalid account kind for {operation}: account {account_id} is {kind:?}")]
    InvalidAccountKind {
        /// The operation that was attempted.
        operation: &'static str,
        /// The account that was rejected.
        account_id: bc_models::AccountId,
        /// The kind that was found.
        kind: bc_models::AccountKind,
    },
    /// A supplied parameter violates a business rule.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A value could not be parsed from its stored representation.
    #[error("data error: {0}")]
    BadData(String),
    /// A tag could not be deleted because it is still referenced by a budget filter.
    #[error("tag in use: {0}")]
    TagInUse(String),
    /// A commodity marker (code, symbol, or alias) collides with another commodity.
    #[error("marker conflict: '{marker}' already maps to {existing}")]
    MarkerConflict {
        /// The colliding marker string.
        marker: String,
        /// The canonical code of the commodity that already owns the marker.
        existing: String,
    },
    /// A commodity could not be deleted because it is still referenced.
    #[error("commodity in use: {0}")]
    CommodityInUse(String),
    /// A commodity code was empty or contained only whitespace.
    #[error("commodity code must not be empty or blank")]
    EmptyCommodityCode,
    /// Two transactions cannot be merged (bad sign, magnitude, commodity, or posting count).
    #[error("not mergeable: {reason}")]
    NotMergeable {
        /// Human-readable reason the merge was rejected.
        reason: String,
    },
    /// A transaction has no merge history to reverse.
    #[error("not merged: {0}")]
    NotMerged(TransactionId),
    /// A database error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// A JSON serialisation or deserialisation error.
    #[error("serialisation error: {0}")]
    Serialisation(#[from] serde_json::Error),
    /// A database migration error.
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_error_displays_id() {
        let err = BcError::NotFound("account_01j".to_owned());
        assert!(err.to_string().contains("account_01j"));
    }

    #[test]
    fn invalid_input_error_displays() {
        let err = BcError::InvalidInput("bad param".to_owned());
        assert!(err.to_string().contains("bad param"));
    }

    #[test]
    fn already_archived_error_displays_id() {
        let id = AccountId::new();
        let err = BcError::AlreadyArchived(id.clone());
        assert!(err.to_string().contains(&id.to_string()));
    }
}
