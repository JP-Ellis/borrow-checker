#![expect(
    clippy::mod_module_files,
    reason = "module split into transaction/mod.rs and changes, edit, leg, resolve, scope and spec"
)]
//! Transaction management sub-commands: list, add, edit, reverse.

use core::str::FromStr as _;

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

mod changes;
mod edit;
mod leg;
mod resolve;
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
    ///
    /// Flags before the first --posting describe the transaction; flags after
    /// a --posting describe that posting, until the next one.
    Add(AddArgs),
    /// Change the date, description, metadata, tags or postings of an
    /// existing transaction.
    ///
    /// Find the transaction by ID, or by --find ACCOUNT DATE [AMOUNT].
    /// Flags before the first --add, --set or --remove change the
    /// transaction; flags after one change that posting, until the next.
    /// A --set changes only what it names, and postings no flag names are
    /// kept unchanged.
    Edit(EditArgs),
    /// Reverse a transaction by creating a new transaction with negated postings.
    Reverse {
        /// Transaction ID to reverse.
        id: String,
    },
}

/// Arguments for `transaction add`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// The transaction and posting flags, in the order written.
    #[command(flatten)]
    flags: scope::Scoped<scope::AddFlags>,
}

/// Arguments for `transaction edit`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct EditArgs {
    /// The transaction and posting flags, in the order written.
    #[command(flatten)]
    flags: scope::Scoped<scope::EditFlags>,
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
        Command::Add(add_args) => add(ctx, add_args).await,
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

/// Resolves the tags every scope and the transaction name, in one call.
async fn resolve_tags(ctx: &AppContext, all: &[&changes::Changes]) -> CliResult<resolve::Tags> {
    let tagged: Vec<&str> = all
        .iter()
        .flat_map(|c| c.tags.iter().map(String::as_str))
        .collect();
    let untagged: Vec<&str> = all
        .iter()
        .flat_map(|c| c.untags.iter().map(String::as_str))
        .collect();
    resolve::tags(ctx, &tagged, &untagged).await
}

/// Prints one `warning:` line per warning to stderr.
fn warn_all(warnings: &[String]) {
    for warning in warnings {
        #[expect(clippy::print_stderr, reason = "CLI output")]
        {
            eprintln!("warning: {warning}");
        }
    }
}

/// Records a new double-entry transaction.
///
/// Every check that needs no tag runs before any tag is created. A failure
/// after that point still reports the tags it created.
async fn add(ctx: &AppContext, args: AddArgs) -> CliResult<()> {
    let plan = scope::fold(&args.flags.written, scope::Command::Add)?;
    if plan.scopes.len() < 2 {
        return Err(crate::error::CliError::Arg(
            "at least two --posting arguments are required".into(),
        ));
    }
    let tx_changes = changes::Changes::from_written(&plan.transaction, "the transaction")?;
    let (Some(date), Some(description)) = (tx_changes.date, tx_changes.description.clone()) else {
        return Err(crate::error::CliError::Arg(
            "--date and --description are required".into(),
        ));
    };

    let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
    let lookup = spec::account_lookup(&resolver);
    let mut legs = Vec::with_capacity(plan.scopes.len());
    for scope in &plan.scopes {
        let leg = changes::Changes::from_written(&scope.modifiers, &scope.label)?;
        // `add` opens only new legs; `Opener` is shared with `edit`.
        let scope::Opener::New {
            account,
            amount: written,
        } = &scope.opener
        else {
            continue;
        };
        let (account_id, amount) = new_leg(&scope.label, account, written.as_ref(), &leg, &lookup)?;
        legs.push((&scope.label, account_id, amount, leg));
    }
    writable_postings(
        legs.len(),
        legs.iter()
            .filter(|(_, _, amount, _)| amount.is_none())
            .count(),
    )?;

    let all: Vec<&changes::Changes> = core::iter::once(&tx_changes)
        .chain(legs.iter().map(|(_, _, _, leg)| leg))
        .collect();
    let tags = resolve_tags(ctx, &all).await?;
    let mut warnings: Vec<String> = tags
        .created()
        .iter()
        .map(|path| format!("created tag '{path}'"))
        .collect();

    let outcome = async {
        let mut postings = Vec::with_capacity(legs.len());
        for (label, account_id, amount, leg) in legs {
            let looked_up = resolve::resolved(ctx, leg, &lookup, &tags).await?;
            postings.push(changes::new_posting(account_id, amount, &looked_up, label)?);
        }
        let tx = resolve::resolved(ctx, tx_changes, &lookup, &tags).await?;
        let transaction = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date)
            .description(description)
            .metadata(bc_models::Metadata::new(tx.entries.clone()))
            .tag_ids(changes::retag(&[], &tx.tags, &[]))
            .postings(postings)
            .reconciliation(bc_models::Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        Ok::<_, crate::error::CliError>(ctx.transactions.create(transaction).await?)
    }
    .await;
    let warned = outcome.inspect_err(|_| warn_all(&warnings))?;
    warnings.extend(warned.warnings.iter().map(ToString::to_string));
    warn_all(&warnings);
    let tx_id = warned.value;

    if ctx.json {
        let created = ctx.transactions.find_by_id(&tx_id).await?;
        return crate::output::print_json(&with_warnings(&created, warnings.as_slice())?);
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created transaction: {tx_id}");
    }
    Ok(())
}

