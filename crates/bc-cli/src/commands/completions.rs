//! Shell completion script generation.

use std::io::Write as _;

use clap::CommandFactory as _;
use clap_complete::Shell;

use crate::error::CliResult;

/// Arguments for the `completions` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Target shell.
    pub shell: Shell,
}

/// Generates and writes a shell completion script for `borrow-checker` to stdout.
///
/// # Arguments
///
/// * `args` - Contains the target shell.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Io`] if writing to stdout fails.
#[expect(
    clippy::print_stdout,
    reason = "completion scripts are intentionally written to stdout"
)]
#[inline]
pub fn execute(args: Args) -> CliResult<()> {
    use clap::CommandFactory as _;
    let mut cmd = crate::cli::Cli::command();
    let bin_name = cmd.get_name().to_owned();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}
