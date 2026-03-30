//! Import sub-command.

use core::str::FromStr as _;
use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `import` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Name of the import profile to use.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Account ID for the offsetting (counterpart) posting.
    ///
    /// CSV and OFX imports produce single-account raw transactions.
    /// This account receives the balancing entry for each imported line.
    #[arg(long, value_name = "ACCOUNT_ID")]
    pub counterpart: String,

    /// File to import.
    pub file: PathBuf,
}

/// Executes the `import` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine, I/O, or parsing.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
pub async fn execute(_args: Args, _ctx: &AppContext) -> CliResult<()> {
    todo!()
}