/// Checks a new leg's opener and cost flags, without creating any tag.
///
/// Returns the leg's account and its amount, `None` when elided.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`], prefixed with `label`, when the
/// account is not found, the amount is malformed, or a lot date or label has
/// no cost.
fn new_leg(
    label: &str,
    account: &str,
    written: Option<&[String; 2]>,
    typed: &changes::Changes,
    lookup: &spec::Lookup<'_>,
) -> CliResult<(bc_models::AccountId, Option<bc_models::Amount>)> {
    let account_id = lookup(account)
        .ok_or_else(|| crate::error::CliError::Arg(format!("{label}: no account '{account}'")))?;
    let amount = written
        .map(|[value, code]| changes::amount_of(value, code))
        .transpose()
        .map_err(|e| crate::error::CliError::Arg(format!("{label}: {e}")))?;
    changes::cost_of(None, typed, label)?;
    Ok((account_id, amount))
}

/// Refuses a transaction left with `total` postings, `elided` of them
/// elided, when the transaction service would, in the words it uses.
///
/// The service refuses the same, but only after tags are created; this check
/// runs before.
fn writable_postings(total: usize, elided: usize) -> CliResult<()> {
    let refusal = if total == 0 {
        "transaction has no postings"
    } else if elided >= 2 {
        "two or more elided postings"
    } else if elided == 1 && total == 1 {
        "a lone elided posting carries no amount"
    } else {
        return Ok(());
    };
    Err(crate::error::CliError::Arg(refusal.into()))
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

/// Finds the transaction `plan` names, by ID or by `--find`.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] when `--find` matches no
/// transaction or several, listing the candidates in the second case.
async fn find_target(
    ctx: &AppContext,
    plan: &scope::Plan,
    lookup: &spec::Lookup<'_>,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<bc_models::Transaction> {
    if let Some(id) = &plan.id {
        let tx_id = bc_models::TransactionId::from_str(id).map_err(|e| {
            crate::error::CliError::Arg(format!("invalid transaction ID '{id}': {e}"))
        })?;
        return Ok(ctx.transactions.find_by_id(&tx_id).await?);
    }
    let Some([account, date, rest @ ..]) = plan.find.as_deref() else {
        return Err(crate::error::CliError::Arg(
            "give a transaction ID, or --find ACCOUNT DATE [AMOUNT]".into(),
        ));
    };
    let account_id = lookup(account)
        .ok_or_else(|| crate::error::CliError::Arg(format!("--find: no account '{account}'")))?;
    let day = jiff::civil::Date::from_str(date)
        .map_err(|e| crate::error::CliError::Arg(format!("--find: invalid date '{date}': {e}")))?;
    let next = day
        .tomorrow()
        .map_err(|e| crate::error::CliError::Arg(format!("--find: invalid date '{date}': {e}")))?;
    let amount = rest
        .first()
        .map(|raw| {
            rust_decimal::Decimal::from_str(raw).map_err(|e| {
                crate::error::CliError::Arg(format!("--find: invalid amount '{raw}': {e}"))
            })
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
            "{} transactions on {account} dated {date}{wanted}; add the amount to \
             --find, or pass one of these transaction IDs:\n{}",
            found.len(),
            candidates.join("\n")
        )));
    }
    found.pop().ok_or_else(|| {
        crate::error::CliError::Arg(format!(
            "--find: no transaction on {account} dated {date}{wanted}"
        ))
    })
}

