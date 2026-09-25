#![expect(
    clippy::mod_module_files,
    reason = "module split into transaction/mod.rs, transaction/leg.rs and transaction/spec.rs"
)]
//! Transaction management sub-commands: list, add, amend, reverse.

use core::str::FromStr as _;

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

mod leg;
mod spec;

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
        /// Posting `ACCOUNT:AMOUNT:COMMODITY`, where `ACCOUNT` is an account
        /// path or an account ID, optionally followed by a cost block and a
        /// price: `ID:2:AAPL{105:AUD:2024-03-01:lot-a}@150:AUD`,
        /// `ID:4.00:USD@@6.37:AUD`, `ID:2:AAPL{{210:AUD}}`. Cost components may
        /// be separated by `:` or `,` in any order; wrap a label in `"…"` to
        /// keep `,`, `:` or `}` inside it. Quote the value when it holds a
        /// comma, or the shell brace-expands it. Repeat for each posting; at
        /// least two are required.
        #[arg(
            long = "posting",
            value_name = "ACCOUNT:AMOUNT:COMMODITY[{COST}][@PRICE]",
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

/// Serialises `value` and adds a `warnings` key naming each warning's
/// [`Display`](std::fmt::Display) rendering.
///
/// Used for the JSON payload of a write that can raise [`bc_core::Warning`]s
/// on a value that otherwise carries none of its own: the payload stays a
/// plain object a script can index straight into, rather than nesting the
/// written value under its own key.
///
/// Each call site also prints these same warnings to stderr unconditionally,
/// so stdout stays parseable while a human still sees them on the terminal.
/// A `--json` consumer that captures stderr too will see each warning twice
/// by design.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Json`] if `value` cannot serialise.
fn with_warnings<T>(
    value: &T,
    warnings: &[bc_core::Warning],
) -> crate::error::CliResult<serde_json::Value>
where
    T: serde::Serialize,
{
    let mut json = serde_json::to_value(value)?;
    if let serde_json::Value::Object(ref mut map) = json {
        map.insert(
            "warnings".to_owned(),
            serde_json::Value::Array(
                warnings
                    .iter()
                    .map(|warning| serde_json::Value::String(warning.to_string()))
                    .collect(),
            ),
        );
    }
    Ok(json)
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
                // The column names what the transaction moved: the positive
                // legs, plus any leg whose cost or price would otherwise be
                // seen only in `--json`, such as a sale.
                .filter_map(|p| {
                    let a = p.amount()?;
                    let annotated = p.cost().is_some() || p.price().is_some();
                    (annotated || a.value() > rust_decimal::Decimal::ZERO)
                        .then(|| leg::render_leg(p))
                        .flatten()
                })
                .collect();
            let amounts_str = amounts.join(", ");
            // Read the payee through `iter()`, which is the only way to reach
            // `mismatched`. A flagged entry is stored as text, so
            // `get_first_text` returns it and drops the flag with no trace,
            // leaving a value the store could not read looking like one it
            // could.
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

    let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
    let lookup = spec::account_lookup(&resolver);
    let postings: Vec<bc_models::Posting> = posting_specs
        .iter()
        .map(|s| spec::parse_posting(s, &lookup))
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

    let warned = ctx.transactions.create(tx).await?;
    for warning in &warned.warnings {
        #[expect(clippy::print_stderr, reason = "CLI output")]
        {
            eprintln!("warning: {warning}");
        }
    }
    let tx_id = warned.value;

    if ctx.json {
        let created = ctx.transactions.find_by_id(&tx_id).await?;
        return crate::output::print_json(&with_warnings(&created, &warned.warnings)?);
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
    let warned = ctx.transactions.amend(updated).await?;
    for warning in &warned.warnings {
        #[expect(clippy::print_stderr, reason = "CLI output")]
        {
            eprintln!("warning: {warning}");
        }
    }

    if ctx.json {
        let reloaded = ctx.transactions.find_by_id(&tx_id).await?;
        return crate::output::print_json(&with_warnings(&reloaded, &warned.warnings)?);
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
