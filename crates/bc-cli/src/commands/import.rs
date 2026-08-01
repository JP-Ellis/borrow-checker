//! Import sub-command.

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `import` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The import operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available import operations.
#[derive(Debug, clap::Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Run an import using a named profile.
    Run(RunArgs),
    /// List import runs, newest first.
    List,
    /// Discard an import run, undoing what it created.
    Discard(DiscardArgs),
}

/// Arguments for `import run`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Name of the import profile to use.
    #[arg(long, value_name = "NAME")]
    pub profile: String,
}

/// Executes the `import` subcommand.
///
/// # Arguments
///
/// * `args` - The parsed arguments, naming the operation.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the chosen operation fails.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::Run(run) => execute_run(run, ctx).await,
        Command::List => execute_list(ctx).await,
        Command::Discard(discard) => execute_discard(discard, ctx).await,
    }
}

/// Executes the `import run` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the profile does not exist, or the
/// importer fails to source and parse its configured files.
#[inline]
pub async fn execute_run(args: RunArgs, ctx: &AppContext) -> CliResult<()> {
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

/// Executes `import list`.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the batches cannot be read.
async fn execute_list(ctx: &AppContext) -> CliResult<()> {
    let batches = ctx.batches.list().await?;

    if ctx.json {
        return crate::output::print_json(&list_to_json(&batches));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", render_list(&batches));
    }
    Ok(())
}

/// Arguments for `import discard`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct DiscardArgs {
    /// ID of the batch to discard, as shown by `import list`.
    #[arg(value_name = "BATCH_ID")]
    pub batch: String,
}

/// Executes `import discard`.
///
/// # Arguments
///
/// * `args` - The parsed arguments, naming the batch.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the ID is malformed, the batch does
/// not exist or has already been discarded, or the snapshot cannot be written.
async fn execute_discard(args: DiscardArgs, ctx: &AppContext) -> CliResult<()> {
    let batch_id = args
        .batch
        .parse::<bc_models::ImportBatchId>()
        .map_err(|e| crate::error::CliError::Arg(format!("invalid batch id: {e}")))?;

    // Resolved before the snapshot so a mistyped ID costs nothing.
    let record = ctx.batches.find_by_id(&batch_id).await?;

    // Discard deletes postings and transactions outright. Restoring this
    // snapshot is the recovery path; the audit event carries counts, not a
    // copy of what was removed.
    if ctx.auto_pre_discard {
        let snapshot = ctx
            .backup
            .backup(bc_core::BackupKind::PreDiscard, None)
            .await?;
        tracing::info!(path = %snapshot.path.display(), "pre-discard snapshot taken");
    }

    let outcome = ctx.batches.discard(&batch_id).await?;

    if ctx.json {
        return crate::output::print_json(&discard_to_json(&outcome));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", render_discard(&outcome, &record));
    }
    Ok(())
}

