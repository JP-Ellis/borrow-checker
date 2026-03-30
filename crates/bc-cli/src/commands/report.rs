//! Report generation sub-commands.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `report` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The report to generate.
    #[command(subcommand)]
    pub command: Command,
}

/// Available reports.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Net worth across all asset and liability accounts.
    NetWorth,
    /// Monthly income and expense summary.
    Monthly {
        /// Month to report (YYYY-MM). Defaults to the current month.
        #[arg(value_name = "YYYY-MM")]
        month: Option<String>,
    },
    /// Annual summary broken down by month.
    Annual {
        /// Year to report (YYYY). Defaults to the current year.
        #[arg(value_name = "YYYY")]
        year: Option<String>,
    },
    /// Budget vs actuals (requires Milestone 5).
    Budget,
}

/// Executes the `report` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or output layer.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::print_stderr,
    reason = "CLI stub message for unimplemented commands"
)]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
pub async fn execute(args: Args, _ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::NetWorth => todo!(),
        Command::Monthly { .. } => todo!(),
        Command::Annual { .. } => todo!(),
        Command::Budget => {
            eprintln!("report budget: requires Milestone 5 — not yet implemented");
            Ok(())
        }
    }
}
