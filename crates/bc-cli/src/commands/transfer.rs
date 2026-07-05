//! Transfer resolution sub-commands: merge, unmerge, suggest.

use core::str::FromStr as _;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for `merge`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct MergeArgs {
    /// The surviving transaction ID (keeps its ID and user fields).
    pub survivor: String,
    /// The transaction to fuse into the survivor.
    pub absorbed: String,
}

/// Arguments for `unmerge`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct UnmergeArgs {
    /// The transaction whose most recent merge is reversed.
    pub transaction: String,
}

/// Arguments for `suggest-transfers`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct SuggestArgs;

/// Parses a transaction ID from a raw command-line argument.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if `raw` is not a valid transaction ID.
fn parse_tx(raw: &str) -> CliResult<bc_models::TransactionId> {
    bc_models::TransactionId::from_str(raw)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid transaction ID '{raw}': {e}")))
}

/// Executes `merge`.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if an ID is invalid or the merge is rejected.
#[inline]
pub async fn merge(args: MergeArgs, ctx: &AppContext) -> CliResult<()> {
    let survivor = parse_tx(&args.survivor)?;
    let absorbed = parse_tx(&args.absorbed)?;
    ctx.transfers.merge(&survivor, &absorbed).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "merged": survivor.to_string() }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Merged {absorbed} into {survivor}.");
    }
    Ok(())
}

/// Executes `unmerge`.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the ID is invalid or there is no merge to reverse.
#[inline]
pub async fn unmerge(args: UnmergeArgs, ctx: &AppContext) -> CliResult<()> {
    let tx = parse_tx(&args.transaction)?;
    let restored = ctx.transfers.unmerge(&tx).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "restored": restored.to_string() }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Unmerged {tx}; restored {restored}.");
    }
    Ok(())
}

/// Executes `suggest-transfers`.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] on a query failure.
#[inline]
pub async fn suggest(_args: SuggestArgs, ctx: &AppContext) -> CliResult<()> {
    let suggestions = ctx.transfers.suggest_transfers().await?;
    if ctx.json {
        return crate::output::print_json(&suggestions);
    }
    if suggestions.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No transfer suggestions.");
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = suggestions
        .iter()
        .map(|s| {
            vec![
                s.debit().to_string(),
                s.credit().to_string(),
                format!("{} {}", s.amount.value(), s.amount.commodity()),
                s.date_debit.to_string(),
                s.date_credit.to_string(),
            ]
        })
        .collect();
    crate::output::print_table(
        &["DEBIT", "CREDIT", "AMOUNT", "DATE(DEBIT)", "DATE(CREDIT)"],
        &rows,
    );
    Ok(())
}
