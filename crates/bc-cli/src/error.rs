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

    /// An exporter failed for a reason the command line did not cause: its
    /// plugin failed, or its output could not be written.
    #[error("{0}")]
    Export(bc_core::ExportError),

    /// An invalid command-line argument.
    #[error("{0}")]
    Arg(String),

    /// An import committed what parsed, but some of its source files did not
    /// read or parse.
    #[error("{0}")]
    FilesFailed(String),
}
