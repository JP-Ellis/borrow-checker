//! Export sub-command.
//!
//! Native Rust exporters have been removed; export support will be restored
//! via WASM plugins in a future milestone. The `--format` and `--output`
//! flags are retained so callers are not broken when exporters arrive.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `export` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Output format (e.g. `ledger`, `beancount`). Must match a loaded exporter plugin name.
    #[arg(long)]
    pub format: String,

    /// Path to write the output file. Omit to write to stdout.
    #[arg(long, short = 'o', value_name = "OUTPUT")]
    pub output: Option<PathBuf>,
}

/// Executes the `export` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] — export via plugins is not yet
/// implemented. The format and output flags are accepted so callers are not
/// broken when exporter plugins are added.
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
#[inline]
pub async fn execute(args: Args, _ctx: &AppContext) -> CliResult<()> {
    Err(crate::error::CliError::Arg(format!(
        "export to '{}' is not yet available; exporter plugins will be added in a future milestone",
        args.format
    )))
}
