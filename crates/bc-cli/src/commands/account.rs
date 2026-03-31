//! Account management sub-commands: list, create, archive.

use core::str::FromStr as _;

use bc_models::AccountKind;
use bc_models::AccountType;
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
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account service or JSON serialisation.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let accounts = ctx.accounts.list_active().await?;

    if ctx.json {
        return crate::output::print_json(&accounts);
    }

    if accounts.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No active accounts.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = accounts
        .iter()
        .map(|account| {
            let type_str = match account.account_type() {
                bc_models::AccountType::Asset => "Asset",
                bc_models::AccountType::Liability => "Liability",
                bc_models::AccountType::Equity => "Equity",
                bc_models::AccountType::Income => "Income",
                bc_models::AccountType::Expense => "Expense",
                _ => "Unknown",
            };
            let kind_str = match account.kind() {
                bc_models::AccountKind::DepositAccount => "DepositAccount",
                bc_models::AccountKind::ManualAsset => "ManualAsset",
                bc_models::AccountKind::Receivable => "Receivable",
                bc_models::AccountKind::VirtualAllocation => "VirtualAllocation",
                _ => "Unknown",
            };
            vec![
                account.id().to_string(),
                account.name().to_owned(),
                type_str.to_owned(),
                kind_str.to_owned(),
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "NAME", "TYPE", "KIND"], &rows);
    Ok(())
}

/// Creates a new account.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account service or JSON serialisation.
async fn create(
    ctx: &AppContext,
    name: String,
    account_type: TypeArg,
    kind: KindArg,
    description: Option<String>,
) -> CliResult<()> {
    let bc_type = match account_type {
        TypeArg::Asset => AccountType::Asset,
        TypeArg::Liability => AccountType::Liability,
        TypeArg::Equity => AccountType::Equity,
        TypeArg::Income => AccountType::Income,
        TypeArg::Expense => AccountType::Expense,
    };

    let bc_kind = match kind {
        KindArg::DepositAccount => AccountKind::DepositAccount,
        KindArg::ManualAsset => AccountKind::ManualAsset,
        KindArg::Receivable => AccountKind::Receivable,
        KindArg::VirtualAllocation => AccountKind::VirtualAllocation,
    };

    let account_id = ctx
        .accounts
        .create(
            &name,
            bc_type,
            bc_kind,
            description.as_deref(),
            None,
            &[],
            &[],
        )
        .await?;

    if ctx.json {
        let account = ctx.accounts.find_by_id(&account_id).await?;
        return crate::output::print_json(&account);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created account: {name} ({account_id})");
    }
    Ok(())
}

/// Archives an account by ID.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account service or JSON serialisation.
async fn archive(ctx: &AppContext, id: String) -> CliResult<()> {
    let account_id = bc_models::AccountId::from_str(&id)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID '{id}': {e}")))?;

    ctx.accounts.archive(&account_id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "archived": true,
            "id": id,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Archived account: {id}");
    }
    Ok(())
}
