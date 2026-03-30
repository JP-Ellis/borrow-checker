//! Budget management sub-commands (stub — requires Milestone 5).

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `budget` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The budget operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available budget operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Show budget status across all envelopes.
    Status,
    /// Allocate funds to an envelope.
    Allocate,
    /// List all budget envelopes.
    Envelopes,
}

/// Executes the `budget` subcommand.
///
/// All operations print a "not yet implemented" stub message to stderr.
///
/// # Errors
///
/// Always returns `Ok(())`.
#[expect(
    clippy::print_stderr,
    reason = "CLI stub message for unimplemented commands"
)]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
pub async fn execute(args: Args, _ctx: &AppContext) -> CliResult<()> {
    let op = match args.command {
        Command::Status => "status",
        Command::Allocate => "allocate",
        Command::Envelopes => "envelopes",
    };
    eprintln!("budget {op}: requires Milestone 5 (budgeting) — not yet implemented");
    Ok(())
}
