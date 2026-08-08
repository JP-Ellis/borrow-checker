//! Report generation sub-commands.

use core::fmt::Write as _;

use bc_core::search::TransactionQuery;
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
    /// Postings totalled by account over a period, rolled up the account tree.
    Categories {
        /// Period granularity. The period instance containing `--date` is used.
        #[arg(long, value_enum, default_value = "monthly")]
        period: PeriodArg,
        /// A date within the desired period (YYYY-MM-DD). Defaults to today.
        #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "fy")]
        date: Option<String>,
        /// Financial year to report, named by the year it ends in.
        #[arg(long, value_name = "YEAR", conflicts_with_all = ["period", "date"])]
        fy: Option<i16>,
        /// Account path to scope to, including its subtree. Repeatable.
        #[arg(long, value_name = "PATH")]
        account: Vec<String>,
        /// Tag path to filter by. Repeatable; multiple tags union.
        #[arg(long, value_name = "PATH")]
        tag: Vec<String>,
        /// Show at most N levels of emitted rows. `depth` counts only rows that
        /// survive pruning, not the account-tree root — a scoped report may
        /// need a higher N. Must be at least 1.
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
        /// Commodity to report in. Other commodities are excluded, not converted.
        #[arg(long, default_value = "AUD")]
        commodity: String,
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
        Command::Categories {
            period,
            date,
            fy,
            account,
            tag,
            depth,
            commodity,
        } => categories(ctx, period, date, fy, account, tag, depth, commodity).await,
    }
}

/// Resolves the report window from a period/date/fy triple.
///
/// `--fy` takes a whole financial year; otherwise the period instance
/// containing `date` (or today, if `date` is unset) is used.
///
/// # Errors
///
/// Propagates [`crate::error::CliError`] from period resolution or date
/// parsing. Returns [`crate::error::CliError::Arg`] if `period` is
/// [`PeriodArg::Custom`] — `report` commands expose no `--duration-*` flags,
/// so a custom period can never be satisfied.
fn resolve_window(
    ctx: &AppContext,
    period: PeriodArg,
    date: Option<&str>,
    fy: Option<i16>,
) -> CliResult<(jiff::civil::Date, jiff::civil::Date)> {
    if matches!(period, PeriodArg::Custom) {
        return Err(crate::error::CliError::Arg(
            "--period custom is not supported by report commands; they expose no \
             --duration-* flags to configure it"
                .to_owned(),
        ));
    }

    let inputs = PeriodInputs {
        fortnightly_anchor: ctx.fortnightly_anchor,
        duration_days: None,
        duration_weeks: None,
        duration_months: None,
        fy_start_month: ctx.fy_start_month,
        fy_start_day: ctx.fy_start_day,
    };

    if let Some(year) = fy {
        return crate::period::fy_window(year, &inputs);
    }

    let anchor = crate::commands::parse_date_or_today(date)?;
    Ok(crate::period::resolve(period, &inputs)?.range_containing(anchor))
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
    let (start, end) = resolve_window(ctx, period, date.as_deref(), fy)?;

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

/// Category report: postings totalled by account over a period, rolled up
/// the account tree.
///
/// # Arguments
///
/// * `ctx` - Shared application context.
/// * `period` - The period granularity to use.
/// * `date` - A date within the desired period. Defaults to today.
/// * `fy` - Financial year to report, named by the year it ends in.
/// * `account` - Account paths to scope to, including their subtrees.
/// * `tag` - Tag paths to filter by.
/// * `depth` - Maximum row depth to render, if capped.
/// * `commodity` - Commodity to report in.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if an `--account` or `--tag`
/// path names nothing. Propagates [`crate::error::CliError`] from the core
/// engine otherwise.
#[expect(clippy::too_many_arguments, reason = "mirrors the CLI flag surface")]
async fn categories(
    ctx: &AppContext,
    period: PeriodArg,
    date: Option<String>,
    fy: Option<i16>,
    account: Vec<String>,
    tag: Vec<String>,
    depth: Option<usize>,
    commodity: String,
) -> CliResult<()> {
    if depth == Some(0) {
        return Err(crate::error::CliError::Arg(
            "--depth 0 would show no rows; the minimum is 1".to_owned(),
        ));
    }

    let (start, end) = resolve_window(ctx, period, date.as_deref(), fy)?;

    let mut account_ids = Vec::new();
    if !account.is_empty() {
        let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
        for raw in &account {
            let path = bc_core::AccountPath::parse(raw)?;
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "Resolution is non_exhaustive; any non-Resolved outcome is an unresolvable path"
            )]
            match resolver.resolve(&path) {
                bc_core::Resolution::Resolved { id, .. } => account_ids.push(id),
                _ => {
                    return Err(crate::error::CliError::Arg(format!(
                        "no account matches path '{raw}'"
                    )));
                }
            }
        }
    }

    let mut tag_ids = Vec::new();
    for raw in &tag {
        let path = raw
            .parse()
            .map_err(|e| crate::error::CliError::Arg(format!("invalid tag path '{raw}': {e}")))?;
        match ctx.tags.find_by_path(&path).await? {
            Some(id) => tag_ids.push(id),
            None => {
                return Err(crate::error::CliError::Arg(format!(
                    "no tag matches path '{raw}'"
                )));
            }
        }
    }

    let query = TransactionQuery::windowed(Some(start), Some(end), account_ids, tag_ids);

    let mut report =
        bc_core::category_totals(&ctx.transactions, &ctx.accounts, &query, &commodity).await?;
    apply_depth_filter(&mut report.rows, depth);

    let rendered = Rendered {
        rows: report.rows,
        excluded_postings: report.excluded_postings,
        ambiguous_transactions: report.ambiguous_transactions,
        commodity,
        start,
        end,
    };

    if ctx.json {
        let rows: Vec<serde_json::Value> = rendered
            .rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "account": row.path,
                    "depth": row.depth,
                    "own": row.own.value().to_string(),
                    "rolled_up": row.rolled_up.value().to_string(),
                })
            })
            .collect();
        let summary = serde_json::json!({
            "start": rendered.start.to_string(),
            "end": rendered.end.to_string(),
            "commodity": rendered.commodity,
            "rows": rows,
            "excluded_postings": rendered.excluded_postings,
            "ambiguous_transactions": rendered.ambiguous_transactions,
        });
        return crate::output::print_json(&summary);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", rendered.render());
    }
    Ok(())
}

