//! Budget management sub-commands: list, create, archive, status.

use core::str::FromStr as _;

use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Period;
use bc_models::RolloverPolicy;
use clap::Subcommand;
use jiff::civil::Date;

use crate::commands::parse_date_or_today;
use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;
use crate::period::PeriodArg;
use crate::period::PeriodInputs;

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
    List,
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
        /// Days component of a custom period duration.
        #[arg(long)]
        duration_days: Option<u32>,
        /// Weeks component of a custom period duration.
        #[arg(long)]
        duration_weeks: Option<u32>,
        /// Months component of a custom period duration.
        #[arg(long)]
        duration_months: Option<u32>,
        /// Financial year start month (1–12).
        #[arg(long)]
        fy_start_month: Option<u8>,
        /// Financial year start day (1–28, default 1).
        #[arg(long, default_value = "1")]
        fy_start_day: u8,
        /// Rollover policy.
        #[arg(long, value_enum, default_value = "reset-to-zero")]
        rollover: RolloverArg,
        /// Date the initial revision takes effect (YYYY-MM-DD); defaults to today.
        #[arg(long)]
        effective: Option<String>,
    },
    /// Archive a budget (hides it; data is preserved).
    Archive {
        /// Budget ID to archive.
        id: String,
    },
    /// Show budget status for all active budgets.
    Status {
        /// Date to evaluate status as of (YYYY-MM-DD). Defaults to today.
        #[arg(long)]
        as_of: Option<String>,
    },
    /// Update a budget's name, target, or rollover policy.
    Update {
        /// Budget ID to update.
        #[arg(long)]
        id: String,
        /// New display name (omit to keep existing).
        #[arg(long)]
        name: Option<String>,
        /// Clear the display name.
        #[arg(long, conflicts_with = "name")]
        clear_name: bool,
        /// New target amount per period.
        #[arg(long)]
        target: Option<rust_decimal::Decimal>,
        /// Commodity code for the new target (required when --target is set).
        #[arg(long)]
        commodity: Option<String>,
        /// Clear the allocation target (make tracking-only).
        #[arg(long, conflicts_with_all = ["target", "commodity"])]
        clear_target: bool,
        /// New rollover policy.
        #[arg(long, value_enum)]
        rollover: Option<RolloverArg>,
    },
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
        Command::List => list(ctx).await,
        Command::Create {
            account,
            tag_filter,
            name,
            target,
            commodity,
            period,
            duration_days,
            duration_weeks,
            duration_months,
            fy_start_month,
            fy_start_day,
            rollover,
            effective,
        } => {
            create(
                ctx,
                account,
                tag_filter,
                name,
                target,
                commodity,
                period,
                duration_days,
                duration_weeks,
                duration_months,
                fy_start_month,
                fy_start_day,
                rollover,
                effective,
            )
            .await
        }
        Command::Archive { id } => archive(ctx, id).await,
        Command::Update {
            id,
            name,
            clear_name,
            target,
            commodity,
            clear_target,
            rollover,
        } => {
            update_budget(
                ctx,
                id,
                name,
                clear_name,
                target,
                commodity,
                clear_target,
                rollover,
            )
            .await
        }
        Command::Status { as_of } => status(ctx, as_of).await,
    }
}

/// List all active budgets.
async fn list(ctx: &AppContext) -> CliResult<()> {
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

    let today = jiff::Zoned::now().date();
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(budgets.len());
    for b in &budgets {
        let revs = ctx.budgets.revisions(b.id()).await?;
        let rev = bc_core::governing_revision(&revs, today);
        let period_str = rev.map_or_else(|| "\u{2014}".to_owned(), |r| period_display(r.period()));
        let target_str = rev.and_then(bc_models::BudgetRevision::target).map_or_else(
            || "\u{2014}".to_owned(),
            |a| format!("{} {}", a.value(), a.commodity()),
        );
        let rollover_str = rev.map_or("\u{2014}", |r| match r.rollover() {
            bc_models::RolloverPolicy::CarryForward => "carry-forward",
            bc_models::RolloverPolicy::ResetToZero => "reset-to-zero",
            bc_models::RolloverPolicy::CapAtTarget => "cap-at-target",
            _ => "unknown",
        });
        let name_str = rev
            .and_then(bc_models::BudgetRevision::name)
            .unwrap_or("\u{2014}")
            .to_owned();
        rows.push(vec![
            b.id().to_string(),
            b.account_id().to_string(),
            name_str,
            period_str,
            target_str,
            rollover_str.to_owned(),
        ]);
    }
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
    duration_days: Option<u32>,
    duration_weeks: Option<u32>,
    duration_months: Option<u32>,
    fy_start_month: Option<u8>,
    fy_start_day: u8,
    rollover_arg: RolloverArg,
    effective: Option<String>,
) -> CliResult<()> {
    let account_id = bc_models::AccountId::from_str(&account)
        .map_err(|e| CliError::Arg(format!("invalid account ID '{account}': {e}")))?;

    let tag_filter_id = tag_filter
        .as_deref()
        .map(|s| {
            bc_models::TagId::from_str(s)
                .map_err(|e| CliError::Arg(format!("invalid tag ID '{s}': {e}")))
        })
        .transpose()?;

    let bc_period = crate::period::resolve(
        period_arg,
        &PeriodInputs {
            fortnightly_anchor: ctx.fortnightly_anchor,
            duration_days,
            duration_weeks,
            duration_months,
            fy_start_month: fy_start_month.unwrap_or(7),
            fy_start_day,
        },
    )?;

    let rollover_policy = match rollover_arg {
        RolloverArg::CarryForward => RolloverPolicy::CarryForward,
        RolloverArg::ResetToZero => RolloverPolicy::ResetToZero,
        RolloverArg::CapAtTarget => RolloverPolicy::CapAtTarget,
    };

    if target.is_some() && commodity.is_none() {
        return Err(CliError::Arg(
            "--commodity is required when --target is set".to_owned(),
        ));
    }
    if commodity.is_some() && target.is_none() {
        return Err(CliError::Arg(
            "--target is required when --commodity is set".to_owned(),
        ));
    }

    let target_amount = target
        .zip(commodity.as_deref())
        .map(|(amt, c)| Amount::new(amt, CommodityCode::new(c)));

    let effective_from = parse_date_or_today(effective.as_deref())?;
    let (budget, revision) = ctx
        .budgets
        .create()
        .account_id(account_id)
        .effective_from(effective_from)
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
            revision.name().unwrap_or("(unnamed)"),
            budget.account_id(),
            budget.id()
        );
    }
    Ok(())
}

