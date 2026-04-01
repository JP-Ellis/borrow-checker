//! Budget management sub-commands: groups, envelopes, allocate, status.

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
    /// Manage envelope groups.
    Groups {
        /// The group operation to perform.
        #[command(subcommand)]
        command: GroupCommand,
    },
    /// Manage budget envelopes.
    Envelopes {
        /// The envelope operation to perform.
        #[command(subcommand)]
        command: EnvelopeCommand,
    },
    /// Allocate funds to an envelope for a period.
    Allocate {
        /// Envelope ID to allocate to.
        #[arg(long)]
        envelope: String,
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
    /// Show budget status for all envelopes.
    Status {
        /// Date to evaluate status as of (YYYY-MM-DD). Defaults to today.
        #[arg(long)]
        as_of: Option<String>,
    },
}

/// Available envelope group operations.
#[non_exhaustive]
#[derive(Debug, Subcommand)]
pub enum GroupCommand {
    /// List all active envelope groups.
    List,
    /// Create a new envelope group.
    Create {
        /// Display name for the group.
        #[arg(long)]
        name: String,
        /// Parent group ID (optional).
        #[arg(long)]
        parent: Option<String>,
    },
}

/// Available envelope operations.
#[non_exhaustive]
#[derive(Debug, Subcommand)]
pub enum EnvelopeCommand {
    /// List all active envelopes.
    List,
    /// Create a new budget envelope.
    Create {
        /// Display name.
        #[arg(long)]
        name: String,
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
        /// Commodity code for this envelope (e.g. `AUD`, `USD`).
        #[arg(long)]
        commodity: String,
        /// Budget target amount per period.
        #[arg(long)]
        target: Option<rust_decimal::Decimal>,
        /// Group ID to assign this envelope to.
        #[arg(long)]
        group: Option<String>,
        /// Display icon (emoji or name).
        #[arg(long)]
        icon: Option<String>,
        /// Display colour (e.g. `#4CAF50`).
        #[arg(long)]
        colour: Option<String>,
    },
    /// Archive an envelope (hides it; data is preserved).
    Archive {
        /// Envelope ID to archive.
        id: String,
    },
    /// Move an envelope to a different group, or remove it from all groups.
    Move {
        /// Envelope ID to move.
        id: String,
        /// Target group ID, or omit to remove from all groups.
        #[arg(long)]
        group: Option<String>,
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
        Command::Groups { command } => groups(command, ctx).await,
        Command::Envelopes { command } => envelopes(command, ctx).await,
        Command::Allocate {
            envelope,
            amount,
            commodity,
            period_start,
        } => allocate(ctx, envelope, amount, commodity, period_start).await,
        Command::Status { as_of } => status(ctx, as_of).await,
    }
}

// ── Groups ────────────────────────────────────────────────────────────────────

/// Dispatches envelope group sub-commands.
async fn groups(cmd: GroupCommand, ctx: &AppContext) -> CliResult<()> {
    match cmd {
        GroupCommand::List => groups_list(ctx).await,
        GroupCommand::Create { name, parent } => groups_create(ctx, name, parent).await,
    }
}

/// Lists all active envelope groups in a table (or JSON).
async fn groups_list(ctx: &AppContext) -> CliResult<()> {
    let groups = ctx.envelopes.list_groups().await?;

    if ctx.json {
        return crate::output::print_json(&groups);
    }

    if groups.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No envelope groups.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            let parent = g.parent_id().map(ToString::to_string).unwrap_or_default();
            vec![g.id().to_string(), g.name().to_owned(), parent]
        })
        .collect();
    crate::output::print_table(&["ID", "NAME", "PARENT"], &rows);
    Ok(())
}

/// Creates a new envelope group and prints the result.
async fn groups_create(ctx: &AppContext, name: String, parent: Option<String>) -> CliResult<()> {
    use core::str::FromStr as _;
    let parent_id = parent
        .as_deref()
        .map(|s| {
            bc_models::EnvelopeGroupId::from_str(s)
                .map_err(|e| CliError::Arg(format!("invalid group ID '{s}': {e}")))
        })
        .transpose()?;

    let group = ctx
        .envelopes
        .create_group(&name, parent_id.as_ref())
        .await?;

    if ctx.json {
        return crate::output::print_json(&group);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created group: {} ({})", group.name(), group.id());
    }
    Ok(())
}

// ── Envelopes ─────────────────────────────────────────────────────────────────

