//! Asset management sub-commands: record-valuation, depreciate, set-loan-terms, amortization, book-value.

use core::str::FromStr as _;

use bc_models::AccountId;
use bc_models::ValuationSource;
use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `asset` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The asset operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available asset operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Record a point-in-time market valuation for a `ManualAsset` account.
    RecordValuation {
        /// Account ID to record a valuation for.
        #[arg(long)]
        account: String,
        /// Market value (decimal, in the account's commodity).
        #[arg(long)]
        amount: String,
        /// Commodity code (e.g. AUD).
        #[arg(long)]
        commodity: String,
        /// Valuation source.
        #[arg(long, value_enum)]
        source: SourceArg,
        /// Business date of the valuation (YYYY-MM-DD, defaults to today).
        #[arg(long, value_name = "YYYY-MM-DD")]
        date: Option<String>,
        /// Optional counterpart account ID for the auto-balancing transaction.
        #[arg(long)]
        counterpart: Option<String>,
    },
    /// Calculate and record depreciation for a `ManualAsset` account.
    Depreciate {
        /// Account ID to depreciate.
        #[arg(long)]
        account: String,
        /// Commodity code (e.g. AUD).
        #[arg(long)]
        commodity: String,
        /// End date of the depreciation period (YYYY-MM-DD, defaults to today).
        #[arg(long, value_name = "YYYY-MM-DD")]
        date: Option<String>,
        /// Expense account ID to debit (e.g. an `Expenses:Depreciation:*` account).
        #[arg(long)]
        expense_account: String,
    },
    /// Set or update loan terms for a Receivable account.
    SetLoanTerms {
        /// Account ID to attach loan terms to.
        #[arg(long)]
        account: String,
        /// Original principal amount.
        #[arg(long)]
        principal: String,
        /// Annual interest rate as a fraction (e.g. 0.065 = 6.5%).
        #[arg(long)]
        rate: String,
        /// Loan start date (YYYY-MM-DD).
        #[arg(long, value_name = "YYYY-MM-DD")]
        start: String,
        /// Total term in months.
        #[arg(long)]
        term_months: u32,
        /// Repayment frequency.
        #[arg(long, value_enum, default_value = "monthly")]
        frequency: FrequencyArg,
        /// Commodity code (e.g. AUD).
        #[arg(long)]
        commodity: String,
        /// Number of days in a custom repayment period (required when --frequency custom).
        #[arg(long, required_if_eq("frequency", "custom"))]
        period_days: Option<u32>,
        /// How interest compounds (default: daily, standard AU mortgage).
        #[arg(long, value_enum, default_value = "daily")]
        compounding_frequency: CompoundingFrequencyArg,
        /// Offset account IDs (may be repeated).
        #[arg(long = "offset-account", value_name = "ACCOUNT_ID")]
        offset_accounts: Vec<String>,
    },
    /// Display the full amortization schedule for a Receivable account.
    Amortization {
        /// Account ID with loan terms set.
        #[arg(long)]
        account: String,
    },
    /// Display the book value for a `ManualAsset` account.
    ///
    /// `book_value = acquisition_cost - SUM(recorded depreciation amounts)`.
    /// Returns nothing if no acquisition cost has been set.
    BookValue {
        /// Account ID to query.
        #[arg(long)]
        account: String,
        /// Commodity code (e.g. AUD).
        #[arg(long)]
        commodity: String,
    },
}

/// CLI representation of [`bc_models::ValuationSource`].
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum SourceArg {
    /// An estimate by the owner.
    #[value(name = "manual-estimate")]
    ManualEstimate,
    /// A formal appraisal by a qualified professional.
    #[value(name = "professional-appraisal")]
    ProfessionalAppraisal,
    /// A government tax assessment.
    #[value(name = "tax-assessment")]
    TaxAssessment,
    /// Market data (exchange price, comparable sales).
    #[value(name = "market-data")]
    MarketData,
    /// An agreed value between parties.
    #[value(name = "agreed-value")]
    AgreedValue,
}

impl From<SourceArg> for ValuationSource {
    #[inline]
    fn from(arg: SourceArg) -> Self {
        match arg {
            SourceArg::ManualEstimate => Self::ManualEstimate,
            SourceArg::ProfessionalAppraisal => Self::ProfessionalAppraisal,
            SourceArg::TaxAssessment => Self::TaxAssessment,
            SourceArg::MarketData => Self::MarketData,
            SourceArg::AgreedValue => Self::AgreedValue,
        }
    }
}

