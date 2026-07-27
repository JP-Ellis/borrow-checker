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
            "unresolved_path_postings": outcome.unresolved_path_postings,
            "other_skipped_postings": outcome.other_skipped_postings,
            "unresolved_paths": outcome.unresolved_paths,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", Report::from(&outcome).render());
    }
    Ok(())
}

/// Renders `count` with `noun`, pluralised.
///
/// # Arguments
///
/// * `count` - How many.
/// * `noun` - The singular noun; an `s` is appended unless `count` is 1.
///
/// # Returns
///
/// A phrase such as `1 posting` or `4 postings`.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The numbers the human-readable report is built from.
///
/// A view over [`bc_core::ImportOutcome`], which is `#[non_exhaustive]` and so
/// cannot be constructed in tests; this can.
struct Report<'out> {
    /// Transactions created by the run.
    new_transactions: usize,
    /// Postings appended to transactions an earlier run created.
    attached_postings: usize,
    /// Postings skipped because their account path named no existing account.
    unresolved_path_postings: usize,
    /// Postings skipped for any other reason.
    other_skipped_postings: usize,
    /// The distinct account paths naming no account.
    unresolved_paths: &'out [String],
}

impl<'out> From<&'out bc_core::ImportOutcome> for Report<'out> {
    #[inline]
    fn from(outcome: &'out bc_core::ImportOutcome) -> Self {
        Self {
            new_transactions: outcome.new_transactions,
            attached_postings: outcome.attached_postings,
            unresolved_path_postings: outcome.unresolved_path_postings,
            other_skipped_postings: outcome.other_skipped_postings,
            unresolved_paths: &outcome.unresolved_paths,
        }
    }
}

impl Report<'_> {
    /// Renders the human-readable report for a completed import run.
    ///
    /// Every number is attributed to its cause: legs skipped because their
    /// account path named no account are actionable — create the accounts and
    /// re-run — while legs skipped for any other reason were each warned about
    /// as they happened. Reporting one total against the unresolved paths would
    /// misattribute the rest.
    ///
    /// # Returns
    ///
    /// The report, newline-terminated.
    fn render(&self) -> String {
        let mut lines: Vec<String> = vec![format!(
            "Imported {}, attached {}.",
            plural(self.new_transactions, "transaction"),
            plural(self.attached_postings, "posting"),
        )];

        if !self.unresolved_paths.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Skipped {} naming {}:",
                plural(self.unresolved_path_postings, "posting"),
                plural(self.unresolved_paths.len(), "unknown account"),
            ));
            lines.extend(self.unresolved_paths.iter().map(|path| format!("  {path}")));
            lines.push(
                if self.unresolved_paths.len() == 1 {
                    "Create it and re-run to import those postings."
                } else {
                    "Create these accounts and re-run to import those postings."
                }
                .to_owned(),
            );
        }

        if self.other_skipped_postings > 0 {
            lines.push(String::new());
            lines.push(format!(
                "Skipped {} for other reasons; each was reported as a warning.",
                plural(self.other_skipped_postings, "posting"),
            ));
        }

        lines.push(String::new());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::Report;

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

    /// A report over the given counts and unresolved paths.
    fn report(
        new_transactions: usize,
        attached_postings: usize,
        unresolved_path_postings: usize,
        other_skipped_postings: usize,
        unresolved_paths: &[String],
    ) -> Report<'_> {
        Report {
            new_transactions,
            attached_postings,
            unresolved_path_postings,
            other_skipped_postings,
            unresolved_paths,
        }
    }

    /// Owned paths, so the borrowed slice in [`Report`] has something to point at.
    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn report_for_a_clean_run() {
        insta::assert_snapshot!(report(3, 0, 0, 0, &[]).render());
    }

    #[test]
    fn report_for_a_partial_run_with_unknown_accounts() {
        // Some new, some attached, two unresolved paths accounting for three legs.
        let unresolved = paths(&["Assets:Brokerage", "Expenses:Fun"]);
        insta::assert_snapshot!(report(2, 1, 3, 0, &unresolved).render());
    }

    #[test]
    fn report_for_a_run_skipping_legs_for_other_reasons() {
        insta::assert_snapshot!(report(1, 0, 0, 2, &[]).render());
    }

    #[test]
    fn report_attributes_both_causes_separately() {
        let unresolved = paths(&["Expenses:Fun"]);
        insta::assert_snapshot!(report(1, 1, 1, 1, &unresolved).render());
    }
}
