#![expect(
    clippy::mod_module_files,
    reason = "module split into transaction/mod.rs, transaction/leg.rs, transaction/scope.rs and transaction/spec.rs"
)]
//! Transaction management sub-commands: list, add, amend, edit, reverse.

use core::str::FromStr as _;

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

mod edit;
mod leg;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired into add in the next commit")
)]
mod scope;
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
    /// Change the postings of an existing transaction.
    ///
    /// Find the transaction by ID, or by --account and --date, adding
    /// --amount when several transactions touch that account on that day.
    /// Postings no operation names are kept unchanged.
    Edit(EditArgs),
    /// Reverse a transaction by creating a new transaction with negated postings.
    Reverse {
        /// Transaction ID to reverse.
        id: String,
    },
}

/// Arguments for `transaction edit`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct EditArgs {
    /// Transaction ID. Give this, or --account and --date.
    #[arg(conflicts_with_all = ["account", "date", "amount"])]
    pub id: Option<String>,
    /// Find the transaction by a posting on exactly this account (path or ID).
    #[arg(long, requires = "date")]
    pub account: Option<String>,
    /// The transaction's date (YYYY-MM-DD).
    #[arg(long, requires = "account")]
    pub date: Option<String>,
    /// That posting's signed amount, when several transactions match.
    #[arg(long, requires = "account", allow_hyphen_values = true)]
    pub amount: Option<String>,
    /// Add a posting `ACCOUNT:AMOUNT:COMMODITY[{COST}][@PRICE]`. Repeat for each.
    #[arg(long = "add-posting", value_name = "SPEC", num_args = 1)]
    pub add: Vec<String>,
    /// Replace a posting's account, amount, cost and price, keeping its
    /// metadata, tags and import link. `POSTING` is a posting ID, an account,
    /// or `ACCOUNT:AMOUNT:COMMODITY` when the account holds several postings.
    #[arg(long = "set-posting", value_name = "POSTING=SPEC", num_args = 1)]
    pub set: Vec<String>,
    /// Remove a posting, named as for --set-posting. Repeat for each.
    #[arg(long = "remove-posting", value_name = "POSTING", num_args = 1)]
    pub remove: Vec<String>,
}

/// Serialises `value` and adds a `warnings` key naming each warning's
/// [`Display`](std::fmt::Display) rendering.
///
/// Used for the JSON payload of a write that can raise warnings, whether
/// [`bc_core::Warning`]s or the CLI's own, on a value that otherwise carries
/// none of its own: the payload stays a plain object a script can index
/// straight into, rather than nesting the written value under its own key.
///
/// Each call site also prints these same warnings to stderr unconditionally,
/// so stdout stays parseable while a human still sees them on the terminal.
/// A `--json` consumer that captures stderr too will see each warning twice
/// by design.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Json`] if `value` cannot serialise.
fn with_warnings<T, W>(value: &T, warnings: &[W]) -> crate::error::CliResult<serde_json::Value>
where
    T: serde::Serialize,
    W: core::fmt::Display,
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
        Command::Edit(edit_args) => edit(ctx, edit_args).await,
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
        return crate::output::print_json(&with_warnings(&created, warned.warnings.as_slice())?);
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
        return crate::output::print_json(&with_warnings(&reloaded, warned.warnings.as_slice())?);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Amended transaction: {id}");
    }
    Ok(())
}

/// Renders a posting as `Account:Path 50.00 AUD` for messages.
fn describe_posting(posting: &bc_models::Posting, resolver: &bc_core::AccountResolver) -> String {
    let account = resolver
        .path_of(posting.account_id())
        .map_or_else(|| posting.account_id().to_string(), ToOwned::to_owned);
    match leg::render_leg(posting) {
        Some(rendered) => format!("{account} {rendered}"),
        None => account,
    }
}

