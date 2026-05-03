//! BorrowChecker CLI entry point.
//!
//! Parses arguments, opens the database, and dispatches to the appropriate
//! command module.

mod cli;
mod commands;
mod context;
mod error;
mod logging;
mod output;

use clap::Parser as _;

use crate::cli::Commands;
use crate::context::AppContext;
use crate::error::CliError;

#[expect(
    clippy::print_stderr,
    reason = "CLI binary: stderr is the intended channel for error messages"
)]
#[tokio::main]
async fn main() {
    let cli = crate::cli::Cli::parse();

    // Load config — non-fatal; fall back to defaults on error.
    let mut settings = bc_config::Settings::load().unwrap_or_else(|e| {
        #[expect(
            clippy::print_stderr,
            reason = "CLI binary: warning to stderr on config load failure"
        )]
        {
            eprintln!("warning: could not load config: {e}; using defaults");
        }
        bc_config::Settings::default()
    });

    let _otel_guard =
        logging::setup_tracing(cli.global.verbose, cli.global.quiet, settings.cli().log());

    if let Some(db_path) = cli.global.db_path.clone() {
        settings.set_db_path(db_path);
    }

    let json = cli.global.json || settings.cli().json();
    let ctx = match AppContext::open(&settings, json).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1_i32);
        }
    };

    let result: crate::error::CliResult<()> = match cli.command {
        Commands::Account(args) => commands::account::execute(args, &ctx).await,
        Commands::Asset(args) => commands::asset::execute(args, &ctx).await,
        Commands::Transaction(args) => commands::transaction::execute(args, &ctx).await,
        Commands::Import(args) => commands::import::execute(args, &ctx).await,
        Commands::Export(args) => commands::export::execute(&args, &ctx),
        Commands::Report(args) => commands::report::execute(args, &ctx).await,
        Commands::Budget(args) => commands::budget::execute(args, &ctx).await,
        Commands::Plugin(args) => commands::plugin::execute(args, &ctx).await,
        Commands::Completions(args) => commands::completions::execute(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        let code = match &e {
            CliError::Core(bc_core::BcError::NotFound(_)) => 2_i32,
            CliError::Core(
                bc_core::BcError::AlreadyVoided(_) | bc_core::BcError::AlreadyArchived(_),
            ) => 3_i32,
            CliError::Core(_) | CliError::Io(_) | CliError::Json(_) | CliError::Arg(_) => 1_i32,
        };
        std::process::exit(code);
    }
}