// MARK: Rendering

/// Renderable form of a category report, independent of any database.
struct Rendered {
    /// Rows in pre-order.
    rows: Vec<bc_core::CategoryRow>,
    /// Legs excluded for a commodity mismatch.
    excluded_postings: usize,
    /// Transactions whose residual could not be attributed.
    ambiguous_transactions: usize,
    /// Commodity the totals are in.
    commodity: String,
    /// Inclusive window start.
    start: jiff::civil::Date,
    /// Exclusive window end.
    end: jiff::civil::Date,
}

impl Rendered {
    /// Formats the report as an indented table with any warnings appended.
    #[expect(
        clippy::let_underscore_must_use,
        reason = "writing into a String via core::fmt::Write is infallible"
    )]
    fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Report: {} – {} ({})",
            self.start, self.end, self.commodity
        );
        out.push('\n');

        if self.rows.is_empty() {
            out.push_str("No activity in this period.\n");
        } else {
            let width = self
                .rows
                .iter()
                .map(|r| {
                    r.depth
                        .saturating_mul(2)
                        .saturating_add(leaf_name(&r.path).len())
                })
                .max()
                .unwrap_or(0)
                .max("ACCOUNT".len());

            let _ = writeln!(out, "{:<width$}  {:>14}", "ACCOUNT", "AMOUNT");
            for row in &self.rows {
                let indent = " ".repeat(row.depth.saturating_mul(2));
                let label = format!("{indent}{}", leaf_name(&row.path));
                let _ = writeln!(out, "{label:<width$}  {:>14}", row.rolled_up.value());
            }
        }

        if self.excluded_postings > 0 {
            let _ = writeln!(
                out,
                "\nnote: {} posting(s) in other commodities were excluded, not converted",
                self.excluded_postings
            );
        }
        if self.ambiguous_transactions > 0 {
            let _ = writeln!(
                out,
                "note: {} transaction(s) carry more than one elided leg and were partly uncounted",
                self.ambiguous_transactions
            );
        }
        out
    }
}

