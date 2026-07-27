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

    let report = Report::from(&outcome);
    if ctx.json {
        return crate::output::print_json(&report.to_json(&outcome.batch_id.to_string()));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", report.render());
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
            lines.push(format!(
                "Create {} and re-run to import {}.",
                if self.unresolved_paths.len() == 1 {
                    "it"
                } else {
                    "these accounts"
                },
                if self.unresolved_path_postings == 1 {
                    "that posting"
                } else {
                    "those postings"
                },
            ));
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

    /// Renders the machine-readable report for a completed import run.
    ///
    /// Built from the same numbers as [`Report::render`], so the two surfaces
    /// cannot drift apart.
    ///
    /// # Arguments
    ///
    /// * `batch` - Identifier of the batch recording the run.
    ///
    /// # Returns
    ///
    /// The payload `--json` prints.
    fn to_json(&self, batch: &str) -> serde_json::Value {
        serde_json::json!({
            "batch": batch,
            "new_transactions": self.new_transactions,
            "attached_postings": self.attached_postings,
            "skipped_postings": self
                .unresolved_path_postings
                .saturating_add(self.other_skipped_postings),
            "unresolved_path_postings": self.unresolved_path_postings,
            "other_skipped_postings": self.other_skipped_postings,
            "unresolved_paths": self.unresolved_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use pretty_assertions::assert_eq;

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

    #[test]
    fn the_json_payload_names_every_number_and_its_cause() {
        let unresolved = paths(&["Expenses:Fun", "Expenses:Rent"]);
        let payload = report(3, 2, 4, 1, &unresolved).to_json("import_batch_stub");

        assert_eq!(
            payload,
            serde_json::json!({
                "batch": "import_batch_stub",
                "new_transactions": 3_usize,
                "attached_postings": 2_usize,
                "skipped_postings": 5_usize,
                "unresolved_path_postings": 4_usize,
                "other_skipped_postings": 1_usize,
                "unresolved_paths": ["Expenses:Fun", "Expenses:Rent"],
            }),
            "a script reads these keys; renaming one or dropping the cause split is a \
             breaking change"
        );
    }

    /// An importer that yields one single-leg transaction, so a run can be
    /// exercised without a compiled WASM plugin.
    struct StubImporter;

    impl bc_core::Importer for StubImporter {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn import(
            &self,
            _config: &bc_core::ImportConfig,
        ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
            Ok(vec![
                bc_core::RawTransaction::builder()
                    .date(jiff::civil::date(2026, 3, 14))
                    .description("STUB ROW")
                    .postings(vec![
                        bc_core::RawPosting::builder()
                            .account("Assets:Bank")
                            .amount(bc_models::Amount::new(
                                rust_decimal::Decimal::from(-25_i64),
                                bc_models::CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .build(),
            ])
        }
    }

    /// Builds the stub importer behind the `stub` name.
    fn make_stub() -> Box<dyn bc_core::Importer> {
        Box::new(StubImporter)
    }

    /// Builds a context over a temporary database, with `stub` registered and a
    /// `nightly` profile driving it.
    ///
    /// # Arguments
    ///
    /// * `home` - Directory to hold the database and the backup directory.
    /// * `auto_pre_import` - The setting under test.
    ///
    /// # Returns
    ///
    /// The context, and the directory backups are written to.
    async fn context_in(
        home: &std::path::Path,
        auto_pre_import: bool,
    ) -> (crate::context::AppContext, std::path::PathBuf) {
        let db_path = home.join("test.db");
        let backup_dir = home.join("backups");
        let policy =
            bc_core::BackupPolicy::new(backup_dir.clone(), Some(5_u32), Some(30_u32), false);
        let pool = bc_core::open_db_with_backup(&db_path, &policy)
            .await
            .expect("open database");

        let mut importers = bc_core::ImporterRegistry::new();
        importers.register(bc_core::ImporterFactory::new("stub", make_stub));

        let profiles = bc_core::ImportProfileService::new(pool.clone());
        profiles
            .create(
                "nightly",
                "stub",
                bc_core::ImportConfig::from_value(serde_json::json!({})),
            )
            .await
            .expect("create profile");

        let ctx = crate::context::AppContext {
            json: false,
            fortnightly_anchor: None,
            plugin_registry: bc_plugins::PluginRegistry::load(&[], None)
                .expect("empty plugin registry"),
            importers,
            accounts: bc_core::AccountService::new(pool.clone()),
            transactions: bc_core::TransactionService::new(pool.clone()),
            balances: bc_core::BalanceEngine::new(pool.clone()),
            profiles,
            assets: bc_core::AssetService::new(pool.clone()),
            loans: bc_core::LoanService::new(pool.clone()),
            budgets: bc_core::BudgetService::new(pool.clone()),
            tags: bc_core::TagService::new(pool.clone()),
            backup: bc_core::BackupService::new(pool.clone(), db_path.clone(), policy),
            db_path,
            sources: bc_core::SourceService::new(pool.clone()),
            transfers: bc_core::TransferService::new(pool.clone()),
            batches: bc_core::ImportBatchService::new(pool.clone()),
            auto_pre_import,
            budget_status: bc_core::BudgetStatusEngine::new(pool, bc_core::noop_fx()),
        };
        (ctx, backup_dir)
    }

    /// Counts `pre-import` snapshots in `dir`.
    fn pre_import_snapshots(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".pre-import.sqlite")
                })
                .count()
        })
    }

    #[tokio::test]
    async fn an_import_snapshots_the_database_first() {
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir) = context_in(home.path(), true).await;

        super::execute(
            super::Args {
                profile: "nightly".to_owned(),
            },
            &ctx,
        )
        .await
        .expect("import runs");

        assert_eq!(
            pre_import_snapshots(&backup_dir),
            1,
            "a misconfigured profile writes plausible-looking wrong data whose source \
             references then suppress a corrected re-import; restoring is the recovery path, \
             so the snapshot must exist"
        );
    }

    #[tokio::test]
    async fn auto_pre_import_false_suppresses_the_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir) = context_in(home.path(), false).await;

        super::execute(
            super::Args {
                profile: "nightly".to_owned(),
            },
            &ctx,
        )
        .await
        .expect("import runs");

        assert_eq!(
            pre_import_snapshots(&backup_dir),
            0,
            "the setting must actually suppress the snapshot, not merely exist"
        );
    }
}
