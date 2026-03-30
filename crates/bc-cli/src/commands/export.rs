//! Export sub-command.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Supported export formats.
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Format {
    /// Ledger journal format.
    Ledger,
    /// Beancount journal format.
    Beancount,
}

/// Arguments for the `export` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Output format.
    #[arg(long, value_enum)]
    pub format: Format,

    /// Output file path. Writes to stdout when omitted.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

/// Executes the `export` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or I/O.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
#[inline]
pub async fn execute(_args: Args, _ctx: &AppContext) -> CliResult<()> {
    todo!()
}
