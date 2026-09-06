//! Account management sub-commands: list, create, archive, balance.

use core::str::FromStr as _;

use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::DepreciationPolicy;
use clap::Subcommand;
use rust_decimal::Decimal;

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
    /// Create an account from a colon-path, minting any missing ancestors.
    ///
    /// Ancestors are created as `group` accounts. The account type is derived
    /// from the root segment (Assets, Liabilities, Equity, Income, Expenses)
    /// unless `--type` is given.
    Create {
        /// The colon-joined account path to create (e.g. `Assets:BankA:Checking`).
        path: String,
        /// Account type. Derived from the root segment when omitted.
        #[arg(long, value_enum)]
        r#type: Option<TypeArg>,
        /// Account maintenance kind for the leaf. Defaults to `deposit-account`
        /// when the account is created; when it already exists, an omitted kind
        /// is not compared.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Optional free-text description.
        #[arg(long)]
        description: Option<String>,
        /// Acquisition date for `ManualAsset` accounts (YYYY-MM-DD).
        #[arg(long, value_name = "YYYY-MM-DD")]
        acquisition_date: Option<String>,
        /// Acquisition cost for `ManualAsset` accounts (decimal).
        #[arg(long)]
        acquisition_cost: Option<String>,
        /// Depreciation method for `ManualAsset` accounts.
        #[arg(long, value_enum)]
        depreciation_policy: Option<DepreciationPolicyArg>,
        /// Annual depreciation rate as a fraction (e.g. 0.10 = 10%).
        ///
        /// Required when `--depreciation-policy` is `straight-line` or
        /// `declining-balance`.
        #[arg(long)]
        annual_rate: Option<String>,
    },
    /// Archive an account (hides it from active lists; data is preserved).
    Archive {
        /// Account ID to archive.
        id: String,
    },
    /// List account balances (default commodity) in a table.
    Balance {
        /// Optional account ID to filter to a single account.
        account_id: Option<String>,
        /// Filter balances to a single commodity code.
        #[arg(long, value_name = "CODE")]
        commodity: Option<String>,
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
    /// Organisational node that holds no postings of its own.
    Group,
}

/// CLI representation of [`bc_models::DepreciationPolicy`] (without the `annual_rate` field).
///
/// The annual rate is supplied via a separate `--annual-rate` flag.
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum DepreciationPolicyArg {
    /// Straight-line depreciation.
    #[value(name = "straight-line")]
    StraightLine,
    /// Declining-balance depreciation.
    #[value(name = "declining-balance")]
    DecliningBalance,
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
            path,
            r#type,
            kind,
            description,
            acquisition_date,
            acquisition_cost,
            depreciation_policy,
            annual_rate,
        } => {
            create(
                ctx,
                path,
                r#type,
                kind,
                description,
                acquisition_date,
                acquisition_cost,
                depreciation_policy,
                annual_rate,
            )
            .await
        }
        Command::Archive { id } => archive(ctx, id).await,
        Command::Balance {
            account_id,
            commodity,
        } => balance(ctx, account_id, commodity).await,
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
                bc_models::AccountKind::Group => "Group",
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