/// CLI representation of [`bc_models::Period`] repayment frequency.
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum FrequencyArg {
    /// Weekly payments.
    Weekly,
    /// Fortnightly payments.
    Fortnightly,
    /// Monthly payments.
    Monthly,
    /// Quarterly payments.
    Quarterly,
    /// A custom repayment period.
    #[value(name = "custom")]
    Custom,
}

/// CLI representation of [`bc_models::CompoundingFrequency`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CompoundingFrequencyArg {
    /// Daily interest accrual (standard for Australian mortgages).
    Daily,
    /// Traditional: one compounding event per repayment period.
    Monthly,
}

/// Executes the `asset` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or output layer.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::RecordValuation {
            account,
            amount,
            commodity,
            source,
            date,
            counterpart,
        } => record_valuation(ctx, account, amount, commodity, source, date, counterpart).await,
        Command::Depreciate {
            account,
            commodity,
            date,
            expense_account,
        } => depreciate(ctx, account, commodity, date, expense_account).await,
        Command::SetLoanTerms {
            account,
            principal,
            rate,
            start,
            term_months,
            frequency,
            commodity,
            period_days,
            compounding_frequency,
            offset_accounts,
        } => {
            set_loan_terms(
                ctx,
                account,
                principal,
                rate,
                start,
                term_months,
                frequency,
                commodity,
                period_days,
                compounding_frequency,
                offset_accounts,
            )
            .await
        }
        Command::Amortization { account } => amortization(ctx, account).await,
        Command::BookValue { account, commodity } => book_value(ctx, account, commodity).await,
    }
}

/// Records a market valuation for a [`bc_models::AccountKind::ManualAsset`] account.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] for invalid IDs, amounts, or dates.
/// Propagates [`crate::error::CliError::Core`] from the asset service.
async fn record_valuation(
    ctx: &AppContext,
    account: String,
    amount: String,
    commodity: String,
    source: SourceArg,
    date: Option<String>,
    counterpart: Option<String>,
) -> CliResult<()> {
    let account_id = AccountId::from_str(&account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;

    let market_value = amount
        .parse::<rust_decimal::Decimal>()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid amount '{amount}': {e}")))?;

    let recorded_at = parse_date_or_today(date.as_deref())?;

    let counterpart_id = counterpart
        .as_deref()
        .map(AccountId::from_str)
        .transpose()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid counterpart ID: {e}")))?;

    let valuation_id = ctx
        .assets
        .record_valuation(
            &account_id,
            market_value,
            &commodity,
            source.into(),
            recorded_at,
            counterpart_id.as_ref(),
        )
        .await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "valuation_id": valuation_id.to_string(),
            "account_id": account_id.to_string(),
            "market_value": market_value.to_string(),
            "commodity": commodity,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!(
            "Valuation recorded: {market_value} {commodity} on {recorded_at} (ID: {valuation_id})"
        );
    }
    Ok(())
}

