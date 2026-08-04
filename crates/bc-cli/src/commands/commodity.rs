//! Commodity registry sub-commands: list, create, update, delete.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Arguments for the `commodity` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The commodity operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available commodity operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List every registered commodity.
    List,
    /// Register a new commodity.
    Create(CreateArgs),
}

/// Arguments for `commodity create`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct CreateArgs {
    /// Canonical code for the commodity (e.g. `BNB`).
    #[arg(value_name = "CODE")]
    pub code: String,
    /// Human-readable full name (e.g. `Bitcoin`).
    #[arg(long)]
    pub name: Option<String>,
    /// Display symbol used when formatting amounts (e.g. `$`).
    #[arg(long)]
    pub symbol: Option<String>,
    /// Exchange or market where the commodity trades (e.g. `NASDAQ`).
    #[arg(long)]
    pub exchange: Option<String>,
    /// Free-text description (e.g. an ISIN).
    #[arg(long)]
    pub description: Option<String>,
    /// Additional input marker for this commodity. Repeatable.
    #[arg(long = "alias", value_name = "ALIAS")]
    pub aliases: Vec<String>,
    /// Number of minor-unit digits shown when formatting amounts.
    #[arg(long, default_value_t = 2)]
    pub decimals: u8,
    /// Treat the commodity as a non-ISO-4217 asset (e.g. crypto, equities).
    #[arg(long = "no-iso")]
    pub no_iso: bool,
    /// Place the display symbol after the amount (e.g. `100 ETH`).
    #[arg(long)]
    pub symbol_after: bool,
    /// First date from which the commodity is valid (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_from: Option<String>,
    /// Last date on which the commodity is valid (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_until: Option<String>,
}

/// Executes the `commodity` subcommand.
///
/// # Arguments
///
/// * `args` - The parsed subcommand arguments.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns a [`crate::error::CliError`] if a service call fails or an argument
/// is invalid.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx).await,
        Command::Create(create_args) => create(ctx, create_args).await,
    }
}

/// Lists every registered commodity as a table or JSON.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Propagates [`crate::error::CliError::Core`] from the commodity service or
/// [`crate::error::CliError::Json`] from JSON serialisation.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let all = ctx.commodities.list_all().await?;
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|c| {
            vec![
                c.code().to_owned(),
                c.name().unwrap_or_default().to_owned(),
                c.symbol().unwrap_or_default().to_owned(),
                c.decimals().to_string(),
                if c.is_iso() { "yes" } else { "no" }.to_owned(),
                c.aliases().join(", "),
            ]
        })
        .collect();

    if ctx.json {
        return crate::output::print_json(&rows);
    }
    if rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No commodities.");
        }
        return Ok(());
    }
    crate::output::print_table(
        &["CODE", "NAME", "SYMBOL", "DECIMALS", "ISO", "ALIASES"],
        &rows,
    );
    Ok(())
}

/// Parses an optional `YYYY-MM-DD` argument, preserving `None`.
///
/// [`crate::commands::parse_date_or_today`] substitutes today's date for
/// `None`; an unset commodity validity bound must stay unset instead.
///
/// # Arguments
///
/// * `flag` - The flag name, used in the error message.
/// * `value` - The optional date string.
///
/// # Returns
///
/// The parsed date, or `None` when `value` is `None`.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the string is not a valid `YYYY-MM-DD` date.
fn parse_optional_date(flag: &str, value: Option<&str>) -> CliResult<Option<jiff::civil::Date>> {
    value
        .map(|d| {
            d.parse::<jiff::civil::Date>()
                .map_err(|e| CliError::Arg(format!("invalid --{flag} '{d}': {e}")))
        })
        .transpose()
}

/// Registers a new commodity.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
/// * `args` - The parsed `create` arguments.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if a date argument is malformed, or propagates
/// [`CliError::Core`] from the commodity service — notably
/// [`bc_core::BcError::MarkerConflict`] when the code, symbol, or an alias is
/// already taken.
async fn create(ctx: &AppContext, args: CreateArgs) -> CliResult<()> {
    let active_from = parse_optional_date("active-from", args.active_from.as_deref())?;
    let active_until = parse_optional_date("active-until", args.active_until.as_deref())?;

    let commodity = bc_models::Commodity::builder()
        .code(args.code.clone())
        .maybe_name(args.name)
        .maybe_symbol(args.symbol)
        .maybe_exchange(args.exchange)
        .maybe_description(args.description)
        .aliases(args.aliases)
        .decimals(args.decimals)
        .is_iso(!args.no_iso)
        .symbol_after(args.symbol_after)
        .maybe_active_from(active_from)
        .maybe_active_until(active_until)
        .build();

    let stored = ctx.commodities.create(&commodity).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "id": stored.id().to_string(),
            "code": stored.code(),
        }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created commodity {} ({})", stored.code(), stored.id());
    }
    Ok(())
}
