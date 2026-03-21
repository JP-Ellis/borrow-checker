//! Core error types.

/// The result type used throughout `bc-core`.
pub type BcResult<T> = Result<T, BcError>;

/// Errors produced by the BorrowChecker core engine.
#[expect(
    clippy::module_name_repetitions,
    reason = "BcError is the canonical domain name regardless of module path"
)]
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum BcError {
    /// An entity with the given ID was not found.
    #[error("not found: {0}")]
    NotFound(String),
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
}
