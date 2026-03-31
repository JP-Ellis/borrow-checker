//! Transaction management sub-commands: list, add, amend, void.

use core::str::FromStr as _;

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `transaction` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The transaction operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available transaction operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List all non-voided transactions ordered by date descending.
    List,
    /// Record a new double-entry transaction.
    Add {
        /// Transaction date in YYYY-MM-DD format.
        #[arg(long)]
        date: String,
        /// Transaction description (memo).
        #[arg(long)]
        description: String,
        /// Optional payee name.
        #[arg(long)]
        payee: Option<String>,
        /// Posting in `ACCOUNT_ID:AMOUNT:COMMODITY` format. Repeat for each posting.
        /// Must include at least two postings that balance to zero.
        #[arg(
            long = "posting",
            value_name = "ACCOUNT:AMOUNT:COMMODITY",
            num_args = 1
        )]
        postings: Vec<String>,
    },
    /// Amend the metadata of an existing transaction.
    Amend {
        /// Transaction ID to amend.
        id: String,
        /// New date (YYYY-MM-DD).
        #[arg(long)]
        date: Option<String>,
        /// New description.
        #[arg(long)]
        description: Option<String>,
        /// New payee. To remove an existing payee, use `--clear-payee` instead.
        #[arg(long, conflicts_with = "clear_payee")]
        payee: Option<String>,
        /// Remove the payee from this transaction.
        #[arg(long)]
        clear_payee: bool,
    },
    /// Void a transaction (preserves data; excludes from balances and reports).
    Void {
        /// Transaction ID to void.
        id: String,
    },
}

/// Parses a posting specification `ACCOUNT_ID:AMOUNT:COMMODITY`.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if the spec is malformed or the
/// amount cannot be parsed as a [`rust_decimal::Decimal`].
fn parse_posting_spec(spec: &str) -> crate::error::CliResult<bc_models::Posting> {
    // `rsplitn` splits right-to-left so index 0 is the rightmost segment.
    let mut parts = spec.rsplitn(3, ':');
    let (Some(commodity), Some(amount_str), Some(account_id_str)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(crate::error::CliError::Arg(format!(
            "invalid posting '{spec}': expected ACCOUNT_ID:AMOUNT:COMMODITY"
        )));
    };

    let account_id = bc_models::AccountId::from_str(account_id_str).map_err(|e| {
        crate::error::CliError::Arg(format!("invalid account ID '{account_id_str}': {e}"))
    })?;
    let value = amount_str
        .parse::<rust_decimal::Decimal>()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid amount '{amount_str}': {e}")))?;

    Ok(bc_models::Posting::builder()
        .id(bc_models::PostingId::new())
        .account_id(account_id)
        .amount(bc_models::Amount::new(
            value,
            bc_models::CommodityCode::new(commodity),
        ))
        .build())
}

/// Executes the `transaction` subcommand.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or output layer.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx).await,
        Command::Add {
            date,
            description,
            payee,
            postings,
        } => add(ctx, date, description, payee, postings).await,
        Command::Amend {
            id,
            date,
            description,
            payee,
            clear_payee,
        } => amend(ctx, id, date, description, payee, clear_payee).await,
        Command::Void { id } => void(ctx, id).await,
    }
}

/// Lists all non-voided transactions.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let transactions = ctx.transactions.list().await?;

    if ctx.json {
        return crate::output::print_json(&transactions);
    }

    if transactions.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No transactions.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = transactions
        .iter()
        .map(|tx| {
            let amounts: Vec<String> = tx
                .postings()
                .iter()
                .filter(|p| p.amount().value() > rust_decimal::Decimal::ZERO)
                .map(|p| format!("{} {}", p.amount().value(), p.amount().commodity().as_str()))
                .collect();
            let amounts_str = amounts.join(", ");
            let description = tx.payee().map_or_else(
                || tx.description().to_owned(),
                |payee| format!("{payee}: {}", tx.description()),
            );
            vec![
                tx.id().to_string(),
                tx.date().to_string(),
                description,
                amounts_str,
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "DATE", "DESCRIPTION", "AMOUNTS"], &rows);
    Ok(())
}

/// Records a new double-entry transaction.
async fn add(
    ctx: &AppContext,
    date: String,
    description: String,
    payee: Option<String>,
    posting_specs: Vec<String>,
) -> CliResult<()> {
    if posting_specs.len() < 2 {
        return Err(crate::error::CliError::Arg(
            "at least two --posting arguments are required".into(),
        ));
    }

    let postings: Vec<bc_models::Posting> = posting_specs
        .iter()
        .map(|s| parse_posting_spec(s))
        .collect::<crate::error::CliResult<_>>()?;

    let parsed_date = jiff::civil::Date::from_str(&date)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{date}': {e}")))?;

    let tx = bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(parsed_date)
        .description(description)
        .maybe_payee(payee)
        .postings(postings)
        .status(bc_models::TransactionStatus::Cleared)
        .created_at(jiff::Timestamp::now())
        .build();

    let tx_id = ctx.transactions.create(tx).await?;

    if ctx.json {
        let created = ctx.transactions.find_by_id(&tx_id).await?;
        return crate::output::print_json(&created);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created transaction: {tx_id}");
    }
    Ok(())
}

/// Amends the metadata of an existing transaction.
async fn amend(
    ctx: &AppContext,
    id: String,
    date: Option<String>,
    description: Option<String>,
    payee: Option<String>,
    clear_payee: bool,
) -> CliResult<()> {
    let tx_id = bc_models::TransactionId::from_str(&id)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid transaction ID '{id}': {e}")))?;

    let original = ctx.transactions.find_by_id(&tx_id).await?;

    let new_date = if let Some(d) = date {
        jiff::civil::Date::from_str(&d)
            .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{d}': {e}")))?
    } else {
        original.date()
    };
    let new_description = description.unwrap_or_else(|| original.description().to_owned());
    // `--clear-payee` sets payee to None; `--payee <value>` sets a new payee;
    // omitting both preserves the original payee.
    let new_payee = if clear_payee {
        None
    } else {
        payee.or_else(|| original.payee().map(str::to_owned))
    };

    let updated = bc_models::Transaction::builder()
        .id(tx_id.clone())
        .date(new_date)
        .description(new_description)
        .maybe_payee(new_payee)
        .postings(original.postings().to_vec())
        .tag_ids(original.tag_ids().to_vec())
        .status(original.status())
        .created_at(*original.created_at())
        .build();
    ctx.transactions.amend(updated).await?;

    if ctx.json {
        let reloaded = ctx.transactions.find_by_id(&tx_id).await?;
        return crate::output::print_json(&reloaded);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Amended transaction: {id}");
    }
    Ok(())
}

/// Voids a transaction by ID.
async fn void(ctx: &AppContext, id: String) -> CliResult<()> {
    let tx_id = bc_models::TransactionId::from_str(&id)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid transaction ID '{id}': {e}")))?;

    ctx.transactions.void(&tx_id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "voided": true,
            "id": id,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Voided transaction: {id}");
    }
    Ok(())
}