/// Creates an account from a colon-path, minting any missing ancestors.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account service or JSON
/// serialisation. Returns [`crate::error::CliError::Arg`] if the path is
/// malformed or if the acquisition date/cost/rate cannot be parsed.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters come from CLI flags"
)]
async fn create(
    ctx: &AppContext,
    path: String,
    account_type: Option<TypeArg>,
    kind: Option<KindArg>,
    description: Option<String>,
    acquisition_date: Option<String>,
    acquisition_cost: Option<String>,
    depreciation_policy: Option<DepreciationPolicyArg>,
    annual_rate: Option<String>,
) -> CliResult<()> {
    let parsed = bc_core::AccountPath::parse(&path).map_err(|err| {
        if let bc_core::BcError::BadData(msg) = err {
            crate::error::CliError::Arg(msg)
        } else {
            crate::error::CliError::Arg(format!("invalid account path '{path}': {err}"))
        }
    })?;

    let bc_type = account_type.map(|arg| match arg {
        TypeArg::Asset => AccountType::Asset,
        TypeArg::Liability => AccountType::Liability,
        TypeArg::Equity => AccountType::Equity,
        TypeArg::Income => AccountType::Income,
        TypeArg::Expense => AccountType::Expense,
    });

    let bc_kind = kind.map(|arg| match arg {
        KindArg::DepositAccount => AccountKind::DepositAccount,
        KindArg::ManualAsset => AccountKind::ManualAsset,
        KindArg::Receivable => AccountKind::Receivable,
        KindArg::VirtualAllocation => AccountKind::VirtualAllocation,
        KindArg::Group => AccountKind::Group,
    });

    let acq_date = acquisition_date
        .as_deref()
        .map(jiff::civil::Date::from_str)
        .transpose()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid acquisition_date: {e}")))?;

    let acq_cost = acquisition_cost
        .as_deref()
        .map(Decimal::from_str)
        .transpose()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid acquisition_cost: {e}")))?;

    let depr_policy = match depreciation_policy {
        None => None,
        Some(policy_arg) => {
            let rate_str = annual_rate.as_deref().ok_or_else(|| {
                crate::error::CliError::Arg(
                    "--annual-rate is required when --depreciation-policy is set".into(),
                )
            })?;
            let rate = Decimal::from_str(rate_str)
                .map_err(|e| crate::error::CliError::Arg(format!("invalid annual_rate: {e}")))?;
            let policy = match policy_arg {
                DepreciationPolicyArg::StraightLine => {
                    DepreciationPolicy::StraightLine { annual_rate: rate }
                }
                DepreciationPolicyArg::DecliningBalance => {
                    DepreciationPolicy::DecliningBalance { annual_rate: rate }
                }
            };
            Some(policy)
        }
    };

    let rendered = parsed.to_string();
    let spec = bc_core::PathSpec::builder()
        .path(parsed)
        .maybe_account_type(bc_type)
        .maybe_kind(bc_kind)
        .maybe_description(description)
        .maybe_acquisition_date(acq_date)
        .maybe_acquisition_cost(acq_cost)
        .maybe_depreciation_policy(depr_policy)
        .build();

    let outcome = ctx
        .accounts
        .create_paths(&[spec])
        .await
        .map_err(reword_underivable_root_error)?;
    let account_id = outcome
        .ids
        .get(&rendered)
        .ok_or_else(|| crate::error::CliError::Arg(format!("path '{rendered}' had no leaf")))?;
    let was_created = outcome.created.iter().any(|p| p == &rendered);
    let ancestors: Vec<&String> = outcome.created.iter().filter(|p| *p != &rendered).collect();

    if ctx.json {
        let account = ctx.accounts.find_by_id(account_id).await?;
        return crate::output::print_json(&serde_json::json!({
            "account": account,
            "created": was_created,
            "also_created": ancestors,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        if was_created {
            println!("Created account: {rendered} ({account_id})");
        } else {
            println!("Account already exists: {rendered} ({account_id})");
        }
        if !ancestors.is_empty() {
            let list = ancestors
                .iter()
                .map(|p| format!("{p} (Group)"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  also created: {list}");
        }
    }
    Ok(())
}

/// Names the `--type` flag in the core's underivable-root error.
///
/// [`bc_core::AccountService::create_paths`] cannot mention CLI flag spellings, so it
/// tells the caller to "pass an explicit type"; this names the concrete flag
/// for a CLI user. Any other error passes through unchanged.
fn reword_underivable_root_error(err: bc_core::BcError) -> crate::error::CliError {
    if let bc_core::BcError::InvalidInput(msg) = &err
        && let Some(prefix) = msg.strip_suffix("pass an explicit type to set it")
    {
        return crate::error::CliError::Arg(format!("{prefix}pass --type to set it explicitly"));
    }
    crate::error::CliError::Core(err)
}

/// Archives an account by ID.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account service or JSON serialisation.
async fn archive(ctx: &AppContext, id: String) -> CliResult<()> {
    let account_id = bc_models::AccountId::from_str(&id)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID '{id}': {e}")))?;

    // The `--cascade` flag lands in a follow-up task; preserve today's
    // non-cascading behaviour until it does.
    ctx.accounts.archive(&account_id, false).await?;

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

/// Human-readable label for an [`AccountType`], matching the `list` table.
fn type_label(account_type: AccountType) -> &'static str {
    match account_type {
        AccountType::Asset => "Asset",
        AccountType::Liability => "Liability",
        AccountType::Equity => "Equity",
        AccountType::Income => "Income",
        AccountType::Expense => "Expense",
        _ => "Unknown",
    }
}

/// Sort rank for an [`AccountType`]: Asset → Liability → Equity → Income → Expense.
///
/// [`AccountType`] does not derive [`Ord`], so ordering is defined explicitly.
/// Unknown future variants sort last.
fn type_rank(account_type: AccountType) -> u8 {
    match account_type {
        AccountType::Asset => 0,
        AccountType::Liability => 1,
        AccountType::Equity => 2,
        AccountType::Income => 3,
        AccountType::Expense => 4,
        _ => u8::MAX,
    }
}

/// Lists account balances in the default commodity.
///
/// Balances come from [`bc_core::BalanceEngine::default_balances`]; account
/// names and types are joined from the active account list. Rows are sorted by
/// account type (Asset → Liability → Equity → Income → Expense) then
/// alphabetically by name.
///
/// # Arguments
///
/// * `ctx` - Shared application context.
/// * `account_id` - Optional account ID to filter to a single account.
/// * `commodity` - Optional commodity code to filter balances.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if `account_id` is not a valid
/// account ID. Propagates [`crate::error::CliError`] from the balance or account
/// service, or JSON serialisation.
async fn balance(
    ctx: &AppContext,
    account_id: Option<String>,
    commodity: Option<String>,
) -> CliResult<()> {
    let filter_id = account_id
        .as_deref()
        .map(bc_models::AccountId::from_str)
        .transpose()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;

    let balances = ctx.balances.default_balances().await?;
    let accounts = ctx.accounts.list_active().await?;

    // Join balances with account metadata, applying the ID and commodity filters.
    let mut rows: Vec<(&bc_models::Account, &bc_models::Amount)> = accounts
        .iter()
        .filter_map(|account| balances.get(account.id()).map(|amount| (account, amount)))
        .filter(|(account, _)| filter_id.as_ref().is_none_or(|id| account.id() == id))
        .filter(|(_, amount)| {
            commodity
                .as_deref()
                .is_none_or(|code| amount.commodity().as_str() == code)
        })
        .collect();

    rows.sort_by(|(a, _), (b, _)| {
        type_rank(a.account_type())
            .cmp(&type_rank(b.account_type()))
            .then_with(|| a.name().cmp(b.name()))
    });

    if ctx.json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|(account, amount)| {
                serde_json::json!({
                    "id": account.id().to_string(),
                    "name": account.name(),
                    "type": account.account_type(),
                    "balance": amount.value().to_string(),
                    "commodity": amount.commodity().as_str(),
                })
            })
            .collect();
        return crate::output::print_json(&json_rows);
    }

    if rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No account balances.");
        }
        return Ok(());
    }

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|(account, amount)| {
            vec![
                account.id().to_string(),
                account.name().to_owned(),
                type_label(account.account_type()).to_owned(),
                amount.value().to_string(),
                amount.commodity().as_str().to_owned(),
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "NAME", "TYPE", "BALANCE", "COMMODITY"], &table_rows);
    Ok(())
}
