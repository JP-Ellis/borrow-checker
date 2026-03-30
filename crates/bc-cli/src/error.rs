//! CLI-level error and result types.

/// Result alias for all CLI operations.
pub type CliResult<T> = Result<T, CliError>;

/// Top-level error type for the `borrow-checker` CLI.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// An error propagated from the core engine.
    #[error("{0}")]
    Core(#[from] bc_core::BcError),

    /// An I/O error (file reading/writing, stdout).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialisation or deserialisation error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// An invalid command-line argument.
    #[error("{0}")]
    Arg(String),
}
