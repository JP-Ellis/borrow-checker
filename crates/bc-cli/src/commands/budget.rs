//! Budget management sub-commands: list, create, archive, allocate, status.

use clap::Subcommand;
use jiff::civil::Date;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Arguments for the `budget` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The budget operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available budget operations.
#[non_exhaustive]
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List all active budgets.
    List {
        /// Include archived budgets.
        #[arg(long)]
        archived: bool,
    },
    /// Create a new budget anchored to an account.
    Create {
        /// Account ID to anchor this budget to.
        #[arg(long)]
        account: String,
        /// Optional tag filter ID (descendant-or-equal matching).
        #[arg(long)]
        tag_filter: Option<String>,
        /// Display name (defaults to the account name).
        #[arg(long)]
        name: Option<String>,
        /// Budget target amount per period (omit for tracking-only).
        #[arg(long)]
        target: Option<rust_decimal::Decimal>,
        /// Commodity code for the target (e.g. AUD, USD). Required when --target is set.
        #[arg(long)]
        commodity: Option<String>,
        /// Budget period type.
        #[arg(long, default_value = "monthly")]
        period: PeriodArg,
        /// Anchor date for fortnightly periods (YYYY-MM-DD).
        #[arg(long)]
        anchor: Option<String>,
        /// Financial year start month (1–12).
        #[arg(long)]
        fy_start_month: Option<u8>,
        /// Financial year start day (1–28, default 1).
        #[arg(long, default_value = "1")]
        fy_start_day: u8,
        /// Rollover policy.
        #[arg(long, value_enum, default_value = "reset-to-zero")]
        rollover: RolloverArg,
    },
    /// Archive a budget (hides it; data is preserved).
    Archive {
        /// Budget ID to archive.
        id: String,
    },
    /// Allocate funds to a budget for a period.
    Allocate {
        /// Budget ID to allocate to.
        #[arg(long)]
        budget: String,
        /// Amount to allocate (decimal, e.g. 500 or 499.99).
        #[arg(long)]
        amount: rust_decimal::Decimal,
        /// Commodity code (e.g. AUD, USD).
        #[arg(long)]
        commodity: String,
        /// Period start date (YYYY-MM-DD). Defaults to the current period start.
        #[arg(long)]
        period_start: Option<String>,
    },
    /// Show budget status for all active budgets.
    Status {
        /// Date to evaluate status as of (YYYY-MM-DD). Defaults to today.
        #[arg(long)]
        as_of: Option<String>,
    },
}

/// CLI representation of budget period types.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum PeriodArg {
    /// Weekly period.
    Weekly,
    /// Fortnightly period (requires --anchor).
    Fortnightly,
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct).
    Quarterly,
    /// Financial year (requires --fy-start-month).
    #[value(name = "financial-year")]
    FinancialYear,
    /// Financial quarter aligned to a financial year.
    #[value(name = "financial-quarter")]
    FinancialQuarter,
    /// Calendar year.
    #[value(name = "calendar-year")]
    CalendarYear,
}

/// CLI representation of rollover policies.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum RolloverArg {
    /// Unspent balance carries into next period.
    #[value(name = "carry-forward")]
    CarryForward,
    /// Balance resets each period.
    #[value(name = "reset-to-zero")]
    ResetToZero,
    /// Carry forward, capped at the allocation target.
    #[value(name = "cap-at-target")]
    CapAtTarget,
}

/// Executes the `budget` subcommand.
///
/// # Errors
///
/// Propagates any [`CliError`] from the core engine or output layer.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List { archived } => list(ctx, archived).await,
        Command::Create {
            account,
            tag_filter,
            name,
            target,
            commodity,
            period,
            anchor,
            fy_start_month,
            fy_start_day,
            rollover,
        } => {
            create(
                ctx,
                account,
                tag_filter,
                name,
                target,
                commodity,
                period,
                anchor,
                fy_start_month,
                fy_start_day,
                rollover,
            )
            .await
        }
        Command::Archive { id } => archive(ctx, id).await,
        Command::Allocate {
            budget,
            amount,
            commodity,
            period_start,
        } => allocate(ctx, budget, amount, commodity, period_start).await,
        Command::Status { as_of } => status(ctx, as_of).await,
    }
}

