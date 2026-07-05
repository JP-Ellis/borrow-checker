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
    let count = bc_core::execute_import(
        &ctx.transactions,
        &ctx.sources,
        &account_id,
        &counterpart_id,
        &raw_txs,
    )
    .await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "imported": count }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Imported {count} transactions.");
    }
    Ok(())
}
