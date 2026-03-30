//! Transaction management sub-commands: list, add, amend, void.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `transaction` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The transaction operation to perform.
    #[command(subcommand)]
    pub command: TransactionCommand,
}

/// Available transaction operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum TransactionCommand {
    /// List all non-voided transactions ordered by date descending.
    List,
    /// Record a new double-entry transaction.
    Add {
        /// Transaction date in YYYY-MM-DD format.
        #[arg(long)]
        date: String,
        /// Transaction description (memo).
        #[arg(long)]
        description: String,
        /// Optional payee name.
        #[arg(long)]
        payee: Option<String>,
        /// Posting in `ACCOUNT_ID:AMOUNT:COMMODITY` format. Repeat for each posting.
        /// Must include at least two postings that balance to zero.
        #[arg(
            long = "posting",
            value_name = "ACCOUNT:AMOUNT:COMMODITY",
            num_args = 1
        )]
        postings: Vec<String>,
    },
    /// Amend the metadata of an existing transaction.
    Amend {
        /// Transaction ID to amend.
        id: String,
        /// New date (YYYY-MM-DD).
        #[arg(long)]
        date: Option<String>,
        /// New description.
        #[arg(long)]
        description: Option<String>,
        /// New payee.
        #[arg(long)]
        payee: Option<String>,
    },
    /// Void a transaction (preserves data; excludes from balances and reports).
    Void {
        /// Transaction ID to void.
        id: String,
    },
}

/// Executes the `transaction` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or output layer.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        TransactionCommand::List => list(ctx).await,
        TransactionCommand::Add {
            date,
            description,
            payee,
            postings,
        } => add(ctx, date, description, payee, postings).await,
        TransactionCommand::Amend {
            id,
            date,
            description,
            payee,
        } => amend(ctx, id, date, description, payee).await,
        TransactionCommand::Void { id } => void(ctx, id).await,
    }
}

async fn list(_ctx: &AppContext) -> CliResult<()> {
    todo!()
}

async fn add(
    _ctx: &AppContext,
    _date: String,
    _description: String,
    _payee: Option<String>,
    _postings: Vec<String>,
) -> CliResult<()> {
    todo!()
}

async fn amend(
    _ctx: &AppContext,
    _id: String,
    _date: Option<String>,
    _description: Option<String>,
    _payee: Option<String>,
) -> CliResult<()> {
    todo!()
}

async fn void(_ctx: &AppContext, _id: String) -> CliResult<()> {
    todo!()
}
