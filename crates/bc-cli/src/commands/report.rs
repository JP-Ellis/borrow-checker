//! Report generation sub-commands.

use core::str::FromStr as _;

use bc_models::AccountType;
use clap::Subcommand;
use rust_decimal::Decimal;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `report` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The report to generate.
    #[command(subcommand)]
    pub command: Command,
}

/// Available reports.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
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
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::NetWorth => net_worth(ctx).await,
        Command::Monthly { month } => monthly(ctx, month).await,
        Command::Annual { year } => annual(ctx, year).await,
        Command::Budget => {
            #[expect(clippy::print_stderr, reason = "CLI stub message")]
            {
                eprintln!("report budget: requires Milestone 5 — not yet implemented");
            }
            Ok(())
        }
    }
}

/// Net-worth report: balance of every asset and liability account.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account or balance service.
async fn net_worth(ctx: &AppContext) -> CliResult<()> {
    const NAME_W: usize = 35;
    const BAL_W: usize = 15;
    const COMM_W: usize = 8;
    const DIVIDER_W: usize = NAME_W + BAL_W + COMM_W + 4;
    // Hard-coded commodity for M3 until a commodity service is added.
    const COMMODITY: &str = "AUD";

    let accounts = ctx.accounts.list_active().await?;

    let mut rows: Vec<(String, String, Decimal)> = Vec::new();
    for account in &accounts {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "AccountType is non_exhaustive; unknown future variants are skipped"
        )]
        match account.account_type() {
            AccountType::Asset | AccountType::Liability => {}
            _ => continue,
        }
        let balance = ctx
            .balances
            .balance_for(account.id(), COMMODITY)
            .await
            .unwrap_or(Decimal::ZERO);
        rows.push((account.name().to_owned(), COMMODITY.to_owned(), balance));
    }

    if ctx.json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, ccy, bal)| {
                serde_json::json!({ "account": name, "commodity": ccy, "balance": bal.to_string() })
            })
            .collect();
        return crate::output::print_json(&json_rows);
    }

    if rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No asset or liability accounts.");
        }
        return Ok(());
    }

    crate::output::print_row(&[("ACCOUNT", NAME_W), ("BALANCE", BAL_W), ("CCY", COMM_W)]);
    crate::output::print_divider(DIVIDER_W);
    for (name, ccy, bal) in &rows {
        crate::output::print_row(&[(name, NAME_W), (&bal.to_string(), BAL_W), (ccy, COMM_W)]);
    }
    Ok(())
}

/// Monthly transactions report.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the transaction service.
async fn monthly(ctx: &AppContext, month: Option<String>) -> CliResult<()> {
    const ID_W: usize = 36;
    const DATE_W: usize = 12;
    const DESC_W: usize = 35;
    const DIVIDER_W: usize = ID_W + DATE_W + DESC_W + 4;

    let month_str = month.unwrap_or_else(|| {
        let now = jiff::Zoned::now();
        format!("{:04}-{:02}", now.year(), now.month())
    });

    let year_month = jiff::civil::Date::from_str(&format!("{month_str}-01"))
        .map_err(|e| crate::error::CliError::Arg(format!("invalid month '{month_str}': {e}")))?;
    let month_end = year_month
        .checked_add(jiff::Span::new().months(1_i64))
        .map_err(|e| crate::error::CliError::Arg(format!("date arithmetic error: {e}")))?;

    let all_txs = ctx.transactions.list().await?;
    let txs: Vec<_> = all_txs
        .iter()
        .filter(|tx| tx.date() >= year_month && tx.date() < month_end)
        .collect();

    if ctx.json {
        return crate::output::print_json(&txs);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Monthly report: {month_str} ({} transactions)", txs.len());
    }

    if txs.is_empty() {
        return Ok(());
    }

    crate::output::print_row(&[("ID", ID_W), ("DATE", DATE_W), ("DESCRIPTION", DESC_W)]);
    crate::output::print_divider(DIVIDER_W);
    for tx in &txs {
        crate::output::print_row(&[
            (&tx.id().to_string(), ID_W),
            (&tx.date().to_string(), DATE_W),
            (tx.description(), DESC_W),
        ]);
    }
    Ok(())
}

/// Annual transactions report grouped by month.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the transaction service.
async fn annual(ctx: &AppContext, year: Option<String>) -> CliResult<()> {
    let year_str = year.unwrap_or_else(|| {
        let now = jiff::Zoned::now();
        format!("{:04}", now.year())
    });

    let year_start = jiff::civil::Date::from_str(&format!("{year_str}-01-01"))
        .map_err(|e| crate::error::CliError::Arg(format!("invalid year '{year_str}': {e}")))?;
    let year_end = year_start
        .checked_add(jiff::Span::new().years(1_i64))
        .map_err(|e| crate::error::CliError::Arg(format!("date arithmetic error: {e}")))?;

    let all_txs = ctx.transactions.list().await?;
    let txs: Vec<_> = all_txs
        .iter()
        .filter(|tx| tx.date() >= year_start && tx.date() < year_end)
        .collect();

    if ctx.json {
        return crate::output::print_json(&txs);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Annual report: {year_str} ({} transactions)", txs.len());
    }

    if txs.is_empty() {
        return Ok(());
    }

    // Group by month.
    let mut by_month: std::collections::BTreeMap<String, Vec<_>> =
        std::collections::BTreeMap::new();
    for tx in &txs {
        let key = format!("{:04}-{:02}", tx.date().year(), tx.date().month());
        by_month.entry(key).or_default().push(tx);
    }

    for (month, month_txs) in &by_month {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("\n  {month} ({} transactions)", month_txs.len());
        }
        for tx in month_txs {
            #[expect(clippy::print_stdout, reason = "CLI output")]
            {
                println!("    {}  {}", tx.date(), tx.description());
            }
        }
    }
    Ok(())
}