/// Archive a budget by ID, hiding it from active lists while preserving history.
async fn archive(ctx: &AppContext, id: String) -> CliResult<()> {
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

/// Show budget status for all active budgets as of a given date.
async fn status(ctx: &AppContext, as_of_str: Option<String>) -> CliResult<()> {
    let as_of = if let Some(s) = as_of_str {
        s.parse::<Date>()
            .map_err(|e| CliError::Arg(format!("invalid as-of date '{s}': {e}")))?
    } else {
        jiff::Zoned::now().date()
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
                .window
                .end
                .checked_sub(jiff::Span::new().days(1_i32))
                .unwrap_or(s.window.end);
            let period_str = format!("{} \u{2013} {}", s.window.start, display_end);
            let commodity_str = s
                .commodity
                .as_ref()
                .map_or("", bc_models::CommodityCode::as_str);
            let fmt = |v: bc_models::Decimal| {
                if commodity_str.is_empty() {
                    v.to_string()
                } else {
                    format!("{v} {commodity_str}")
                }
            };
            let is_tracking_only = s
                .governing
                .as_ref()
                .is_none_or(bc_models::BudgetRevision::is_tracking_only);
            let alloc_str = if s.allocated.is_zero() && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                fmt(s.allocated)
            };
            let actuals_str = fmt(s.actuals);
            let avail_str = if is_tracking_only && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                fmt(s.available)
            };
            let name_str = s
                .governing
                .as_ref()
                .and_then(bc_models::BudgetRevision::name)
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

/// Update a budget's name, target, or rollover policy.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument maps to a CLI flag"
)]
async fn update_budget(
    ctx: &AppContext,
    id_str: String,
    name: Option<String>,
    clear_name: bool,
    target: Option<rust_decimal::Decimal>,
    commodity: Option<String>,
    clear_target: bool,
    rollover: Option<RolloverArg>,
) -> CliResult<()> {
    let id = bc_models::BudgetId::from_str(&id_str)
        .map_err(|e| CliError::Arg(format!("invalid budget id '{id_str}': {e}")))?;

    let today = jiff::Zoned::now().date();
    let revs = ctx.budgets.revisions(&id).await?;
    let base_rev = bc_core::governing_revision(&revs, today)
        .or_else(|| revs.first())
        .ok_or_else(|| {
            CliError::Core(bc_core::BcError::NotFound(format!(
                "no revision found for budget {id}"
            )))
        })?;

    let new_name: Option<String> = if clear_name {
        None
    } else {
        name.or_else(|| base_rev.name().map(str::to_owned))
    };

    let new_target: Option<bc_models::Amount> = if clear_target {
        None
    } else if let Some(dec) = target {
        let code = commodity
            .as_deref()
            .ok_or_else(|| CliError::Arg("--commodity is required when --target is set".into()))?;
        Some(bc_models::Amount::new(
            dec,
            bc_models::CommodityCode::new(code),
        ))
    } else {
        base_rev.target().cloned()
    };

    let new_rollover = rollover.map_or_else(
        || base_rev.rollover(),
        |r| match r {
            RolloverArg::CarryForward => bc_models::RolloverPolicy::CarryForward,
            RolloverArg::ResetToZero => bc_models::RolloverPolicy::ResetToZero,
            RolloverArg::CapAtTarget => bc_models::RolloverPolicy::CapAtTarget,
        },
    );

    let revised = bc_models::BudgetRevision::builder()
        .id(base_rev.id().clone())
        .budget_id(base_rev.budget_id().clone())
        .effective_from(base_rev.effective_from())
        .maybe_name(new_name)
        .maybe_target(new_target)
        .period(base_rev.period().clone())
        .rollover(new_rollover)
        .maybe_tag_filter(base_rev.tag_filter().cloned())
        .created_at(*base_rev.created_at())
        .build();

    let updated = ctx
        .budgets
        .revise(&id, revised)
        .await
        .map_err(CliError::Core)?;

    if ctx.json {
        return crate::output::print_json(&updated);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Updated budget {id}");
        if let Some(n) = updated.name() {
            println!("  name:    {n}");
        }
        if let Some(t) = updated.target() {
            println!("  target:  {} {}", t.value(), t.commodity());
        } else {
            println!("  target:  (tracking-only)");
        }
        let rollover_str = match updated.rollover() {
            bc_models::RolloverPolicy::CarryForward => "carry-forward",
            bc_models::RolloverPolicy::ResetToZero => "reset-to-zero",
            bc_models::RolloverPolicy::CapAtTarget => "cap-at-target",
            _ => "unknown",
        };
        println!("  rollover: {rollover_str}");
    }

    Ok(())
}

/// Format a [`bc_models::Period`] as a human-readable string for table output.
fn period_display(period: &bc_models::Period) -> String {
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