/// Finds the transaction `args` names, by ID or by selector.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] when the selector matches no
/// transaction or several, listing the candidates in the second case.
async fn find_target(
    ctx: &AppContext,
    args: &EditArgs,
    lookup: &spec::Lookup<'_>,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<bc_models::Transaction> {
    if let Some(id) = &args.id {
        let tx_id = bc_models::TransactionId::from_str(id).map_err(|e| {
            crate::error::CliError::Arg(format!("invalid transaction ID '{id}': {e}"))
        })?;
        return Ok(ctx.transactions.find_by_id(&tx_id).await?);
    }
    let (Some(account), Some(date)) = (&args.account, &args.date) else {
        return Err(crate::error::CliError::Arg(
            "give a transaction ID, or --account and --date to find one".into(),
        ));
    };
    let account_id = lookup(account)
        .ok_or_else(|| crate::error::CliError::Arg(format!("no account '{account}'")))?;
    let day = jiff::civil::Date::from_str(date)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{date}': {e}")))?;
    let next = day
        .tomorrow()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{date}': {e}")))?;
    let amount = args
        .amount
        .as_deref()
        .map(|raw| {
            rust_decimal::Decimal::from_str(raw)
                .map_err(|e| crate::error::CliError::Arg(format!("invalid amount '{raw}': {e}")))
        })
        .transpose()?;

    let mut found: Vec<bc_models::Transaction> = ctx
        .transactions
        .list_for_account_in_range(&account_id, day, next)
        .await?
        .filter(|tx| edit::touches(tx, &account_id, amount))
        .collect();
    let wanted = amount.map(|a| format!(" for {a}")).unwrap_or_default();
    if found.len() > 1 {
        let candidates: Vec<String> = found
            .iter()
            .map(|tx| {
                let legs: Vec<String> = tx.postings().iter().map(describe).collect();
                format!(
                    "  {}  {}  {}: {}",
                    tx.id(),
                    tx.date(),
                    tx.description(),
                    legs.join(", ")
                )
            })
            .collect();
        return Err(crate::error::CliError::Arg(format!(
            "{} transactions on {account} dated {date}{wanted}; narrow the \
             selector, or pass one of these transaction IDs:\n{}",
            found.len(),
            candidates.join("\n")
        )));
    }
    found.pop().ok_or_else(|| {
        crate::error::CliError::Arg(format!("no transaction on {account} dated {date}{wanted}"))
    })
}

/// Changes the postings of an existing transaction.
async fn edit(ctx: &AppContext, args: EditArgs) -> CliResult<()> {
    if args.add.is_empty() && args.set.is_empty() && args.remove.is_empty() {
        return Err(crate::error::CliError::Arg(
            "nothing to edit: give --add-posting, --set-posting or --remove-posting".into(),
        ));
    }
    let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
    let lookup = spec::account_lookup(&resolver);
    let describe = |posting: &bc_models::Posting| describe_posting(posting, &resolver);

    let current = find_target(ctx, &args, &lookup, &describe).await?;
    let select = |text: &str| -> CliResult<bc_models::PostingId> {
        let selector = edit::parse_selector(text, &lookup)?;
        edit::select_posting(&current, &selector, text, &describe).cloned()
    };

    let mut set = Vec::with_capacity(args.set.len());
    for text in &args.set {
        let (target, spec_text) = text.split_once('=').ok_or_else(|| {
            crate::error::CliError::Arg(format!(
                "invalid --set-posting '{text}': expected POSTING=SPEC"
            ))
        })?;
        set.push((select(target)?, spec::parse_posting(spec_text, &lookup)?));
    }
    let ops = edit::Ops {
        add: args
            .add
            .iter()
            .map(|s| spec::parse_posting(s, &lookup))
            .collect::<CliResult<_>>()?,
        set,
        remove: args
            .remove
            .iter()
            .map(|t| select(t))
            .collect::<CliResult<_>>()?,
    };
    let updated = edit::apply(&current, &ops, &describe)?;

    let imported: std::collections::HashSet<String> = ctx
        .sources
        .provenance_by_posting(current.id())
        .await?
        .into_keys()
        .collect();
    let mut warnings: Vec<String> = edit::imported_removals(&current, &ops, &imported)
        .into_iter()
        .map(|p| {
            format!(
                "removed posting {} came from an import and no longer matches its statement row",
                describe(p)
            )
        })
        .collect();

    let warned = ctx.transactions.edit(updated).await?;
    warnings.extend(warned.warnings.iter().map(ToString::to_string));
    for warning in &warnings {
        #[expect(clippy::print_stderr, reason = "CLI output")]
        {
            eprintln!("warning: {warning}");
        }
    }

    if ctx.json {
        let reloaded = ctx.transactions.find_by_id(current.id()).await?;
        return crate::output::print_json(&with_warnings(&reloaded, warnings.as_slice())?);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Edited transaction: {}", current.id());
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