/// Renders the human-readable discard report.
///
/// Every line after the first is a warning about work the discard touched that
/// the import did not create, so each is printed only when it has something to
/// say. None of them stops the discard.
///
/// # Arguments
///
/// * `outcome` - What the discard removed.
/// * `batch` - The batch record, for the header.
///
/// # Returns
///
/// The report, newline-terminated.
fn render_discard(outcome: &bc_core::DiscardOutcome, batch: &bc_core::ImportBatch) -> String {
    let mut lines: Vec<String> = vec![
        format!(
            "Discarded batch {} ({}, {}).",
            outcome.batch_id, batch.importer, batch.started_at
        ),
        format!(
            "  {} removed, {} removed",
            plural(outcome.removed_postings, "posting"),
            plural(outcome.removed_transactions, "transaction"),
        ),
    ];

    if outcome.edited_postings > 0 {
        lines.push(format!(
            "  {} of them you had edited since the import",
            outcome.edited_postings,
        ));
    }
    if outcome.reconciled_postings > 0 {
        lines.push(format!(
            "  {} of them sat in reconciled transactions",
            outcome.reconciled_postings,
        ));
    }
    if outcome.detached_adopted > 0 {
        lines.push(format!(
            "  {} kept — their provenance was adopted, not imported",
            plural(outcome.detached_adopted, "posting"),
        ));
    }
    if outcome.freed_tombstones > 0 {
        lines.push(format!(
            "  {} freed for legs you had deleted",
            plural(outcome.freed_tombstones, "slot"),
        ));
    }
    if outcome.other_batch_references_removed > 0 {
        lines.push(format!(
            "  {} from other batches removed with their transactions",
            plural(outcome.other_batch_references_removed, "reference"),
        ));
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Renders the machine-readable discard report.
///
/// Built from the same outcome as [`render_discard`], so the two surfaces
/// cannot drift apart.
///
/// # Arguments
///
/// * `outcome` - What the discard removed.
///
/// # Returns
///
/// The payload `--json` prints.
fn discard_to_json(outcome: &bc_core::DiscardOutcome) -> serde_json::Value {
    serde_json::json!({
        "batch": outcome.batch_id.to_string(),
        "removed_postings": outcome.removed_postings,
        "removed_transactions": outcome.removed_transactions,
        "detached_adopted": outcome.detached_adopted,
        "freed_tombstones": outcome.freed_tombstones,
        "other_batch_references_removed": outcome.other_batch_references_removed,
        "edited_postings": outcome.edited_postings,
        "reconciled_postings": outcome.reconciled_postings,
    })
}

/// Describes what a run did, in one column.
///
/// A run that never finished reports `incomplete` rather than the zeros its
/// unrecorded counts would otherwise render as: those zeros sit beside rows
/// proving the run wrote something, and reading them as a result is what makes
/// the bad batch the one a user skips over.
///
/// # Arguments
///
/// * `batch` - The run to describe.
///
/// # Returns
///
/// The outcome column's text.
fn outcome_of(batch: &bc_core::ImportBatch) -> String {
    if batch.discarded_at.is_some() {
        return "discarded".to_owned();
    }
    match &batch.counts {
        None => "incomplete".to_owned(),
        Some(counts) => format!(
            "{} new, {} attached, {} skipped",
            counts.new_transactions,
            counts.attached_postings,
            counts.skipped()
        ),
    }
}

/// Renders the profile column: the profile's ID, or `-` if the run was not
/// profile-driven.
///
/// A run's importer alone does not distinguish it from another run: two
/// profiles can share the same importer (two CSV bank profiles, say), and the
/// profile is what tells them apart in this table.
///
/// # Arguments
///
/// * `batch` - The run to describe.
///
/// # Returns
///
/// The profile column's text.
fn profile_of(batch: &bc_core::ImportBatch) -> String {
    batch
        .profile_id
        .as_ref()
        .map_or_else(|| "-".to_owned(), ToString::to_string)
}

/// Renders the human-readable batch listing.
///
/// Columns are `BATCH`, `STARTED`, `IMPORTER`, `PROFILE`, `OUTCOME`. Column
/// widths match the widest real value each column holds (a full
/// [`bc_models::ImportBatchId`] or [`bc_models::ProfileId`], and an RFC 3339
/// timestamp), so the header and every row line up.
///
/// # Arguments
///
/// * `batches` - The runs to list, newest first.
///
/// # Returns
///
/// The listing, newline-terminated.
fn render_list(batches: &[bc_core::ImportBatch]) -> String {
    if batches.is_empty() {
        return "No import runs recorded.\n".to_owned();
    }

    let mut lines: Vec<String> = vec![format!(
        "{:<39}  {:<30}  {:<10}  {:<34}  {}",
        "BATCH", "STARTED", "IMPORTER", "PROFILE", "OUTCOME"
    )];
    lines.extend(batches.iter().map(|batch| {
        format!(
            "{:<39}  {:<30}  {:<10}  {:<34}  {}",
            batch.id,
            batch.started_at,
            batch.importer,
            profile_of(batch),
            outcome_of(batch)
        )
    }));
    lines.push(String::new());
    lines.join("\n")
}

/// Renders the machine-readable batch listing.
///
/// Built from the same values as [`render_list`], so the two surfaces cannot
/// drift apart.
///
/// # Arguments
///
/// * `batches` - The runs to list, newest first.
///
/// # Returns
///
/// The payload `--json` prints.
fn list_to_json(batches: &[bc_core::ImportBatch]) -> serde_json::Value {
    serde_json::json!({
        "batches": batches
            .iter()
            .map(|batch| serde_json::json!({
                "id": batch.id.to_string(),
                "importer": batch.importer,
                "profile": batch.profile_id.as_ref().map(ToString::to_string),
                "started_at": batch.started_at.to_string(),
                "finished_at": batch.finished_at.map(|ts| ts.to_string()),
                "discarded_at": batch.discarded_at.map(|ts| ts.to_string()),
                "new_transactions": batch.counts.map(|c| c.new_transactions),
                "attached_postings": batch.counts.map(|c| c.attached_postings),
                "skipped_postings": batch.counts.map(|c| c.skipped()),
                "unresolved_path_postings": batch.counts.map(|c| c.unresolved_path_postings),
                "other_skipped_postings": batch.counts.map(|c| c.other_skipped_postings),
            }))
            .collect::<Vec<_>>(),
    })
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

    use super::Command;
    use super::Report;

    /// Wrapper needed because `Args` is a subcommand arg group.
    #[derive(clap::Parser)]
    struct Wrap {
        #[command(flatten)]
        args: super::Args,
    }

    #[test]
    fn run_takes_the_profile() {
        let wrap = Wrap::try_parse_from(["bc", "run", "--profile", "Bank"]).expect("parse");
        let Command::Run(run) = wrap.args.command else {
            panic!("expected the run subcommand");
        };
        assert_eq!(run.profile, "Bank");
    }

    #[test]
    fn a_bare_profile_flag_is_rejected() {
        // `import --profile X` was the old shape; it must not silently keep working.
        assert!(Wrap::try_parse_from(["bc", "--profile", "Bank"]).is_err());
    }

    #[test]
    fn account_flag_is_rejected() {
        let rejected =
            Wrap::try_parse_from(["x", "run", "--profile", "bank", "--account", "acc-1"]);
        assert!(
            rejected.is_err(),
            "--account is gone: every leg names its own account path"
        );
    }

    #[test]
    fn a_trailing_file_positional_is_rejected() {
        let rejected = Wrap::try_parse_from(["x", "run", "--profile", "bank", "some/file.csv"]);
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
            auto_pre_discard: true,
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

        super::execute_run(
            super::RunArgs {
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

        super::execute_run(
            super::RunArgs {
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

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn list_renders_each_outcome_state(pool: sqlx::SqlitePool) {
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let done = batches.open(None, "csv").await.expect("open");
        let mut done_counts = bc_core::ImportBatchCounts::default();
        done_counts.new_transactions = 12;
        done_counts.attached_postings = 3;
        batches.close(&done, done_counts).await.expect("close");
        let thrown_away = batches.open(None, "ofx").await.expect("open");
        batches
            .close(&thrown_away, bc_core::ImportBatchCounts::default())
            .await
            .expect("close");
        batches.discard(&thrown_away).await.expect("discard");
        let _open = batches.open(None, "ledger").await.expect("open");
        let nothing_to_do = batches.open(None, "beancount").await.expect("open");
        batches
            .close(&nothing_to_do, bc_core::ImportBatchCounts::default())
            .await
            .expect("close");

        let listed = batches.list().await.expect("list");
        let rendered = super::render_list(&listed);

        assert!(rendered.contains("PROFILE"), "the profile column is named");
        assert!(rendered.contains("incomplete"), "an open run says so");
        assert!(rendered.contains("discarded"), "a discarded run says so");
        assert!(
            rendered.contains("12"),
            "a completed run reports what it did"
        );

        let nothing_to_do_line = rendered
            .lines()
            .find(|line| line.contains("beancount"))
            .expect("the completed-but-empty run has a row");
        assert!(
            nothing_to_do_line.contains("0 new, 0 attached, 0 skipped"),
            "a run that completed but did nothing reports zeros, not silence"
        );
        assert!(
            !nothing_to_do_line.contains("incomplete"),
            "a completed run reporting zeros must read differently from one that never \
             finished; collapsing the two hides a batch that legitimately imported nothing"
        );
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn an_empty_list_says_so(pool: sqlx::SqlitePool) {
        let batches = bc_core::ImportBatchService::new(pool);
        let listed = batches.list().await.expect("list");
        assert!(super::render_list(&listed).contains("No import"));
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn the_discard_report_names_only_what_happened(pool: sqlx::SqlitePool) {
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let id = batches.open(None, "csv").await.expect("open");
        batches
            .close(&id, bc_core::ImportBatchCounts::default())
            .await
            .expect("close");
        let record = batches.find_by_id(&id).await.expect("find");
        let outcome = batches.discard(&id).await.expect("discard");

        let rendered = super::render_discard(&outcome, &record);

        assert!(rendered.contains("0 postings removed"));
        assert!(
            !rendered.contains("adopted"),
            "a line with nothing to report is not printed"
        );
        assert!(!rendered.contains("edited"));
        assert!(!rendered.contains("reconciled"));
        assert!(!rendered.contains("freed"));
        assert!(!rendered.contains("other batches"));
    }

    /// Builds a context over a temporary database, with a batch already open
    /// and closed, ready to discard.
    ///
    /// # Arguments
    ///
    /// * `home` - Directory to hold the database and the backup directory.
    /// * `auto_pre_discard` - The setting under test.
    ///
    /// # Returns
    ///
    /// The context, the directory backups are written to, and the ID of the
    /// closed batch.
    async fn context_with_a_batch(
        home: &std::path::Path,
        auto_pre_discard: bool,
    ) -> (
        crate::context::AppContext,
        std::path::PathBuf,
        bc_models::ImportBatchId,
    ) {
        let (mut ctx, backup_dir) = context_in(home, true).await;
        ctx.auto_pre_discard = auto_pre_discard;

        let batch_id = ctx.batches.open(None, "stub").await.expect("open");
        ctx.batches
            .close(&batch_id, bc_core::ImportBatchCounts::default())
            .await
            .expect("close");

        (ctx, backup_dir, batch_id)
    }

    /// Counts `pre-discard` snapshots in `dir`.
    fn pre_discard_snapshots(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".pre-discard.sqlite")
                })
                .count()
        })
    }

    #[tokio::test]
    async fn a_discard_snapshots_the_database_first() {
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir, batch_id) = context_with_a_batch(home.path(), true).await;

        super::execute_discard(
            super::DiscardArgs {
                batch: batch_id.to_string(),
            },
            &ctx,
        )
        .await
        .expect("discard");

        assert_eq!(pre_discard_snapshots(&backup_dir), 1);
    }

    #[tokio::test]
    async fn auto_pre_discard_false_suppresses_the_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir, batch_id) = context_with_a_batch(home.path(), false).await;

        super::execute_discard(
            super::DiscardArgs {
                batch: batch_id.to_string(),
            },
            &ctx,
        )
        .await
        .expect("discard");

        assert_eq!(pre_discard_snapshots(&backup_dir), 0);
    }

    #[test]
    fn an_unparsable_batch_id_is_an_argument_error() {
        // Parsing happens before any database work, so a typo does not snapshot.
        "not-a-batch-id"
            .parse::<bc_models::ImportBatchId>()
            .expect_err("not a valid batch id");
    }
}
