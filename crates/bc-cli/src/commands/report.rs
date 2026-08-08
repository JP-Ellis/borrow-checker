//! Report generation sub-commands.

use core::str::FromStr as _;

use bc_models::AccountType;
use clap::Subcommand;
use rust_decimal::Decimal;

use crate::context::AppContext;
use crate::error::CliResult;
use crate::period::PeriodArg;
use crate::period::PeriodInputs;

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
        /// Period granularity. The period instance containing `--date` is used.
        #[arg(long, value_enum, default_value = "monthly")]
        period: PeriodArg,
        /// A date within the desired period (YYYY-MM-DD). Defaults to today.
        #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "fy")]
        date: Option<String>,
        /// Financial year to report, named by the year it ends in.
        #[arg(long, value_name = "YEAR", conflicts_with_all = ["period", "date"])]
        fy: Option<i16>,
    },
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
        Command::Summary { period, date, fy } => summary(ctx, period, date, fy).await,
    }
}

/// Net-worth report: balance of every asset and liability account.
///
/// Uses [`bc_core::AssetService::latest_market_value`] for
/// [`bc_models::AccountKind::ManualAsset`] accounts and
/// [`bc_core::BalanceEngine::balance_for`] for all others.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the account, asset, or balance service.
#[expect(
    clippy::too_many_lines,
    reason = "report function spans table setup and output"
)]
async fn net_worth(ctx: &AppContext) -> CliResult<()> {
    const COMMODITY: &str = "AUD";

    /// Returns a stable, user-friendly string for an [`bc_models::AccountKind`].
    fn kind_label(kind: bc_models::AccountKind) -> &'static str {
        match kind {
            bc_models::AccountKind::DepositAccount => "deposit",
            bc_models::AccountKind::ManualAsset => "manual asset",
            bc_models::AccountKind::Receivable => "receivable",
            bc_models::AccountKind::VirtualAllocation => "virtual",
            bc_models::AccountKind::Group => "group",
            _ => "unknown",
        }
    }

    #[expect(clippy::print_stderr, reason = "user-visible limitation warning")]
    {
        eprintln!(
            "note: net-worth shows {COMMODITY} balances only; multi-currency support requires Milestone 5"
        );
    }

    let total = ctx.balances.net_worth(COMMODITY).await?.value();
    let accounts = ctx.accounts.list_active().await?;

    if ctx.json {
        let mut rows = Vec::new();
        for account in &accounts {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "AccountType is non_exhaustive; unknown future variants are skipped"
            )]
            match account.account_type() {
                AccountType::Asset | AccountType::Liability => {}
                _ => continue,
            }
            let balance = {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "AccountKind is non_exhaustive; fall through to posting-based balance"
                )]
                match account.kind() {
                    bc_models::AccountKind::ManualAsset => ctx
                        .assets
                        .latest_market_value(account.id(), COMMODITY)
                        .await?
                        .unwrap_or(Decimal::ZERO),
                    _ => ctx
                        .balances
                        .balance_for(account.id(), COMMODITY)
                        .await?
                        .value(),
                }
            };
            rows.push(serde_json::json!({
                "account": account.name(),
                "kind": kind_label(account.kind()),
                "commodity": COMMODITY,
                "balance": balance.to_string(),
            }));
        }
        let summary = serde_json::json!({
            "accounts": rows,
            "total": total.to_string(),
            "commodity": COMMODITY,
        });
        return crate::output::print_json(&summary);
    }

    // Human-readable table.
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for account in &accounts {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "AccountType is non_exhaustive; unknown future variants are skipped"
        )]
        match account.account_type() {
            AccountType::Asset | AccountType::Liability => {}
            _ => continue,
        }
        let balance = {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "AccountKind is non_exhaustive; fall through to posting-based balance"
            )]
            match account.kind() {
                bc_models::AccountKind::ManualAsset => ctx
                    .assets
                    .latest_market_value(account.id(), COMMODITY)
                    .await?
                    .unwrap_or(Decimal::ZERO),
                _ => ctx
                    .balances
                    .balance_for(account.id(), COMMODITY)
                    .await?
                    .value(),
            }
        };
        table_rows.push(vec![
            account.name().to_owned(),
            kind_label(account.kind()).to_owned(),
            balance.to_string(),
            COMMODITY.to_owned(),
        ]);
    }

    if table_rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No asset or liability accounts.");
        }
        return Ok(());
    }

    crate::output::print_table(&["ACCOUNT", "KIND", "BALANCE", "CCY"], &table_rows);

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("\nNet Worth: {total} {COMMODITY}");
    }
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
/// * `fy` - Financial year to report, named by the year it ends in.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from the transaction service or
/// date parsing.
async fn summary(
    ctx: &AppContext,
    period: PeriodArg,
    date: Option<String>,
    fy: Option<i16>,
) -> CliResult<()> {
    let inputs = PeriodInputs {
        fortnightly_anchor: ctx.fortnightly_anchor,
        duration_days: None,
        duration_weeks: None,
        duration_months: None,
        fy_start_month: ctx.fy_start_month,
        fy_start_day: ctx.fy_start_day,
    };

    let (start, end) = if let Some(year) = fy {
        crate::period::fy_window(year, &inputs)?
    } else {
        let anchor = if let Some(d) = date {
            jiff::civil::Date::from_str(&d)
                .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{d}': {e}")))?
        } else {
            jiff::Zoned::now().date()
        };
        crate::period::resolve(period, &inputs)?.range_containing(anchor)
    };

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
