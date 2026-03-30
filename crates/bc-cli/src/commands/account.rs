//! Account management sub-commands: list, create, archive.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `account` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The account operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available account operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List all active accounts.
    List,
    /// Create a new account.
    Create {
        /// Display name for the account.
        #[arg(long)]
        name: String,
        /// Account type (asset, liability, equity, income, expense).
        #[arg(long, value_enum)]
        r#type: TypeArg,
        /// Account maintenance kind.
        #[arg(long, value_enum, default_value = "deposit-account")]
        kind: KindArg,
        /// Optional free-text description.
        #[arg(long)]
        description: Option<String>,
    },
    /// Archive an account (hides it from active lists; data is preserved).
    Archive {
        /// Account ID to archive.
        id: String,
    },
}

/// CLI representation of [`bc_models::AccountType`].
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum TypeArg {
    /// Asset account.
    Asset,
    /// Liability account.
    Liability,
    /// Equity account.
    Equity,
    /// Income account.
    Income,
    /// Expense account.
    Expense,
}

/// CLI representation of [`bc_models::AccountKind`].
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum KindArg {
    /// Standard bank/card/brokerage account (may have an import profile).
    #[value(name = "deposit-account")]
    DepositAccount,
    /// Manually-valued real asset (property, vehicle).
    #[value(name = "manual-asset")]
    ManualAsset,
    /// Money owed to you by a third party.
    Receivable,
    /// Sub-account that subdivides a parent account's balance.
    #[value(name = "virtual-allocation")]
    VirtualAllocation,
}

/// Executes the `account` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or output layer.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx).await,
        Command::Create {
            name,
            r#type,
            kind,
            description,
        } => create(ctx, name, r#type, kind, description).await,
        Command::Archive { id } => archive(ctx, id).await,
    }
}

/// Lists all active accounts.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn list(_ctx: &AppContext) -> CliResult<()> {
    todo!()
}

/// Creates a new account.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn create(
    _ctx: &AppContext,
    _name: String,
    _account_type: TypeArg,
    _kind: KindArg,
    _description: Option<String>,
) -> CliResult<()> {
    todo!()
}

/// Archives an account by ID.
#[expect(clippy::todo, reason = "implemented in a subsequent task")]
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
async fn archive(_ctx: &AppContext, _id: String) -> CliResult<()> {
    todo!()
}
