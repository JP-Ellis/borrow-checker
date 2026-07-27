//! Import sub-command.

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `import` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Name of the import profile to use.
    #[arg(long, value_name = "NAME")]
    pub profile: String,
}

/// Executes the `import` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the profile does not exist, or the
/// importer fails to source and parse its configured files.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    // Find the import profile by its unique name.
    let profile = ctx.profiles.find_by_name(&args.profile).await?;

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

    // Snapshot before writing: a misconfigured profile (wrong date format, an
    // inverted sign convention) produces plausible-looking wrong data whose
    // source references then suppress a corrected re-import. Restoring is the
    // recovery path (see #343).
    if ctx.auto_pre_import {
        let record = ctx
            .backup
            .backup(bc_core::BackupKind::PreImport, None)
            .await?;
        tracing::info!(path = %record.path.display(), "pre-import snapshot taken");
    }

    // Source and parse the profile's files (the importer reads them itself).
    let raw_txs = importer
        .import(&profile.config)
        .map_err(|e| crate::error::CliError::Arg(format!("import error: {e}")))?;

    let outcome = bc_core::execute_import(
        &ctx.transactions,
        &ctx.sources,
        &ctx.accounts,
        &ctx.batches,
        Some(&profile.id),
        &profile.importer,
        &raw_txs,
    )
    .await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "batch": outcome.batch_id.to_string(),
            "new_transactions": outcome.new_transactions,
            "attached_postings": outcome.attached_postings,
            "skipped_postings": outcome.skipped_postings,
            "unresolved_paths": outcome.unresolved_paths,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!(
            "Imported {} transactions, attached {} postings.",
            outcome.new_transactions, outcome.attached_postings
        );
        if !outcome.unresolved_paths.is_empty() {
            println!(
                "\nSkipped {} postings naming {} unknown account(s):",
                outcome.skipped_postings,
                outcome.unresolved_paths.len()
            );
            for path in &outcome.unresolved_paths {
                println!("  {path}");
            }
            println!("\nCreate these accounts and re-run to import the skipped postings.");
        } else if outcome.skipped_postings > 0 {
            println!(
                "Skipped {} postings; see warnings.",
                outcome.skipped_postings
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    /// Wrapper needed because `Args` is a subcommand arg group.
    #[derive(clap::Parser)]
    struct Wrap {
        #[command(flatten)]
        args: super::Args,
    }

    #[test]
    fn profile_is_the_only_required_argument() {
        let ok = Wrap::try_parse_from(["x", "--profile", "bank"]);
        assert!(ok.is_ok(), "--profile alone must parse");
    }

    #[test]
    fn account_flag_is_rejected() {
        let rejected = Wrap::try_parse_from(["x", "--profile", "bank", "--account", "acc-1"]);
        assert!(
            rejected.is_err(),
            "--account is gone: every leg names its own account path"
        );
    }

    #[test]
    fn a_trailing_file_positional_is_rejected() {
        let rejected = Wrap::try_parse_from(["x", "--profile", "bank", "some/file.csv"]);
        assert!(rejected.is_err(), "importers source their own files");
    }
}