/// Drops rows whose `depth` is at or beyond `n`, in place.
///
/// `depth` counts only emitted ancestors (see [`bc_core::CategoryRow::depth`]),
/// so `n` bounds how many levels of *surviving* rows are shown, not the
/// account tree's absolute depth. A `None` cap leaves `rows` untouched.
fn apply_depth_filter(rows: &mut Vec<bc_core::CategoryRow>, depth: Option<usize>) {
    if let Some(n) = depth {
        rows.retain(|r| r.depth < n);
    }
}

/// Returns the last colon-separated segment of an account path.
fn leaf_name(path: &str) -> &str {
    path.rsplit(':').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::Rendered;
    use super::apply_depth_filter;

    /// Builds a row at `depth` with equal own and rolled-up totals.
    fn leaf(path: &str, depth: usize, value: rust_decimal::Decimal) -> bc_core::CategoryRow {
        bc_core::CategoryRow::new(
            bc_models::AccountId::new(),
            path.to_owned(),
            depth,
            bc_models::Amount::new(value, "AUD"),
            bc_models::Amount::new(value, "AUD"),
        )
    }

    /// Builds a parent row with no own activity.
    fn parent(path: &str, depth: usize, rolled: rust_decimal::Decimal) -> bc_core::CategoryRow {
        bc_core::CategoryRow::new(
            bc_models::AccountId::new(),
            path.to_owned(),
            depth,
            bc_models::Amount::new(rust_decimal::Decimal::ZERO, "AUD"),
            bc_models::Amount::new(rolled, "AUD"),
        )
    }

    #[test]
    fn renders_an_indented_tree() {
        let rendered = Rendered {
            rows: vec![
                parent("Income", 0, dec!(-48000)),
                parent("Income:Interest", 1, dec!(-3000)),
                leaf("Income:Interest:Bank-A", 2, dec!(-2000)),
                leaf("Income:Interest:Bank-B", 2, dec!(-1000)),
                leaf("Income:Rent", 1, dec!(-45000)),
            ],
            excluded_postings: 0,
            ambiguous_transactions: 0,
            commodity: "AUD".to_owned(),
            start: date(2025, 7, 1),
            end: date(2026, 7, 1),
        };
        insta::assert_snapshot!(rendered.render());
    }

    #[test]
    fn renders_the_excluded_commodity_warning() {
        let rendered = Rendered {
            rows: vec![leaf("Assets:Crypto", 0, dec!(0.5))],
            excluded_postings: 12,
            ambiguous_transactions: 3,
            commodity: "AUD".to_owned(),
            start: date(2025, 7, 1),
            end: date(2026, 7, 1),
        };
        insta::assert_snapshot!(rendered.render());
    }

    #[test]
    fn renders_an_empty_report() {
        let rendered = Rendered {
            rows: vec![],
            excluded_postings: 0,
            ambiguous_transactions: 0,
            commodity: "AUD".to_owned(),
            start: date(2025, 7, 1),
            end: date(2026, 7, 1),
        };
        insta::assert_snapshot!(rendered.render());
    }

    #[test]
    fn depth_filter_drops_rows_at_or_beyond_the_cap() {
        let mut rows = vec![
            parent("Income", 0, dec!(-3000)),
            parent("Income:Interest", 1, dec!(-3000)),
            leaf("Income:Interest:Bank-A", 2, dec!(-2000)),
            leaf("Income:Interest:Bank-B", 2, dec!(-1000)),
        ];
        apply_depth_filter(&mut rows, Some(2));

        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Income", "Income:Interest"],
            "depth 2 keeps rows at depth 0 and 1, drops rows at depth 2"
        );
    }

    #[test]
    fn depth_filter_none_leaves_rows_untouched() {
        let mut rows = vec![
            parent("Income", 0, dec!(-3000)),
            leaf("Income:Interest", 1, dec!(-3000)),
        ];
        apply_depth_filter(&mut rows, None);
        assert_eq!(rows.len(), 2, "an unset --depth applies no filter");
    }
}