/// List all active (and optionally archived) budgets.
async fn list(ctx: &AppContext, _archived: bool) -> CliResult<()> {
    let budgets = ctx.budgets.list().await?;

    if ctx.json {
        return crate::output::print_json(&budgets);
    }

    if budgets.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No budgets.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = budgets
        .iter()
        .map(|b| {
            let period_str = period_display(b.period());
            let target_str = b.target().map_or_else(
                || "\u{2014}".to_owned(),
                |a| format!("{} {}", a.value(), a.commodity()),
            );
            let rollover_str = match b.rollover() {
                bc_models::RolloverPolicy::CarryForward => "carry-forward",
                bc_models::RolloverPolicy::ResetToZero => "reset-to-zero",
                bc_models::RolloverPolicy::CapAtTarget => "cap-at-target",
                _ => "unknown",
            };
            let name_str = b.name().unwrap_or("\u{2014}").to_owned();
            vec![
                b.id().to_string(),
                b.account_id().to_string(),
                name_str,
                period_str,
                target_str,
                rollover_str.to_owned(),
            ]
        })
        .collect();
    crate::output::print_table(
        &["ID", "ACCOUNT", "NAME", "PERIOD", "TARGET", "ROLLOVER"],
        &rows,
    );
    Ok(())
}

/// Create a new budget anchored to an account.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument maps to a CLI flag"
)]
async fn create(
    ctx: &AppContext,
    account: String,
    tag_filter: Option<String>,
    name: Option<String>,
    target: Option<rust_decimal::Decimal>,
    commodity: Option<String>,
    period_arg: PeriodArg,
    anchor: Option<String>,
    fy_start_month: Option<u8>,
    fy_start_day: u8,
    rollover_arg: RolloverArg,
) -> CliResult<()> {
    use core::str::FromStr as _;

    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::RolloverPolicy;

    let account_id = bc_models::AccountId::from_str(&account)
        .map_err(|e| CliError::Arg(format!("invalid account ID '{account}': {e}")))?;

    let tag_filter_id = tag_filter
        .as_deref()
        .map(|s| {
            bc_models::TagId::from_str(s)
                .map_err(|e| CliError::Arg(format!("invalid tag ID '{s}': {e}")))
        })
        .transpose()?;

    let bc_period = resolve_period(period_arg, anchor, fy_start_month, fy_start_day)?;

    let rollover_policy = match rollover_arg {
        RolloverArg::CarryForward => RolloverPolicy::CarryForward,
        RolloverArg::ResetToZero => RolloverPolicy::ResetToZero,
        RolloverArg::CapAtTarget => RolloverPolicy::CapAtTarget,
    };

    let target_amount = target
        .zip(commodity.as_deref())
        .map(|(amt, c)| Amount::new(amt, CommodityCode::new(c)));

    if target.is_some() && commodity.is_none() {
        return Err(CliError::Arg(
            "--commodity is required when --target is set".to_owned(),
        ));
    }

    let budget = ctx
        .budgets
        .create()
        .account_id(account_id)
        .maybe_tag_filter(tag_filter_id)
        .maybe_name(name)
        .maybe_target(target_amount)
        .period(bc_period)
        .rollover(rollover_policy)
        .call()
        .await?;

    if ctx.json {
        return crate::output::print_json(&budget);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!(
            "Created budget: {} on account {} ({})",
            budget.name().unwrap_or("(unnamed)"),
            budget.account_id(),
            budget.id()
        );
    }
    Ok(())
}

/// Archive a budget by ID, hiding it from active lists while preserving history.
async fn archive(ctx: &AppContext, id: String) -> CliResult<()> {
    use core::str::FromStr as _;
    let budget_id = bc_models::BudgetId::from_str(&id)
        .map_err(|e| CliError::Arg(format!("invalid budget ID '{id}': {e}")))?;
    ctx.budgets.archive(&budget_id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "archived": true, "id": id }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Archived budget: {id}");
    }
    Ok(())
}

/// Allocate funds to a budget for a specific period.
async fn allocate(
    ctx: &AppContext,
    budget: String,
    amount: rust_decimal::Decimal,
    commodity: String,
    period_start_str: Option<String>,
) -> CliResult<()> {
    use core::str::FromStr as _;

    use bc_models::Amount;
    use bc_models::CommodityCode;

    let budget_id = bc_models::BudgetId::from_str(&budget)
        .map_err(|e| CliError::Arg(format!("invalid budget ID '{budget}': {e}")))?;
    let b = ctx.budgets.get(&budget_id).await?;

    let period_start = if let Some(s) = period_start_str {
        let date = s
            .parse::<Date>()
            .map_err(|e| CliError::Arg(format!("invalid period-start '{s}': {e}")))?;
        let canonical = b.period().range_containing(date).0;
        if canonical != date {
            return Err(CliError::Arg(format!(
                "'{date}' is not a canonical period start for this budget's {:?} period; \
                 did you mean '{canonical}'?",
                b.period(),
            )));
        }
        date
    } else {
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date();
        b.period().range_containing(today).0
    };

    let alloc = ctx
        .budgets
        .allocate(
            &budget_id,
            period_start,
            Amount::new(amount, CommodityCode::new(commodity)),
        )
        .await?;

    if ctx.json {
        return crate::output::print_json(&alloc);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!(
            "Allocated {} {} to budget '{}' for period starting {}",
            alloc.amount().value(),
            alloc.amount().commodity(),
            b.name().unwrap_or("(unnamed)"),
            period_start,
        );
    }
    Ok(())
}