/// Dispatches envelope sub-commands.
async fn envelopes(cmd: EnvelopeCommand, ctx: &AppContext) -> CliResult<()> {
    match cmd {
        EnvelopeCommand::List => envelopes_list(ctx).await,
        EnvelopeCommand::Create {
            name,
            period,
            anchor,
            fy_start_month,
            fy_start_day,
            rollover,
            commodity,
            target,
            group,
            icon,
            colour,
        } => {
            envelopes_create(
                ctx,
                name,
                period,
                anchor,
                fy_start_month,
                fy_start_day,
                rollover,
                commodity,
                target,
                group,
                icon,
                colour,
            )
            .await
        }
        EnvelopeCommand::Archive { id } => envelopes_archive(ctx, id).await,
        EnvelopeCommand::Move { id, group } => envelopes_move(ctx, id, group).await,
    }
}

/// Lists all active envelopes in a table (or JSON).
async fn envelopes_list(ctx: &AppContext) -> CliResult<()> {
    let envs = ctx.envelopes.list().await?;

    if ctx.json {
        return crate::output::print_json(&envs);
    }

    if envs.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No envelopes.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = envs
        .iter()
        .map(|env| {
            let period_str = period_display(env.period());
            let target_str = env.allocation_target().map_or_else(
                || "\u{2014}".to_owned(),
                |a| format!("{} {}", a.value(), a.commodity()),
            );
            let rollover_str = match env.rollover_policy() {
                bc_models::RolloverPolicy::CarryForward => "carry-forward",
                bc_models::RolloverPolicy::ResetToZero => "reset-to-zero",
                bc_models::RolloverPolicy::CapAtTarget => "cap-at-target",
                _ => "unknown",
            };
            vec![
                env.id().to_string(),
                env.name().to_owned(),
                period_str,
                target_str,
                rollover_str.to_owned(),
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "NAME", "PERIOD", "TARGET", "ROLLOVER"], &rows);
    Ok(())
}

/// Creates a new envelope with the given parameters and prints the result.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument maps to a CLI flag"
)]
async fn envelopes_create(
    ctx: &AppContext,
    name: String,
    period_arg: PeriodArg,
    anchor: Option<String>,
    fy_start_month: Option<u8>,
    fy_start_day: u8,
    rollover_arg: RolloverArg,
    commodity: String,
    target: Option<rust_decimal::Decimal>,
    group: Option<String>,
    icon: Option<String>,
    colour: Option<String>,
) -> CliResult<()> {
    use core::str::FromStr as _;

    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::RolloverPolicy;

    let bc_period = resolve_period(period_arg, anchor, fy_start_month, fy_start_day)?;
    let rollover_policy = match rollover_arg {
        RolloverArg::CarryForward => RolloverPolicy::CarryForward,
        RolloverArg::ResetToZero => RolloverPolicy::ResetToZero,
        RolloverArg::CapAtTarget => RolloverPolicy::CapAtTarget,
    };
    let allocation_target =
        target.map(|amt| Amount::new(amt, CommodityCode::new(commodity.clone())));
    let envelope_commodity = CommodityCode::new(commodity);
    let group_id = group
        .as_deref()
        .map(|s| {
            bc_models::EnvelopeGroupId::from_str(s)
                .map_err(|e| CliError::Arg(format!("invalid group ID '{s}': {e}")))
        })
        .transpose()?;

    let env = ctx
        .envelopes
        .create(bc_core::EnvelopeCreateParams::new(
            name.clone(),
            group_id,
            icon,
            colour,
            envelope_commodity,
            allocation_target,
            bc_period,
            rollover_policy,
            vec![],
        ))
        .await?;

    if ctx.json {
        return crate::output::print_json(&env);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created envelope: {} ({})", env.name(), env.id());
    }
    Ok(())
}

/// Archives an envelope by ID and prints a confirmation.
async fn envelopes_archive(ctx: &AppContext, id: String) -> CliResult<()> {
    use core::str::FromStr as _;
    let env_id = bc_models::EnvelopeId::from_str(&id)
        .map_err(|e| CliError::Arg(format!("invalid envelope ID '{id}': {e}")))?;
    ctx.envelopes.archive(&env_id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "archived": true, "id": id }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Archived envelope: {id}");
    }
    Ok(())
}

