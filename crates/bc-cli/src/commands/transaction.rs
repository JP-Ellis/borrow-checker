//! Transaction management sub-commands: list, add, amend, void.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `transaction` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The transaction operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available transaction operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
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
        Command::List => list(ctx).await,
        Command::Add {
            date,
            description,
            payee,
            postings,
        } => add(ctx, date, description, payee, postings).await,
        Command::Amend {
            id,
            date,
            description,
            payee,
        } => amend(ctx, id, date, description, payee).await,
        Command::Void { id } => void(ctx, id).await,
    }
}

/// Lists all non-voided transactions.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn list(_ctx: &AppContext) -> CliResult<()> {
    todo!()
}

/// Records a new double-entry transaction.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn add(
    _ctx: &AppContext,
    _date: String,
    _description: String,
    _payee: Option<String>,
    _postings: Vec<String>,
) -> CliResult<()> {
    todo!()
}

/// Amends the metadata of an existing transaction.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn amend(
    _ctx: &AppContext,
    _id: String,
    _date: Option<String>,
    _description: Option<String>,
    _payee: Option<String>,
) -> CliResult<()> {
    todo!()
}

/// Voids a transaction by ID.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn void(_ctx: &AppContext, _id: String) -> CliResult<()> {
    todo!()
}
