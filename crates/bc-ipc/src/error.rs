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

    /// An unexpected internal error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
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
