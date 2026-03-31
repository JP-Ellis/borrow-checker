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
    /// Transaction summary for a configurable time period.
    Summary {
        /// Period granularity.
        ///
        /// Determines the date range: the period instance containing `--date`
        /// is selected. Defaults to `monthly`.
        #[arg(long, value_enum, default_value = "monthly")]
        period: PeriodArg,
        /// A date within the desired period (YYYY-MM-DD). Defaults to today.
        #[arg(long, value_name = "YYYY-MM-DD")]
        date: Option<String>,
    },
    /// Budget vs actuals (requires Milestone 5).
    Budget,
}

/// CLI period selector for the `report summary` command.
///
/// Covers the fixed-anchor periods that require no additional configuration.
/// Financial-year periods (which need a configurable start month/day from the
/// config file) will be added once config integration is complete.
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum PeriodArg {
    /// Every 7 days (Monday–Sunday).
    Weekly,
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct).
    Quarterly,
    /// Full calendar year (1 Jan – 31 Dec).
    #[value(name = "calendar-year")]
    CalendarYear,
}

impl From<PeriodArg> for bc_models::Period {
    #[inline]
    fn from(arg: PeriodArg) -> Self {
        match arg {
            PeriodArg::Weekly => Self::Weekly,
            PeriodArg::Monthly => Self::Monthly,
            PeriodArg::Quarterly => Self::Quarterly,
            PeriodArg::CalendarYear => Self::CalendarYear,
        }
    }
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
        Command::Summary { period, date } => summary(ctx, period, date).await,
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
    // Hard-coded commodity for M3 until a commodity service is added.
    // Only balances denominated in AUD are included; non-AUD accounts show zero.
    const COMMODITY: &str = "AUD";

    let accounts = ctx.accounts.list_active().await?;

    #[expect(clippy::print_stderr, reason = "user-visible limitation warning")]
    {
        eprintln!(
            "note: net-worth currently shows {COMMODITY} balances only; multi-currency support requires Milestone 5"
        );
    }

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
        let balance = ctx.balances.balance_for(account.id(), COMMODITY).await?;
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

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|(name, ccy, bal)| vec![name.clone(), bal.to_string(), ccy.clone()])
        .collect();
    crate::output::print_table(&["ACCOUNT", "BALANCE", "CCY"], &table_rows);
    Ok(())
}

/// Period summary report: lists transactions within the period instance
/// containing `date`.
///
/// # Arguments
///
/// * `ctx` - Shared application context.
/// * `period` - The period granularity to use.
/// * `date` - A date within the desired period. Defaults to today.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the transaction service or
/// date parsing.
async fn summary(ctx: &AppContext, period: PeriodArg, date: Option<String>) -> CliResult<()> {
    let anchor = if let Some(d) = date {
        jiff::civil::Date::from_str(&d)
            .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{d}': {e}")))?
    } else {
        jiff::Zoned::now().date()
    };

    let bc_period = bc_models::Period::from(period);
    let (start, end) = bc_period.range_containing(anchor);

    let all_txs = ctx.transactions.list().await?;
    let txs: Vec<_> = all_txs
        .iter()
        .filter(|tx| tx.date() >= start && tx.date() < end)
        .collect();

    if ctx.json {
        return crate::output::print_json(&txs);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Report: {} – {} ({} transactions)", start, end, txs.len());
    }

    if txs.is_empty() {
        return Ok(());
    }

    let rows: Vec<Vec<String>> = txs
        .iter()
        .map(|tx| {
            vec![
                tx.id().to_string(),
                tx.date().to_string(),
                tx.description().to_owned(),
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "DATE", "DESCRIPTION"], &rows);
    Ok(())
}

#[cfg(test)]
mod tests {
    use bc_models::Period;

    use super::PeriodArg;

    #[test]
    fn period_arg_converts_to_bc_models_period() {
        assert!(matches!(Period::from(PeriodArg::Weekly), Period::Weekly));
        assert!(matches!(Period::from(PeriodArg::Monthly), Period::Monthly));
        assert!(matches!(
            Period::from(PeriodArg::Quarterly),
            Period::Quarterly
        ));
        assert!(matches!(
            Period::from(PeriodArg::CalendarYear),
            Period::CalendarYear
        ));
    }
}
