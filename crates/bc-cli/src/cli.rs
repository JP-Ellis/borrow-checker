//! Clap CLI definition: top-level [`Cli`] struct and [`Commands`] enum.

use std::path::PathBuf;

use crate::commands::account;
use crate::commands::asset;
use crate::commands::budget;
use crate::commands::completions;
use crate::commands::export;
use crate::commands::import;
use crate::commands::plugin;
use crate::commands::report;
use crate::commands::transaction;

/// BorrowChecker — personal finance with ledger/beancount compatibility.
#[non_exhaustive]
#[derive(Debug, clap::Parser)]
#[command(name = "borrow-checker", version, about, long_about = None)]
pub struct Cli {
    /// Global flags shared across all subcommands.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Global flags available on every subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true, env = "BC_JSON")]
    pub json: bool,

    /// Path to the SQLite database file.
    ///
    /// Overrides the `db_path` config file setting and the platform default.
    /// The `BC_DB_PATH` environment variable is also honoured via the config
    /// layer rather than directly by this flag.
    #[arg(long, global = true)]
    pub db_path: Option<PathBuf>,

    /// Increase log verbosity.
    ///
    /// Pass once for info (`-v`), twice for debug (`-vv`), three times for
    /// trace (`-vvv`). Use `RUST_LOG` for fine-grained per-crate control.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Decrease log verbosity.
    ///
    /// Pass once for error-only output (`-q`). Use `RUST_LOG` for
    /// fine-grained per-crate control.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub quiet: u8,
}

/// Top-level subcommands.
#[derive(Debug, clap::Subcommand)]
#[non_exhaustive]
pub enum Commands {
    /// Manage accounts (list, create, archive).
    Account(account::Args),
    /// Manage assets (record-valuation, depreciate, set-loan-terms, amortization).
    Asset(asset::Args),
    /// Manage transactions (list, add, amend, void).
    Transaction(transaction::Args),
    /// Import transactions from a file using a named import profile.
    Import(import::Args),
    /// Export all accounts and transactions to a file or stdout.
    Export(export::Args),
    /// Generate financial reports.
    Report(report::Args),
    /// Manage budget envelopes (requires Milestone 5).
    Budget(budget::Args),
    /// Manage plugins (requires Milestone 6).
    Plugin(plugin::Args),
    /// Generate shell completion scripts.
    Completions(completions::Args),
}
