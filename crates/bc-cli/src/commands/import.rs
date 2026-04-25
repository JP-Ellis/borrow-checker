//! Import sub-command.

use core::str::FromStr as _;
use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `import` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Name of the import profile to use.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Account ID for the offsetting (counterpart) posting.
    ///
    /// CSV and OFX imports produce single-account raw transactions.
    /// This account receives the balancing entry for each imported line.
    #[arg(long, value_name = "ACCOUNT_ID")]
    pub counterpart: String,

    /// File to import.
    pub file: PathBuf,
}

/// Executes the `import` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the profile does not exist, the
/// file cannot be read, or the importer fails to parse it.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    // Resolve counterpart account ID.
    let counterpart_id = bc_models::AccountId::from_str(&args.counterpart).map_err(|e| {
        crate::error::CliError::Arg(format!(
            "invalid counterpart account ID '{}': {e}",
            args.counterpart
        ))
    })?;

    // Find the import profile by name.
    let profiles = ctx.profiles.list_all().await?;
    let profile = profiles
        .iter()
        .find(|p| p.name == args.profile)
        .ok_or_else(|| {
            crate::error::CliError::Core(bc_core::BcError::NotFound(format!(
                "import profile '{}'",
                args.profile
            )))
        })?;

    // Read the file.
    let bytes = std::fs::read(&args.file).map_err(crate::error::CliError::Io)?;

    // Create the importer.
    let importer = ctx
        .importers
        .create_for_name(&profile.importer)
        .ok_or_else(|| {
            crate::error::CliError::Arg(format!(
                "unknown importer '{}' for profile '{}'",
                profile.importer, profile.name
            ))
        })?;

    // Parse the file.
    let raw_txs = importer
        .import(&bytes, &profile.config)
        .map_err(|e| crate::error::CliError::Arg(format!("import parse error: {e}")))?;

    let account_id = profile.account_id.clone();
    // NOTE: Import does not yet deduplicate — running the same file twice will create
    // duplicate transactions. Deduplication (ContentHash / FitId strategies) is
    // deferred to a later milestone once the full import pipeline matures.
    for raw in &raw_txs {
        let posting_account = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id.clone())
            .amount(raw.amount.clone())
            .build();

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "financial negation: Decimal arithmetic is bounded by the type"
        )]
        let negated = -raw.amount.value();
        let counterpart_amount = bc_models::Amount::new(negated, raw.amount.commodity().clone());
        let posting_counterpart = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(counterpart_id.clone())
            .amount(counterpart_amount)
            .build();

        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(raw.date)
            .maybe_payee(raw.payee.clone())
            .description(raw.description.clone())
            .postings(vec![posting_account, posting_counterpart])
            .status(bc_models::TransactionStatus::Cleared)
            .created_at(jiff::Timestamp::now())
            .build();

        ctx.transactions.create(tx).await?;
    }

    let count = raw_txs.len();
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "imported": count }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Imported {count} transactions.");
    }
    Ok(())
}
