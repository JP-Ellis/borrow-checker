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
    /// The transaction has already been voided and cannot be voided again.
    #[error("transaction already voided: {0}")]
    AlreadyVoided(TransactionId),
    /// A transaction's postings do not sum to zero per commodity.
    #[error("transaction postings are not balanced to zero")]
    UnbalancedTransaction,
    /// A value could not be parsed from its stored representation.
    #[error("data error: {0}")]
    BadData(String),
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
    fn unbalanced_transaction_error_displays() {
        let err = BcError::UnbalancedTransaction;
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn already_archived_error_displays_id() {
        let id = AccountId::new();
        let err = BcError::AlreadyArchived(id.clone());
        assert!(err.to_string().contains(&id.to_string()));
    }

    #[test]
    fn already_voided_error_displays_id() {
        let id = TransactionId::new();
        let err = BcError::AlreadyVoided(id.clone());
        assert!(err.to_string().contains(&id.to_string()));
    }
}
