//! Transaction management sub-commands: list, add, amend, reverse.

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
        /// Transaction description.
        #[arg(long)]
        description: String,
        /// Metadata entry `KEY=VALUE`. Repeat for each entry, including
        /// repeats of one key. The value's type comes from the key registry,
        /// and is inferred from the value for a key not yet registered.
        #[arg(long = "meta", value_name = "KEY=VALUE", num_args = 1)]
        meta: Vec<String>,
        /// Posting in `ACCOUNT_ID:AMOUNT:COMMODITY` format. Repeat for each posting.
        /// Must include at least two postings that balance to zero.
        #[arg(
            long = "posting",
            value_name = "ACCOUNT:AMOUNT:COMMODITY",
            num_args = 1
        )]
        postings: Vec<String>,
    },
    /// Amend the date, description or metadata of an existing transaction.
    Amend {
        /// Transaction ID to amend.
        id: String,
        /// New date (YYYY-MM-DD).
        #[arg(long)]
        date: Option<String>,
        /// New description.
        #[arg(long)]
        description: Option<String>,
        /// Metadata entry `KEY=VALUE`, replacing every stored entry under that
        /// key. Repeat the key to store several entries under it. To remove a
        /// key's entries, use `--clear-meta` instead.
        #[arg(long = "meta", value_name = "KEY=VALUE", num_args = 1)]
        meta: Vec<String>,
        /// Remove every metadata entry under this key. Repeat for each key.
        #[arg(long = "clear-meta", value_name = "KEY", num_args = 1)]
        clear_meta: Vec<String>,
    },
    /// Reverse a transaction by creating a new transaction with negated postings.
    Reverse {
        /// Transaction ID to reverse.
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
            meta: meta_specs,
            postings,
        } => add(ctx, date, description, &meta_specs, postings).await,
        Command::Amend {
            id,
            date,
            description,
            meta: meta_specs,
            clear_meta,
        } => amend(ctx, id, date, description, &meta_specs, &clear_meta).await,
        Command::Reverse { id } => reverse(ctx, id).await,
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

    // Only worth a query when an account-valued entry is actually on screen.
    let resolver = if transactions.iter().any(|tx| {
        tx.metadata()
            .iter()
            .any(|e| matches!(*e.value(), bc_models::MetaValue::Account(_)))
    }) {
        Some(bc_core::AccountResolver::load(&ctx.accounts).await?)
    } else {
        None
    };
    let payee_key = bc_models::MetaKey::new("payee")
        .map_err(|e| crate::error::CliError::Arg(format!("invalid metadata key 'payee': {e}")))?;

    let rows: Vec<Vec<String>> = transactions
        .iter()
        .map(|tx| {
            let amounts: Vec<String> = tx
                .postings()
                .iter()
                .filter_map(|p| {
                    let a = p.amount()?;
                    (a.value() > rust_decimal::Decimal::ZERO)
                        .then(|| format!("{} {}", a.value(), a.commodity().as_str()))
                })
                .collect();
            let amounts_str = amounts.join(", ");
            // Read the payee through `iter()`: `get_first_text` answers `None`
            // both for an absent payee and for one stored under another type,
            // and a flagged entry is exactly the second case.
            let description = tx
                .metadata()
                .iter()
                .find(|e| e.key() == &payee_key)
                .map_or_else(
                    || tx.description().to_owned(),
                    |entry| {
                        let flag = if entry.mismatched() { "!" } else { "" };
                        format!("{flag}{}: {}", entry.value().canonical(), tx.description())
                    },
                );
            let metadata: Vec<String> = tx
                .metadata()
                .iter()
                .map(|e| super::meta::render_entry(e, resolver.as_ref()))
                .collect();
            vec![
                tx.id().to_string(),
                tx.date().to_string(),
                description,
                amounts_str,
                metadata.join(", "),
            ]
        })
        .collect();
    crate::output::print_table(&["ID", "DATE", "DESCRIPTION", "AMOUNTS", "META"], &rows);
    Ok(())
}

/// Records a new double-entry transaction.
async fn add(
    ctx: &AppContext,
    date: String,
    description: String,
    meta_specs: &[String],
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

    let metadata: bc_models::Metadata = super::meta::entries_for(ctx, meta_specs)
        .await?
        .into_iter()
        .collect();

    let tx = bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(parsed_date)
        .description(description)
        .metadata(metadata)
        .postings(postings)
        .reconciliation(bc_models::Reconciliation::Reconciled)
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

/// Amends the date, description or metadata of an existing transaction.
async fn amend(
    ctx: &AppContext,
    id: String,
    date: Option<String>,
    description: Option<String>,
    meta_specs: &[String],
    clear_meta: &[String],
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

    let cleared: Vec<bc_models::MetaKey> = clear_meta
        .iter()
        .map(|key| super::meta::parse_meta_key(key))
        .collect::<CliResult<_>>()?;
    let entries = super::meta::entries_for(ctx, meta_specs).await?;
    if let Some(entry) = entries.iter().find(|e| cleared.contains(e.key())) {
        return Err(crate::error::CliError::Arg(format!(
            "--meta and --clear-meta both name '{}': one sets the key, the other removes it",
            entry.key()
        )));
    }
    let new_metadata = super::meta::apply_changes(original.metadata(), &entries, &cleared);

    let updated = bc_models::Transaction::builder()
        .id(tx_id.clone())
        .date(new_date)
        .description(new_description)
        .metadata(new_metadata)
        .postings(original.postings().to_vec())
        .tag_ids(original.tag_ids().to_vec())
        .reconciliation(original.reconciliation())
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

/// Reverses a transaction by ID, creating a new transaction with negated postings.
async fn reverse(ctx: &AppContext, id: String) -> CliResult<()> {
    let tx_id = bc_models::TransactionId::from_str(&id)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid transaction ID '{id}': {e}")))?;

    let reversal_id = ctx.transactions.reverse(&tx_id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "reversed": true,
            "id": id,
            "reversal_id": reversal_id.to_string(),
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Reversed transaction: {id}");
        println!("Reversal transaction: {reversal_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_posting_spec;

    #[test]
    fn valid_posting_spec_parses() {
        // Use AccountId::new() to get a valid ID string.
        let account_id = bc_models::AccountId::new().to_string();
        let spec = format!("{account_id}:50.00:AUD");
        let posting = parse_posting_spec(&spec).expect("valid spec");
        let amount = posting.amount().expect("amount should be set");
        pretty_assertions::assert_eq!(amount.value().to_string(), "50.00");
        pretty_assertions::assert_eq!(amount.commodity().as_str(), "AUD");
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "test — asserting error path, panics are acceptable"
    )]
    fn posting_spec_too_few_segments_returns_error() {
        // Only one colon — missing commodity.
        let err = parse_posting_spec("someaccount:50.00").unwrap_err();
        assert!(err.to_string().contains("ACCOUNT_ID:AMOUNT:COMMODITY"));
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "test — asserting error path, panics are acceptable"
    )]
    fn posting_spec_invalid_amount_returns_error() {
        let account_id = bc_models::AccountId::new().to_string();
        let spec = format!("{account_id}:notanumber:AUD");
        let err = parse_posting_spec(&spec).unwrap_err();
        assert!(err.to_string().contains("invalid amount"));
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "test — asserting error path, panics are acceptable"
    )]
    fn posting_spec_invalid_account_id_returns_error() {
        // Clearly invalid account ID.
        let err = parse_posting_spec("notanid:50.00:AUD").unwrap_err();
        assert!(err.to_string().contains("invalid account ID"));
    }
}