/// Calculates and records depreciation for a [`bc_models::AccountKind::ManualAsset`] account.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] for invalid IDs or dates.
/// Propagates [`crate::error::CliError::Core`] from the asset service.
#[inline]
async fn depreciate(
    ctx: &AppContext,
    account: String,
    commodity: String,
    date: Option<String>,
    expense_account: String,
) -> CliResult<()> {
    let account_id = AccountId::from_str(&account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;
    let expense_id = AccountId::from_str(&expense_account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid expense_account ID: {e}")))?;
    let as_of = parse_date_or_today(date.as_deref())?;

    ctx.assets
        .record_depreciation(&account_id, &commodity, as_of, &expense_id)
        .await?;

    if ctx.json {
        return crate::output::print_json(
            &serde_json::json!({ "depreciated": true, "account_id": account, "as_of": as_of.to_string() }),
        );
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Depreciation recorded for {account} up to {as_of}");
    }
    Ok(())
}

/// Sets or updates loan terms for a Receivable account.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] for invalid IDs, amounts, or dates.
/// Propagates [`crate::error::CliError::Core`] from the loan service.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters come from CLI flags"
)]
async fn set_loan_terms(
    ctx: &AppContext,
    account: String,
    principal: String,
    rate: String,
    start: String,
    term_months: u32,
    frequency: FrequencyArg,
    commodity: String,
    period_days: Option<u32>,
    compounding_frequency: CompoundingFrequencyArg,
    offset_accounts: Vec<String>,
) -> CliResult<()> {
    let account_id = AccountId::from_str(&account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;

    let principal_val = principal.parse::<rust_decimal::Decimal>().map_err(|e| {
        crate::error::CliError::Arg(format!("invalid principal '{principal}': {e}"))
    })?;

    let annual_rate = rate
        .parse::<rust_decimal::Decimal>()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid rate '{rate}': {e}")))?;

    let start_date = jiff::civil::Date::from_str(&start)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid start date '{start}': {e}")))?;

    let repayment_frequency: bc_models::Period = match frequency {
        FrequencyArg::Weekly => bc_models::Period::Weekly,
        FrequencyArg::Fortnightly => bc_models::Period::Fortnightly { anchor: start_date },
        FrequencyArg::Monthly => bc_models::Period::Monthly,
        FrequencyArg::Quarterly => bc_models::Period::Quarterly,
        FrequencyArg::Custom => {
            let days = period_days.ok_or_else(|| {
                crate::error::CliError::Arg("--period-days required when --frequency custom".into())
            })?;
            if days == 0 {
                return Err(crate::error::CliError::Arg(
                    "--period-days must be at least 1 when --frequency custom".into(),
                ));
            }
            bc_models::Period::Custom {
                days: Some(days),
                weeks: None,
                months: None,
            }
        }
    };

    let compounding = match compounding_frequency {
        CompoundingFrequencyArg::Daily => bc_models::CompoundingFrequency::Daily,
        CompoundingFrequencyArg::Monthly => bc_models::CompoundingFrequency::Monthly,
    };

    let offset_account_ids = offset_accounts
        .into_iter()
        .map(|s| {
            s.parse::<AccountId>()
                .map_err(|e| crate::error::CliError::Arg(format!("invalid account id '{s}': {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let terms = bc_models::LoanTerms::builder()
        .account_id(account_id.clone())
        .principal(principal_val)
        .annual_rate(annual_rate)
        .start_date(start_date)
        .term_months(term_months)
        .repayment_frequency(repayment_frequency)
        .compounding_frequency(compounding)
        .offset_account_ids(offset_account_ids)
        .commodity(commodity)
        .build();

    ctx.loans.set_loan_terms(&terms).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "set": true,
            "account_id": account,
            "term_months": term_months,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Loan terms set for account: {account}");
    }
    Ok(())
}

/// Displays the full amortization schedule for a Receivable account.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] for an invalid account ID.
/// Propagates [`crate::error::CliError::Core`] from the loan service.
async fn amortization(ctx: &AppContext, account: String) -> CliResult<()> {
    let account_id = AccountId::from_str(&account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;

    // Load offset balances for projection.
    let offset_balances = {
        let mut map = std::collections::HashMap::new();
        if let Some(ref terms) = ctx.loans.loan_terms_for(&account_id).await? {
            for offset_id in terms.offset_account_ids() {
                let bal = ctx
                    .balances
                    .balance_for(offset_id, terms.commodity())
                    .await?;
                map.insert(offset_id.clone(), bal);
            }
        }
        map
    };

    let schedule = ctx
        .loans
        .amortization_schedule(&account_id, offset_balances)
        .await?;

    if ctx.json {
        return crate::output::print_json(&schedule);
    }

    if schedule.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No amortization schedule (no loan terms set).");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = schedule
        .iter()
        .map(|r| {
            vec![
                r.payment_number.to_string(),
                r.date.to_string(),
                r.total_payment.to_string(),
                r.principal.to_string(),
                r.interest.to_string(),
                r.remaining_balance.to_string(),
            ]
        })
        .collect();

    crate::output::print_table(
        &["#", "DATE", "TOTAL", "PRINCIPAL", "INTEREST", "BALANCE"],
        &rows,
    );
    Ok(())
}

/// Displays the book value for a [`bc_models::AccountKind::ManualAsset`] account.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] for an invalid account ID.
/// Propagates [`crate::error::CliError::Core`] from the asset service.
async fn book_value(ctx: &AppContext, account: String, commodity: String) -> CliResult<()> {
    let account_id = AccountId::from_str(&account)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid account ID: {e}")))?;

    let value = ctx.assets.book_value(&account_id, &commodity).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "account_id": account_id.to_string(),
            "commodity": commodity,
            "book_value": value.map(|v| v.to_string()),
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        match value {
            Some(v) => println!("Book value: {v} {commodity}"),
            None => println!("No acquisition cost set for this account."),
        }
    }
    Ok(())
}

/// Parses a `YYYY-MM-DD` date string or returns today's date.
///
/// # Arguments
///
/// * `s` - Optional date string in `YYYY-MM-DD` format.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if the string cannot be parsed.
fn parse_date_or_today(s: Option<&str>) -> CliResult<jiff::civil::Date> {
    match s {
        Some(d) => jiff::civil::Date::from_str(d)
            .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{d}': {e}"))),
        None => Ok(jiff::Zoned::now().date()),
    }
}
