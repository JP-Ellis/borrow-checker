//! Serialisable error types returned by Tauri commands.

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

/// Serialisable error type returned by all Tauri commands.
///
/// All variants carry `String` payloads so the type stays `Send + Sync` and
/// serialises cleanly across the IPC boundary without native-only error sources.
///
/// # Example
///
/// ```rust
/// use bc_ipc::BcError;
/// let e = BcError::NotFound("account-001".to_owned());
/// assert_eq!(e.to_string(), "not found: account-001");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[non_exhaustive]
#[expect(
    clippy::error_impl_error,
    reason = "named `Error` internally; re-exported as `BcError` at the crate root following workspace convention"
)]
pub enum Error {
    /// A requested resource could not be found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An argument or field failed validation.
    #[error("validation error: {0}")]
    Validation(String),

    /// The target changed since the client loaded it; the write was refused.
    #[error("conflict: {0}")]
    Conflict(String),

    /// An unexpected internal error occurred.
    #[error("internal error: {0}")]
    Internal(String),

    /// Query text did not parse or resolve.
    #[error("invalid query: {}", joined(.0))]
    Query(Vec<QueryProblem>),
}

/// One problem with query text: what is wrong, and the byte range
/// `[start, end)` of the text it concerns.
///
/// The range indexes the `query` of the request that failed. The stats,
/// sparkline and budget pages send a reprint with some top-level conjuncts
/// stripped, so slice that text, never the stored filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QueryProblem {
    /// What is wrong.
    pub message: String,
    /// Byte offset of the first character concerned.
    pub start: usize,
    /// Byte offset one past the last character concerned.
    pub end: usize,
}

impl QueryProblem {
    /// Creates a problem.
    ///
    /// # Arguments
    ///
    /// * `message` - What is wrong.
    /// * `start` - Byte offset of the first character concerned.
    /// * `end` - Byte offset one past the last.
    #[must_use]
    pub fn new(message: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            message: message.into(),
            start,
            end,
        }
    }
}

/// Joins problems' messages for [`Error::Query`]'s display.
fn joined(problems: &[QueryProblem]) -> String {
    problems
        .iter()
        .map(|p| p.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn not_found_display() {
        let e = Error::NotFound("account-001".to_owned());
        assert_eq!(e.to_string(), "not found: account-001");
    }

    #[test]
    fn validation_display() {
        let e = Error::Validation("amount must be positive".to_owned());
        assert_eq!(e.to_string(), "validation error: amount must be positive");
    }

    #[test]
    fn conflict_display() {
        let e = Error::Conflict("transaction tx-1 changed since it was opened".to_owned());
        assert_eq!(
            e.to_string(),
            "conflict: transaction tx-1 changed since it was opened"
        );
    }

    #[test]
    fn query_display_joins_messages() {
        let e = Error::Query(vec![
            QueryProblem::new("unknown field 'acount' (did you mean 'account'?)", 0, 6),
            QueryProblem::new("unclosed group", 9, 10),
        ]);
        assert_eq!(
            e.to_string(),
            "invalid query: unknown field 'acount' (did you mean 'account'?); unclosed group"
        );
    }

    #[test]
    fn serde_roundtrip_not_found() {
        let e = Error::NotFound("x".to_owned());
        let json = serde_json::to_string(&e).expect("serialises");
        let e2: Error = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(e, e2);
    }

    #[test]
    fn serde_roundtrip_internal() {
        let e = Error::Internal("db exploded".to_owned());
        let json = serde_json::to_string(&e).expect("serialises");
        let e2: Error = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(e, e2);
    }

    #[test]
    fn is_send_sync() {
        fn assert_send_sync<T>()
        where
            T: Send + Sync,
        {
        }
        assert_send_sync::<Error>();
    }
}