/// Moves an envelope to a different group, or removes it from all groups.
async fn envelopes_move(ctx: &AppContext, id: String, group: Option<String>) -> CliResult<()> {
    use core::str::FromStr as _;

    let env_id = bc_models::EnvelopeId::from_str(&id)
        .map_err(|e| CliError::Arg(format!("invalid envelope ID '{id}': {e}")))?;
    let group_id = group
        .as_deref()
        .map(|s| {
            bc_models::EnvelopeGroupId::from_str(s)
                .map_err(|e| CliError::Arg(format!("invalid group ID '{s}': {e}")))
        })
        .transpose()?;

    let env = ctx
        .envelopes
        .move_to_group(&env_id, group_id.as_ref())
        .await?;

    if ctx.json {
        return crate::output::print_json(&env);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        match env.parent_id() {
            Some(gid) => println!("Moved envelope {} to group {gid}", env.id()),
            None => println!("Moved envelope {} to root (no group)", env.id()),
        }
    }
    Ok(())
}

// ── Allocate ──────────────────────────────────────────────────────────────────

/// Allocates funds to an envelope and prints a confirmation.
async fn allocate(
    ctx: &AppContext,
    envelope: String,
    amount: rust_decimal::Decimal,
    commodity: String,
    period_start_str: Option<String>,
) -> CliResult<()> {
    use core::str::FromStr as _;

    use bc_models::Amount;
    use bc_models::CommodityCode;

    let env_id = bc_models::EnvelopeId::from_str(&envelope)
        .map_err(|e| CliError::Arg(format!("invalid envelope ID '{envelope}': {e}")))?;

    let env = ctx.envelopes.get(&env_id).await?;

    let period_start = if let Some(s) = period_start_str {
        let date = s
            .parse::<Date>()
            .map_err(|e| CliError::Arg(format!("invalid period-start '{s}': {e}")))?;
        let canonical = env.period().range_containing(date).0;
        if canonical != date {
            return Err(CliError::Arg(format!(
                "'{date}' is not a canonical period start for this envelope's {:?} period; \
                 did you mean '{canonical}'?",
                env.period(),
            )));
        }
        date
    } else {
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date();
        env.period().range_containing(today).0
    };

    let alloc = ctx
        .envelopes
        .allocate(
            &env_id,
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
            "Allocated {} {} to '{}' for period starting {}",
            alloc.amount().value(),
            alloc.amount().commodity(),
            env.name(),
            period_start,
        );
    }
    Ok(())
}

// ── Status ────────────────────────────────────────────────────────────────────

/// Prints the budget status for all envelopes as of a given date.
async fn status(ctx: &AppContext, as_of_str: Option<String>) -> CliResult<()> {
    let as_of = if let Some(s) = as_of_str {
        s.parse::<Date>()
            .map_err(|e| CliError::Arg(format!("invalid as-of date '{s}': {e}")))?
    } else {
        jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date()
    };

    let envs = ctx.envelopes.list().await?;
    let statuses = ctx.budget.status_all(&envs, as_of).await?;

    if ctx.json {
        return crate::output::print_json(&statuses);
    }

    if statuses.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No envelopes.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = statuses
        .iter()
        .map(|s| {
            // period_end is exclusive; subtract one day for the inclusive human-readable range.
            let display_end = s
                .period_end
                .checked_sub(jiff::Span::new().days(1_i32))
                .unwrap_or(s.period_end);
            let period_str = format!("{} \u{2013} {}", s.period_start, display_end);
            let alloc_str = if s.allocated.is_zero() && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                format!("{} {}", s.allocated, s.commodity)
            };
            let actuals_str = format!("{} {}", s.actuals, s.commodity);
            let avail_str = if s.envelope.is_tracking_only() && s.rollover.is_zero() {
                "\u{2014}".to_owned()
            } else {
                format!("{} {}", s.available, s.commodity)
            };
            vec![
                s.envelope.name().to_owned(),
                period_str,
                alloc_str,
                actuals_str,
                avail_str,
            ]
        })
        .collect();
    crate::output::print_table(
        &["ENVELOPE", "PERIOD", "ALLOCATED", "ACTUALS", "AVAILABLE"],
        &rows,
    );
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Converts CLI period arguments into a [`bc_models::Period`].
///
/// # Errors
///
/// Returns [`CliError::Arg`] if required arguments (anchor, fy-start-month) are
/// missing or if the provided date/period values are invalid.
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

/// Returns a short human-readable label for a period.
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
            tracing::warn!(
                "unrecognised Period variant in period_display — \
                 add a match arm if a new Period variant was introduced"
            );
            "Unknown".to_owned()
        }
    }
}
