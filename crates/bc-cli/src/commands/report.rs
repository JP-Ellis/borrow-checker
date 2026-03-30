//! Report generation sub-commands.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `report` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The report to generate.
    #[command(subcommand)]
    pub command: ReportCommand,
}

/// Available reports.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum ReportCommand {
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
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        ReportCommand::NetWorth => todo!(),
        ReportCommand::Monthly { .. } => todo!(),
        ReportCommand::Annual { .. } => todo!(),
        ReportCommand::Budget => {
            eprintln!("report budget: requires Milestone 5 — not yet implemented");
            Ok(())
        }
    }
}