/// What one posting scope of `edit` does, once checked.
enum Step {
    /// Append a new leg on this account, with this amount unless elided.
    Add(bc_models::AccountId, Option<bc_models::Amount>),
    /// Change this stored posting.
    Set(bc_models::PostingId),
    /// Drop this stored posting.
    Remove(bc_models::PostingId),
}

/// Checks one posting scope of `edit` against `current`, without creating
/// any tag.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`], prefixed with the scope label,
/// when an account or posting is not found, an amount is malformed, or a lot
/// date or label has no cost.
fn check_scope(
    current: &bc_models::Transaction,
    scope: &scope::Scope,
    typed: &changes::Changes,
    lookup: &spec::Lookup<'_>,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<Step> {
    let label = &scope.label;
    let in_scope = |e: crate::error::CliError| crate::error::CliError::Arg(format!("{label}: {e}"));
    let select = |tokens: &[String]| -> CliResult<bc_models::PostingId> {
        let selector = edit::parse_selector(tokens, lookup).map_err(in_scope)?;
        edit::select_posting(current, &selector, label, describe).cloned()
    };
    match &scope.opener {
        scope::Opener::New {
            account,
            amount: written,
        } => {
            let (account_id, amount) = new_leg(label, account, written.as_ref(), typed, lookup)?;
            Ok(Step::Add(account_id, amount))
        }
        scope::Opener::Set(tokens) => {
            let id = select(tokens)?;
            if let Some(text) = &typed.account
                && lookup(text).is_none()
            {
                return Err(crate::error::CliError::Arg(format!(
                    "{label}: no account '{text}'"
                )));
            }
            let stored = current.postings().iter().find(|p| p.id() == &id);
            changes::cost_of(stored.and_then(bc_models::Posting::cost), typed, label)?;
            Ok(Step::Set(id))
        }
        scope::Opener::Remove(tokens) => Ok(Step::Remove(select(tokens)?)),
    }
}

/// How many postings `current` holds once `steps` apply, and how many of
/// those are elided.
///
/// A removed posting no longer counts, a `--set` with `--amount` gives its
/// posting an amount, and a new leg adds one, elided when it has no amount.
fn postings_after(
    current: &bc_models::Transaction,
    steps: &[(&scope::Scope, changes::Changes, Step)],
) -> (usize, usize) {
    let kept: Vec<&bc_models::Posting> = current
        .postings()
        .iter()
        .filter(|p| {
            !steps
                .iter()
                .any(|(_, _, step)| matches!(step, Step::Remove(id) if id == p.id()))
        })
        .collect();
    let kept_elided = kept
        .iter()
        .filter(|p| p.amount().is_none())
        .filter(|p| {
            !steps.iter().any(|(_, typed, step)| {
                matches!(step, Step::Set(id) if id == p.id()) && typed.amount.is_some()
            })
        })
        .count();
    let added = steps
        .iter()
        .filter(|(_, _, step)| matches!(step, Step::Add(..)))
        .count();
    let added_elided = steps
        .iter()
        .filter(|(_, _, step)| matches!(step, Step::Add(_, None)))
        .count();
    (
        kept.len().saturating_add(added),
        kept_elided.saturating_add(added_elided),
    )
}