/// Show budget status for all active budgets as of a given date.
async fn status(ctx: &AppContext, as_of_str: Option<String>) -> CliResult<()> {
    let as_of = if let Some(s) = as_of_str {
        s.parse::<Date>()
            .map_err(|e| CliError::Arg(format!("invalid as-of date '{s}': {e}")))?
    } else {
        jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date()
    };

    let budgets = ctx.budgets.list().await?;
    let statuses = ctx.budget_status.status_all(&budgets, as_of).await?;

    if ctx.json {
        return crate::output::print_json(&statuses);
    }

    if statuses.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No budgets.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = statuses
        .iter()
        .map(|s| {
            let display_end = s
                .period_end
                .checked_sub(jiff::Span::new().days(1_i32))
                .unwrap_or(s.period_end);
            let period_str = format!("{} \u{2013} {}", s.period_start, display_end);
            let commodity_str = s
                .commodity
                .as_ref()
                .map_or("", bc_models::CommodityCode::as_str);
            let alloc_str = if s.allocated.is_zero() && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                format!("{} {}", s.allocated, commodity_str)
            };
            let actuals_str = format!("{} {}", s.actuals, commodity_str);
            let avail_str = if s.budget.is_tracking_only() && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                format!("{} {}", s.available, commodity_str)
            };
            let name_str = s
                .budget
                .name()
                .map_or_else(|| s.budget.account_id().to_string(), str::to_owned);
            vec![name_str, period_str, alloc_str, actuals_str, avail_str]
        })
        .collect();
    crate::output::print_table(
        &["BUDGET", "PERIOD", "ALLOCATED", "ACTUALS", "AVAILABLE"],
        &rows,
    );
    Ok(())
}

/// Convert CLI period arguments into a [`bc_models::Period`].
fn resolve_period(
    period_arg: PeriodArg,
    anchor: Option<String>,
    fy_start_month: Option<u8>,
    fy_start_day: u8,
) -> CliResult<bc_models::Period> {
    use bc_models::Period;
    match period_arg {
        PeriodArg::Weekly => Ok(Period::Weekly),
        PeriodArg::Monthly => Ok(Period::Monthly),
        PeriodArg::Quarterly => Ok(Period::Quarterly),
        PeriodArg::CalendarYear => Ok(Period::CalendarYear),
        PeriodArg::Fortnightly => {
            let anchor_str = anchor.ok_or_else(|| {
                CliError::Arg("--anchor is required for fortnightly periods".to_owned())
            })?;
            let anchor_date = anchor_str
                .parse::<Date>()
                .map_err(|e| CliError::Arg(format!("invalid anchor date '{anchor_str}': {e}")))?;
            Ok(Period::Fortnightly {
                anchor: anchor_date,
            })
        }
        PeriodArg::FinancialYear => {
            let month = fy_start_month.ok_or_else(|| {
                CliError::Arg("--fy-start-month is required for financial-year periods".to_owned())
            })?;
            bc_models::Period::financial_year(month, fy_start_day)
                .map_err(|e| CliError::Arg(format!("invalid financial year: {e}")))
        }
        PeriodArg::FinancialQuarter => {
            let month = fy_start_month.ok_or_else(|| {
                CliError::Arg(
                    "--fy-start-month is required for financial-quarter periods".to_owned(),
                )
            })?;
            bc_models::Period::financial_quarter(month, fy_start_day)
                .map_err(|e| CliError::Arg(format!("invalid financial quarter: {e}")))
        }
    }
}

/// Format a [`bc_models::Period`] as a human-readable string for table output.
fn period_display(period: &bc_models::Period) -> String {
    use bc_models::Period;
    match period {
        Period::Weekly => "Weekly".to_owned(),
        Period::Fortnightly { anchor } => format!("Fortnightly ({anchor})"),
        Period::Monthly => "Monthly".to_owned(),
        Period::Quarterly => "Quarterly".to_owned(),
        Period::FinancialYear {
            start_month,
            start_day,
        } => {
            format!("FY ({start_month:02}/{start_day:02})")
        }
        Period::FinancialQuarter {
            start_month,
            start_day,
        } => {
            format!("FQ ({start_month:02}/{start_day:02})")
        }
        Period::CalendarYear => "Calendar Year".to_owned(),
        Period::Custom {
            days,
            weeks,
            months,
        } => {
            let mut parts = vec![];
            if let Some(d) = days {
                parts.push(format!("{d}d"));
            }
            if let Some(w) = weeks {
                parts.push(format!("{w}w"));
            }
            if let Some(m) = months {
                parts.push(format!("{m}mo"));
            }
            format!("Custom ({})", parts.join("+"))
        }
        _ => {
            tracing::warn!("unrecognised Period variant in period_display");
            "Unknown".to_owned()
        }
    }
}
