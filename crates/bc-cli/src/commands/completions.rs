//! Shell completion script generation.

use std::io::Write as _;

use clap::CommandFactory as _;
use clap_complete::Shell;

use crate::error::CliResult;

/// Arguments for the `completions` subcommand.
#[non_exhaustive]
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
    clippy::needless_pass_by_value,
    reason = "Args is consumed to unpack shell; clap convention passes by value"
)]
#[inline]
pub fn execute(args: Args) -> CliResult<()> {
    let mut cmd = crate::cli::Cli::command();
    let bin_name = cmd.get_name().to_owned();
    let mut buf = Vec::new();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut buf);
    std::io::stdout()
        .write_all(&buf)
        .map_err(crate::error::CliError::Io)
}