/// Builds the edit `steps` describe, writes it, and returns its warnings.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] from the key registry, the posting
/// builders, or the transaction service.
async fn write_edit(
    ctx: &AppContext,
    current: &bc_models::Transaction,
    tx_changes: changes::Changes,
    steps: Vec<(&scope::Scope, changes::Changes, Step)>,
    lookup: &spec::Lookup<'_>,
    tags: &resolve::Tags,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<Vec<String>> {
    let mut ops = edit::Ops::default();
    if !tx_changes.is_empty() {
        ops.transaction = Some(resolve::resolved(ctx, tx_changes, lookup, tags).await?);
    }
    for (scope, typed, step) in steps {
        match step {
            Step::Add(account_id, amount) => {
                let looked_up = resolve::resolved(ctx, typed, lookup, tags).await?;
                ops.add.push(changes::new_posting(
                    account_id,
                    amount,
                    &looked_up,
                    &scope.label,
                )?);
            }
            Step::Set(id) => {
                let looked_up = resolve::resolved(ctx, typed, lookup, tags).await?;
                ops.set.push((id, scope.label.clone(), looked_up));
            }
            Step::Remove(id) => ops.remove.push(id),
        }
    }
    let updated = edit::apply(current, &ops, describe)?;

    let imported: std::collections::HashSet<String> = ctx
        .sources
        .provenance_by_posting(current.id())
        .await?
        .into_keys()
        .collect();
    let mut warnings: Vec<String> = edit::imported_removals(current, &ops, &imported)
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
    Ok(warnings)
}

/// Changes the date, description, metadata, tags or postings of an existing
/// transaction.
///
/// Every check that needs no tag runs before any tag is created. A failure
/// after that point still reports the tags it created.
async fn edit(ctx: &AppContext, args: EditArgs) -> CliResult<()> {
    let plan = scope::fold(&args.flags.written, scope::Command::Edit)?;
    let tx_changes = changes::Changes::from_written(&plan.transaction, "the transaction")?;
    if plan.scopes.is_empty() && tx_changes.is_empty() {
        return Err(crate::error::CliError::Arg(
            "nothing to edit: give --add, --set, --remove, or a transaction flag".into(),
        ));
    }
    let mut scoped = Vec::with_capacity(plan.scopes.len());
    for scope in &plan.scopes {
        let typed = changes::Changes::from_written(&scope.modifiers, &scope.label)?;
        if matches!(scope.opener, scope::Opener::Set(_)) && typed.is_empty() {
            return Err(crate::error::CliError::Arg(format!(
                "{}: nothing to change; name what changes after it",
                scope.label
            )));
        }
        scoped.push((scope, typed));
    }

    let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
    let lookup = spec::account_lookup(&resolver);
    let describe = |posting: &bc_models::Posting| describe_posting(posting, &resolver);
    let current = find_target(ctx, &plan, &lookup, &describe).await?;

    let mut steps = Vec::with_capacity(scoped.len());
    for (scope, typed) in scoped {
        let step = check_scope(&current, scope, &typed, &lookup, &describe)?;
        steps.push((scope, typed, step));
    }
    edit::named_once(
        &current,
        steps.iter().filter_map(|(_, _, step)| match step {
            Step::Set(id) | Step::Remove(id) => Some(id),
            Step::Add(..) => None,
        }),
        &describe,
    )?;
    let (total, elided) = postings_after(&current, &steps);
    writable_postings(total, elided)?;

    let all: Vec<&changes::Changes> = core::iter::once(&tx_changes)
        .chain(steps.iter().map(|(_, typed, _)| typed))
        .collect();
    let tags = resolve_tags(ctx, &all).await?;
    let mut warnings: Vec<String> = tags
        .created()
        .iter()
        .map(|path| format!("created tag '{path}'"))
        .collect();

    let outcome = write_edit(ctx, &current, tx_changes, steps, &lookup, &tags, &describe).await;
    warnings.extend(outcome.inspect_err(|_| warn_all(&warnings))?);
    warn_all(&warnings);

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
