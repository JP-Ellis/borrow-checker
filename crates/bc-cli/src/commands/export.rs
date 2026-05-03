//! Export sub-command.
//!
//! Native Rust exporters have been removed; export support will be restored
//! via WASM plugins in a future milestone.

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
}

/// Executes the `export` subcommand.
///
/// # Errors
///
/// Always returns [`crate::error::CliError::Arg`] — export via plugins is not
/// yet implemented.
#[inline]
pub fn execute(args: &Args, _ctx: &AppContext) -> CliResult<()> {
    let name = match args.format {
        Format::Ledger => "ledger",
        Format::Beancount => "beancount",
    };
    Err(crate::error::CliError::Arg(format!(
        "export to '{name}' is not yet available; native exporters have been removed \
         and plugin-based export will be added in a future milestone"
    )))
}
