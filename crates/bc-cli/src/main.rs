//! BorrowChecker CLI entry point.
//!
//! Parses arguments, opens the database, and dispatches to the appropriate
//! command module.

mod cli;
mod commands;
mod context;
mod error;
mod output;

use clap::Parser as _;

use crate::cli::Commands;
use crate::context::AppContext;
use crate::context::default_db_path;
use crate::error::CliError;

#[expect(
    clippy::print_stderr,
    reason = "CLI binary: stderr is the intended channel for error messages"
)]
#[tokio::main]
async fn main() {
    let cli = crate::cli::Cli::parse();

    let db_path = cli.global.db_path.clone().unwrap_or_else(default_db_path);

    if let Some(parent) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: cannot create database directory: {e}");
            std::process::exit(1_i32);
        }
    }

    let ctx = match AppContext::open(&db_path, cli.global.json).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1_i32);
        }
    };

    let result: crate::error::CliResult<()> = match cli.command {
        Commands::Account(args) => commands::account::execute(args, &ctx).await,
        Commands::Transaction(args) => commands::transaction::execute(args, &ctx).await,
        Commands::Import(args) => commands::import::execute(args, &ctx).await,
        Commands::Export(args) => commands::export::execute(args, &ctx).await,
        Commands::Report(args) => commands::report::execute(args, &ctx).await,
        Commands::Budget(args) => commands::budget::execute(args, &ctx).await,
        Commands::Plugin(args) => commands::plugin::execute(args, &ctx).await,
        Commands::Completions(args) => commands::completions::execute(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        let code = match &e {
            CliError::Core(bc_core::BcError::NotFound(_)) => 2_i32,
            CliError::Core(_) | CliError::Io(_) | CliError::Json(_) | CliError::Arg(_) => 1_i32,
        };
        std::process::exit(code);
    }
}
