//! Import sub-command.

use rust_decimal::Decimal;

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

    /// Report what the import would do, without writing anything.
    ///
    /// Resolves exactly as a real run does and prints the same decisions, but
    /// opens no batch, creates no tags and persists no postings. Takes no
    /// pre-import snapshot: there is nothing to snapshot before.
    #[arg(long)]
    pub dry_run: bool,
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

    // Source and parse the profile's files (the importer reads them itself).
    // This happens before any snapshot: it reads files and writes nothing, so
    // a dry run can use it without taking a backup first.
    let raw_txs = importer
        .import(&profile.config)
        .map_err(|e| crate::error::CliError::Arg(format!("import error: {e}")))?;

    if args.dry_run {
        let plan = bc_core::plan_import(
            &ctx.transactions,
            &ctx.sources,
            &ctx.accounts,
            &ctx.commodities,
            &ctx.tags,
            &ctx.batches,
            Some(&profile.id),
            &profile.importer,
            &raw_txs,
        )
        .await?;
        let report = PlanReport::from(&plan);

        if ctx.json {
            return crate::output::print_json(&report.to_json());
        }

        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            print!("{}", render_plan(&report, &profile.name, &profile.importer));
        }
        return Ok(());
    }

    // Snapshot before writing: a misconfigured profile (wrong date format, an
    // inverted sign convention) produces plausible-looking wrong data whose
    // source references then suppress a corrected re-import. Restoring is one
    // recovery path; `import discard` is the other, and unlike a restore it
    // keeps everything else that has happened since.
    if ctx.auto_pre_import {
        let record = ctx
            .backup
            .backup(bc_core::BackupKind::PreImport, None)
            .await?;
        tracing::info!(path = %record.path.display(), "pre-import snapshot taken");
    }

    let outcome = bc_core::execute_import(
        &ctx.transactions,
        &ctx.sources,
        &ctx.accounts,
        &ctx.commodities,
        &ctx.tags,
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

    // Rejected before the snapshot too: core's `discard` is the authoritative
    // guard against a repeat, but it only raises the error after this
    // function has already written a full database copy. `ensure_discardable`
    // runs the same predicate and message as a cheap short-circuit, so this
    // check and `discard`'s own cannot drift apart.
    ctx.batches.ensure_discardable(&batch_id).await?;

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
        return crate::output::print_json(&discard_to_json(&outcome, &record));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        print!("{}", render_discard(&outcome, &record));
    }
    Ok(())
}

/// Renders the human-readable discard report.
///
/// The header and the totals line always print. Every line after those is a
/// warning — about the user's own work the discard destroyed, or about what it
/// touched beyond the batch — and prints only when its own count is non-zero.
/// None of them stops the discard.
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
    if outcome.flagged_postings > 0 {
        lines.push(format!(
            "  {} of them sat in flagged transactions",
            outcome.flagged_postings,
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
            "  {} removed with {} transaction{}",
            plural(outcome.other_batch_references_removed, "other reference"),
            if outcome.other_batch_references_removed == 1 {
                "its"
            } else {
                "their"
            },
            if outcome.other_batch_references_removed == 1 {
                ""
            } else {
                "s"
            },
        ));
    }
    if outcome.other_batch_references_tombstoned > 0 {
        lines.push(format!(
            "  {} from another batch lost {} posting",
            plural(outcome.other_batch_references_tombstoned, "reference"),
            if outcome.other_batch_references_tombstoned == 1 {
                "its"
            } else {
                "their"
            },
        ));
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Renders the machine-readable discard report.
///
/// Built from the same outcome and record as [`render_discard`], so the two
/// surfaces cannot drift apart — including the importer and start time, which
/// name the run a script has just destroyed without a second `import list`.
///
/// # Arguments
///
/// * `outcome` - What the discard removed.
/// * `batch` - The batch record, for the identifying fields.
///
/// # Returns
///
/// The payload `--json` prints.
fn discard_to_json(
    outcome: &bc_core::DiscardOutcome,
    batch: &bc_core::ImportBatch,
) -> serde_json::Value {
    serde_json::json!({
        "batch": outcome.batch_id.to_string(),
        "importer": batch.importer,
        "started_at": batch.started_at.to_string(),
        "removed_postings": outcome.removed_postings,
        "removed_transactions": outcome.removed_transactions,
        "detached_adopted": outcome.detached_adopted,
        "freed_tombstones": outcome.freed_tombstones,
        "other_batch_references_removed": outcome.other_batch_references_removed,
        "other_batch_references_tombstoned": outcome.other_batch_references_tombstoned,
        "edited_postings": outcome.edited_postings,
        "reconciled_postings": outcome.reconciled_postings,
        "flagged_postings": outcome.flagged_postings,
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
                "unresolved_account_postings": batch.counts.map(|c| c.unresolved_account_postings),
                "unresolved_commodity_postings": batch.counts.map(|c| c.unresolved_commodity_postings),
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
/// * `noun` - The singular noun. Unless `count` is 1 it gains an `s`, or `ies`
///   in place of a consonant-preceded `y` — enough for the nouns this module
///   reports on (`posting`, `account`, `commodity`), and no more.
///
/// # Returns
///
/// A phrase such as `1 posting`, `4 postings`, or `2 unregistered commodities`.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        return format!("{count} {noun}");
    }
    let consonant_y = noun.strip_suffix('y').filter(|stem| {
        stem.chars()
            .next_back()
            .is_some_and(|last| !matches!(last, 'a' | 'e' | 'i' | 'o' | 'u'))
    });
    match consonant_y {
        Some(stem) => format!("{count} {stem}ies"),
        None => format!("{count} {noun}s"),
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
    unresolved_account_postings: usize,
    /// Postings skipped because their commodity code named no registered commodity.
    unresolved_commodity_postings: usize,
    /// Postings skipped for any other reason.
    other_skipped_postings: usize,
    /// The distinct account paths naming no account.
    unresolved_accounts: &'out [String],
    /// The distinct codes naming no registered commodity.
    unresolved_commodities: &'out [String],
    /// The tag paths the run created.
    created_tags: &'out [String],
}

impl<'out> From<&'out bc_core::ImportOutcome> for Report<'out> {
    #[inline]
    fn from(outcome: &'out bc_core::ImportOutcome) -> Self {
        Self {
            new_transactions: outcome.new_transactions,
            attached_postings: outcome.attached_postings,
            unresolved_account_postings: outcome.unresolved_account_postings,
            unresolved_commodity_postings: outcome.unresolved_commodity_postings,
            other_skipped_postings: outcome.other_skipped_postings,
            unresolved_accounts: &outcome.unresolved_accounts,
            unresolved_commodities: &outcome.unresolved_commodities,
            created_tags: &outcome.created_tags,
        }
    }
}

impl Report<'_> {
    /// Renders the human-readable report for a completed import run.
    ///
    /// Every number is attributed to its cause: legs skipped because their
    /// account path named no account are actionable — create the accounts and
    /// re-run — as are legs whose commodity code named nothing, which need the
    /// commodity registered; legs skipped for any other reason were each warned about
    /// as they happened. Reporting one total against the unresolved accounts
    /// would misattribute the rest.
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

        if !self.unresolved_accounts.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Skipped {} naming {}:",
                plural(self.unresolved_account_postings, "posting"),
                plural(self.unresolved_accounts.len(), "unknown account"),
            ));
            lines.extend(
                self.unresolved_accounts
                    .iter()
                    .map(|path| format!("  {path}")),
            );
            lines.push(format!(
                "Create {} and re-run to import {}.",
                if self.unresolved_accounts.len() == 1 {
                    "it"
                } else {
                    "these accounts"
                },
                if self.unresolved_account_postings == 1 {
                    "that posting"
                } else {
                    "those postings"
                },
            ));
        }

        if !self.unresolved_commodities.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Skipped {} naming {}:",
                plural(self.unresolved_commodity_postings, "posting"),
                plural(self.unresolved_commodities.len(), "unregistered commodity"),
            ));
            lines.extend(
                self.unresolved_commodities
                    .iter()
                    .map(|code| format!("  {code}")),
            );
            lines.push(format!(
                "Register {} and re-run to import {}.",
                if self.unresolved_commodities.len() == 1 {
                    "it"
                } else {
                    "these commodities"
                },
                if self.unresolved_commodity_postings == 1 {
                    "that posting"
                } else {
                    "those postings"
                },
            ));
        }

        if !self.created_tags.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Created {}:",
                plural(self.created_tags.len(), "tag"),
            ));
            lines.extend(self.created_tags.iter().map(|path| format!("  {path}")));
            lines.push(
                "Tags named by the source are created automatically; rename or delete any that \
                 are typos."
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
                .unresolved_account_postings
                .saturating_add(self.unresolved_commodity_postings)
                .saturating_add(self.other_skipped_postings),
            "unresolved_account_postings": self.unresolved_account_postings,
            "unresolved_commodity_postings": self.unresolved_commodity_postings,
            "other_skipped_postings": self.other_skipped_postings,
            "unresolved_accounts": self.unresolved_accounts,
            "unresolved_commodities": self.unresolved_commodities,
            "created_tags": self.created_tags,
        })
    }
}

/// One leg or row a planned run would skip, and why.
///
/// Owned rather than borrowing [`bc_core::Diagnostic`] fields: that type is
/// `#[non_exhaustive]`, so `bc-cli` cannot construct one, and test fixtures
/// need to build these directly.
struct PlanDiagnostic {
    /// Where the document says this came from.
    location: String,
    /// Why it was skipped.
    cause: bc_core::SkipCause,
    /// Human-readable detail: the offending path, code, or conflict.
    detail: String,
}

/// The numbers the dry-run report is built from.
///
/// A view over [`bc_core::ImportPlan`], which is `#[non_exhaustive]` and so
/// cannot be constructed in tests; this can. Unlike [`Report`], this owns its
/// data rather than borrowing it: the diagnostics are grouped and counted
/// during rendering, so a borrowing view would tie the grouped form's
/// lifetime to the plan for no gain.
struct PlanReport {
    /// Transactions the run would create.
    new_transactions: usize,
    /// Legs it would book onto transactions an earlier run created.
    attached_postings: usize,
    /// Postings it could not persist, whatever the cause.
    skipped_postings: usize,
    /// Postings whose account path names no existing account.
    unresolved_account_postings: usize,
    /// Postings whose commodity code names no registered commodity.
    unresolved_commodity_postings: usize,
    /// Postings skipped for any other reason.
    other_skipped_postings: usize,
    /// Account paths that resolve to no account, deduplicated and sorted.
    unresolved_accounts: Vec<String>,
    /// The distinct unregistered codes encountered, sorted.
    unresolved_commodities: Vec<String>,
    /// Tag paths the run would create, sorted.
    would_create_tags: Vec<String>,
    /// Per-account sums of the legs that would post, keyed by rendered
    /// account path, each holding one `(commodity, total)` entry per
    /// commodity touched. An account whose legs net to zero still holds an
    /// entry, with an empty commodity list — touched, but netting to
    /// nothing, which is not the same as untouched.
    account_totals: Vec<(String, Vec<(String, Decimal)>)>,
    /// Postings charged to each cause, in the cause's declaration order.
    /// This is what the skipped-legs table's count column renders — not the
    /// same as counting `diagnostics` per cause, since one diagnostic can
    /// charge several postings (or none at all).
    charged_by_cause: Vec<(bc_core::SkipCause, usize)>,
    /// Every leg or row the run would skip, with its cause and location.
    diagnostics: Vec<PlanDiagnostic>,
}

impl From<&bc_core::ImportPlan> for PlanReport {
    #[inline]
    fn from(plan: &bc_core::ImportPlan) -> Self {
        Self {
            new_transactions: plan.new_transactions,
            attached_postings: plan.attached_postings,
            skipped_postings: plan.skipped_postings,
            unresolved_account_postings: plan.unresolved_account_postings,
            unresolved_commodity_postings: plan.unresolved_commodity_postings,
            other_skipped_postings: plan.other_skipped_postings,
            unresolved_accounts: plan.unresolved_accounts.clone(),
            unresolved_commodities: plan.unresolved_commodities.clone(),
            would_create_tags: plan.would_create_tags.clone(),
            account_totals: plan
                .account_totals
                .iter()
                .map(|(path, balances)| {
                    (
                        path.clone(),
                        balances
                            .iter()
                            .map(|(code, amount)| (code.to_owned(), amount))
                            .collect(),
                    )
                })
                .collect(),
            charged_by_cause: plan.charged_by_cause.clone(),
            diagnostics: plan
                .diagnostics
                .iter()
                .map(|diagnostic| PlanDiagnostic {
                    location: diagnostic.location.clone(),
                    cause: diagnostic.cause,
                    detail: diagnostic.detail.clone(),
                })
                .collect(),
        }
    }
}

impl PlanReport {
    /// Renders the machine-readable dry-run report.
    ///
    /// Built from the same fields [`render_plan`] draws on, so the two
    /// surfaces cannot drift apart — but unlike the human report, nothing
    /// here is truncated: this is the payload a profile-tuning script reads,
    /// and a silent cap would hide the very rows it is looking for. Carries
    /// no `batch_id` key at all, because a dry run opens no batch — there is
    /// no unknown identifier to report, only the absence of one.
    ///
    /// # Returns
    ///
    /// The payload `--json` prints for a dry run.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "dry_run": true,
            "new_transactions": self.new_transactions,
            "attached_postings": self.attached_postings,
            "skipped_postings": self.skipped_postings,
            "unresolved_account_postings": self.unresolved_account_postings,
            "unresolved_commodity_postings": self.unresolved_commodity_postings,
            "other_skipped_postings": self.other_skipped_postings,
            "unresolved_accounts": self.unresolved_accounts,
            "unresolved_commodities": self.unresolved_commodities,
            "would_create_tags": self.would_create_tags,
            "account_totals": self
                .account_totals
                .iter()
                .flat_map(|(account, balances)| {
                    balances.iter().map(move |(commodity, amount)| {
                        serde_json::json!({
                            "account": account,
                            "commodity": commodity,
                            "amount": amount.to_string(),
                        })
                    })
                })
                .collect::<Vec<_>>(),
            "charged_by_cause": self
                .charged_by_cause
                .iter()
                .map(|(cause, charged)| serde_json::json!({
                    "cause": cause.label(),
                    "charged_postings": charged,
                }))
                .collect::<Vec<_>>(),
            "diagnostics": self
                .diagnostics
                .iter()
                .map(|diagnostic| serde_json::json!({
                    "location": diagnostic.location,
                    "cause": diagnostic.cause.label(),
                    "detail": diagnostic.detail,
                }))
                .collect::<Vec<_>>(),
        })
    }
}

/// How many locations a skip-cause group prints before it truncates.
const MAX_SAMPLE_LOCATIONS: usize = 3;

/// Renders the "skipped" block: a header naming the total, then one line per
/// [`bc_core::SkipCause`], ordered by descending charged-leg count.
///
/// The count column renders [`PlanReport::charged_by_cause`], not the number
/// of diagnostics in the group — a cause charges the postings it costs, and a
/// single diagnostic can speak for several of them (or, for
/// `SkipCause::MalformedTag`, for none). Location samples are a separate
/// concern: they cap at [`MAX_SAMPLE_LOCATIONS`] with an explicit `… N of M`
/// naming how many diagnostics the group holds, whenever the group holds more
/// than that — a silent cap would read as "that is all of them".
///
/// # Arguments
///
/// * `plan` - The report to render the skipped block for.
///
/// # Returns
///
/// The block's lines, not yet joined with the rest of the report.
fn render_skips(plan: &PlanReport) -> Vec<String> {
    let charged: std::collections::BTreeMap<bc_core::SkipCause, usize> =
        plan.charged_by_cause.iter().copied().collect();

    let mut groups: std::collections::BTreeMap<bc_core::SkipCause, Vec<&PlanDiagnostic>> =
        std::collections::BTreeMap::new();
    for diagnostic in &plan.diagnostics {
        groups.entry(diagnostic.cause).or_default().push(diagnostic);
    }
    let mut grouped: Vec<(bc_core::SkipCause, Vec<&PlanDiagnostic>)> = groups.into_iter().collect();
    grouped.sort_by(|left, right| {
        let left_charged = charged.get(&left.0).copied().unwrap_or_default();
        let right_charged = charged.get(&right.0).copied().unwrap_or_default();
        right_charged
            .cmp(&left_charged)
            .then(left.0.label().cmp(right.0.label()))
    });

    let mut lines = vec![format!("skipped {}", plural(plan.skipped_postings, "leg"))];
    for (cause, entries) in grouped {
        let charged_count = charged.get(&cause).copied().unwrap_or_default();
        let location_total = entries.len();
        let locations: Vec<&str> = entries
            .iter()
            .take(MAX_SAMPLE_LOCATIONS)
            .map(|diagnostic| diagnostic.location.as_str())
            .collect();
        let shown = locations.len();
        let listed = locations.join(", ");
        let suffix = if location_total > MAX_SAMPLE_LOCATIONS {
            format!(" … {shown} of {location_total}")
        } else {
            String::new()
        };
        let label = cause.label();
        lines.push(format!(
            "  {label:<24}{charged_count:>6}   {listed}{suffix}"
        ));
    }
    lines
}

/// One row of the `WOULD POST` table.
struct AccountRow {
    /// The account's rendered path.
    path: String,
    /// The numeral, or `nets to zero` when the account's legs net to
    /// nothing — in which case `code` is empty.
    amount: String,
    /// The commodity code, or empty for a `nets to zero` row.
    code: String,
}

/// Renders the per-account `WOULD POST` table.
///
/// One row per `(account, commodity)`, so an account touched in several
/// commodities repeats its path once per row rather than cramming every
/// commodity onto one line. An account whose legs net to zero still gets a
/// row, reading `nets to zero`, so it is not mistaken for an account the run
/// never touched.
///
/// The amount is right-aligned in its own column and the commodity code
/// follows in a second, left-aligned one, so a wrong sign or magnitude is
/// spottable at a glance regardless of how many digits or which code shares
/// the row above or below it.
///
/// # Arguments
///
/// * `plan` - The report to render the account table for.
///
/// # Returns
///
/// The block's lines, not yet joined with the rest of the report.
fn render_account_totals(plan: &PlanReport) -> Vec<String> {
    let mut rows: Vec<AccountRow> = Vec::new();
    for (path, entries) in &plan.account_totals {
        if entries.is_empty() {
            rows.push(AccountRow {
                path: path.clone(),
                amount: "nets to zero".to_owned(),
                code: String::new(),
            });
            continue;
        }
        for (code, amount) in entries {
            rows.push(AccountRow {
                path: path.clone(),
                amount: amount.to_string(),
                code: code.clone(),
            });
        }
    }

    let account_width = rows
        .iter()
        .map(|row| row.path.len())
        .chain(core::iter::once("ACCOUNT".len()))
        .max()
        .unwrap_or_default();
    let amount_width = rows
        .iter()
        .map(|row| row.amount.len())
        .max()
        .unwrap_or_default();

    let mut lines = vec![format!("{:<account_width$}  WOULD POST", "ACCOUNT")];
    lines.extend(rows.iter().map(|row| {
        if row.code.is_empty() {
            format!(
                "{:<account_width$}  {:>amount_width$}",
                row.path, row.amount
            )
        } else {
            format!(
                "{:<account_width$}  {:>amount_width$}  {}",
                row.path, row.amount, row.code
            )
        }
    }));
    lines
}

/// Renders the human-readable dry-run report.
///
/// Leads with what is broken — the unresolved accounts and commodities, then
/// the legs skipped and why — because the report exists for profile tuning:
/// someone iterating on date formats, sign conventions and column indices
/// needs to see what is wrong before what would otherwise succeed. The
/// per-account `WOULD POST` table closes the report, so an inverted sign or a
/// misdirected column shows up immediately.
///
/// Every section but the header and the totals line is omitted when its own
/// count is zero.
///
/// # Arguments
///
/// * `plan` - What the run would do.
/// * `profile` - Name of the profile driving the run.
/// * `importer` - Stable name of the importer the profile uses.
///
/// # Returns
///
/// The report, newline-terminated.
fn render_plan(plan: &PlanReport, profile: &str, importer: &str) -> String {
    let mut blocks: Vec<Vec<String>> = vec![vec![
        format!("dry run — profile '{profile}', importer '{importer}'"),
        "nothing was written".to_owned(),
    ]];

    let mut worklist: Vec<String> = Vec::new();
    if !plan.unresolved_accounts.is_empty() {
        worklist.push(format!(
            "unresolved accounts ({})",
            plan.unresolved_accounts.len()
        ));
        worklist.extend(
            plan.unresolved_accounts
                .iter()
                .map(|path| format!("  {path}")),
        );
    }
    if !plan.unresolved_commodities.is_empty() {
        worklist.push(format!(
            "unregistered commodities ({})",
            plan.unresolved_commodities.len()
        ));
        worklist.push(format!("  {}", plan.unresolved_commodities.join(", ")));
    }
    if !worklist.is_empty() {
        blocks.push(worklist);
    }

    if !plan.diagnostics.is_empty() {
        blocks.push(render_skips(plan));
    }

    let mut totals = vec![format!(
        "would create {}, attach {} to existing",
        plural(plan.new_transactions, "new transaction"),
        plural(plan.attached_postings, "posting"),
    )];
    if !plan.would_create_tags.is_empty() {
        totals.push(format!(
            "would create {}: {}",
            plural(plan.would_create_tags.len(), "tag"),
            plan.would_create_tags.join(", "),
        ));
    }
    blocks.push(totals);

    if !plan.account_totals.is_empty() {
        blocks.push(render_account_totals(plan));
    }

    let mut lines: Vec<String> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(block.iter().cloned());
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use bc_core::SkipCause;
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::ImportBatchId;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::SourceRef;
    use bc_models::TransactionId;
    use clap::Parser as _;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use super::Command;
    use super::PlanDiagnostic;
    use super::PlanReport;
    use super::Report;
    use super::plural;
    use super::render_plan;

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

    #[test]
    fn report_lists_created_tags() {
        let created = vec!["household".to_owned(), "group:alpha".to_owned()];
        let report = Report {
            new_transactions: 1,
            attached_postings: 0,
            unresolved_account_postings: 0,
            unresolved_commodity_postings: 0,
            other_skipped_postings: 0,
            unresolved_accounts: &[],
            unresolved_commodities: &[],
            created_tags: &created,
        };

        insta::assert_snapshot!(report.render());
    }

    /// A report over the given counts, unresolved accounts and commodities.
    fn report<'out>(
        new_transactions: usize,
        attached_postings: usize,
        unresolved_account_postings: usize,
        unresolved_commodity_postings: usize,
        other_skipped_postings: usize,
        unresolved_accounts: &'out [String],
        unresolved_commodities: &'out [String],
    ) -> Report<'out> {
        Report {
            new_transactions,
            attached_postings,
            unresolved_account_postings,
            unresolved_commodity_postings,
            other_skipped_postings,
            unresolved_accounts,
            unresolved_commodities,
            created_tags: &[],
        }
    }

    /// Owned strings, so the borrowed slices in [`Report`] have something to
    /// point at.
    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[rstest]
    #[case::one(1, "posting", "1 posting")]
    #[case::many(4, "posting", "4 postings")]
    #[case::consonant_y(2, "unregistered commodity", "2 unregistered commodities")]
    #[case::vowel_y(2, "day", "2 days")]
    fn plural_renders_the_noun(#[case] count: usize, #[case] noun: &str, #[case] expected: &str) {
        assert_eq!(plural(count, noun), expected);
    }

    #[test]
    fn report_for_a_clean_run() {
        insta::assert_snapshot!(report(3, 0, 0, 0, 0, &[], &[]).render());
    }

    #[test]
    fn report_for_a_partial_run_with_unknown_accounts() {
        // Some new, some attached, two unresolved accounts accounting for three legs.
        let unresolved = paths(&["Assets:Brokerage", "Expenses:Fun"]);
        insta::assert_snapshot!(report(2, 1, 3, 0, 0, &unresolved, &[]).render());
    }

    #[test]
    fn report_for_a_run_skipping_legs_for_other_reasons() {
        insta::assert_snapshot!(report(1, 0, 0, 0, 2, &[], &[]).render());
    }

    #[test]
    fn report_for_a_run_with_unregistered_commodities() {
        // Registering the commodity and re-running is the recovery path, so the
        // report has to name the codes rather than just count the legs.
        let unregistered = paths(&["DOGE", "XYZ"]);
        insta::assert_snapshot!(report(1, 0, 0, 3, 0, &[], &unregistered).render());
    }

    #[test]
    fn report_attributes_every_cause_separately() {
        let unresolved = paths(&["Expenses:Fun"]);
        let unregistered = paths(&["DOGE"]);
        insta::assert_snapshot!(report(1, 1, 1, 1, 1, &unresolved, &unregistered).render());
    }

    #[test]
    fn the_json_payload_names_every_number_and_its_cause() {
        let unresolved = paths(&["Expenses:Fun", "Expenses:Rent"]);
        let unregistered = paths(&["DOGE"]);
        let payload =
            report(3, 2, 4, 2, 1, &unresolved, &unregistered).to_json("import_batch_stub");

        assert_eq!(
            payload,
            serde_json::json!({
                "batch": "import_batch_stub",
                "new_transactions": 3_usize,
                "attached_postings": 2_usize,
                "skipped_postings": 7_usize,
                "unresolved_account_postings": 4_usize,
                "unresolved_commodity_postings": 2_usize,
                "other_skipped_postings": 1_usize,
                "unresolved_accounts": ["Expenses:Fun", "Expenses:Rent"],
                "unresolved_commodities": ["DOGE"],
                "created_tags": Vec::<String>::new(),
            }),
            "a script reads these keys; renaming one or dropping the cause split is a \
             breaking change"
        );
    }

    /// A [`PlanReport`] exercising every section: unresolved accounts and
    /// commodities, two skip causes, tags it would create, and per-account
    /// totals including one account whose legs net to zero.
    fn sample_report() -> PlanReport {
        PlanReport {
            new_transactions: 2,
            attached_postings: 1,
            skipped_postings: 3,
            unresolved_account_postings: 1,
            unresolved_commodity_postings: 0,
            other_skipped_postings: 2,
            unresolved_accounts: vec![
                "Expenses:Utilities:Gas".to_owned(),
                "Income:Interest".to_owned(),
            ],
            unresolved_commodities: vec!["DOT".to_owned(), "XRP".to_owned()],
            would_create_tags: vec!["groceries".to_owned(), "holiday".to_owned()],
            account_totals: vec![
                (
                    "Assets:BankA:Checking".to_owned(),
                    vec![("AUD".to_owned(), Decimal::new(-1_240_518, 2))],
                ),
                (
                    "Assets:Crypto:ExchangeA".to_owned(),
                    vec![("BTC".to_owned(), Decimal::new(75, 2))],
                ),
                ("Expenses:Groceries".to_owned(), Vec::new()),
            ],
            // AmbiguousResidual is charged 2 (the row's two legs, noted once
            // for the whole row) against UnresolvedAccount's 1, so the skip
            // table's descending-count order is exercised, not just its
            // alphabetical tie-break.
            charged_by_cause: vec![
                (SkipCause::UnresolvedAccount, 1),
                (SkipCause::AmbiguousResidual, 2),
            ],
            diagnostics: vec![
                PlanDiagnostic {
                    location: "bank-a-2024-03.csv:14".to_owned(),
                    cause: SkipCause::UnresolvedAccount,
                    detail: "Expenses:Utilities:Gas".to_owned(),
                },
                PlanDiagnostic {
                    location: "bank-a-2024-07.csv:88".to_owned(),
                    cause: SkipCause::AmbiguousResidual,
                    detail: "2 legs, two or more elided".to_owned(),
                },
            ],
        }
    }

    /// A [`PlanReport`] whose sole skip cause is charged `count` times, each
    /// against a distinct location — enough to exercise the truncation rule
    /// at whatever count the caller wants to probe.
    fn report_with_many_skips(count: usize) -> PlanReport {
        PlanReport {
            new_transactions: 0,
            attached_postings: 0,
            skipped_postings: count,
            unresolved_account_postings: count,
            unresolved_commodity_postings: 0,
            other_skipped_postings: 0,
            unresolved_accounts: vec!["Expenses:Utilities:Gas".to_owned()],
            unresolved_commodities: Vec::new(),
            would_create_tags: Vec::new(),
            account_totals: Vec::new(),
            charged_by_cause: vec![(SkipCause::UnresolvedAccount, count)],
            diagnostics: (0..count)
                .map(|index| PlanDiagnostic {
                    location: format!("bank-a-2024-03.csv:{}", 14_usize.saturating_add(index)),
                    cause: SkipCause::UnresolvedAccount,
                    detail: "Expenses:Utilities:Gas".to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn plan_json_is_unabridged() {
        let report = report_with_many_skips(201);

        let json = report.to_json();

        let diagnostics = json
            .get("diagnostics")
            .and_then(serde_json::Value::as_array)
            .expect("a diagnostics array");
        assert_eq!(
            diagnostics.len(),
            201,
            "the human report truncates; --json must not"
        );
    }

    #[test]
    fn plan_json_carries_no_batch() {
        let report = sample_report();

        let json = report.to_json();

        assert!(json.get("batch_id").is_none(), "a dry run opens no batch");
        assert_eq!(json.get("dry_run"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn plan_json_matches_its_snapshot() {
        let report = sample_report();

        insta::assert_json_snapshot!(report.to_json());
    }

    #[test]
    fn render_plan_leads_with_what_is_missing() {
        let report = sample_report();

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn render_plan_states_the_denominator_when_it_truncates() {
        let report = report_with_many_skips(201);

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        assert!(rendered.contains("3 of 201"), "got:\n{rendered}");
    }

    #[test]
    fn render_plan_orders_skip_causes_by_descending_charged_legs() {
        // sample_report charges ambiguous residual 2 legs against unresolved
        // account's 1, so a comparator flipped to ascending — or one that
        // only ever breaks the alphabetical tie — would still pass a bare
        // snapshot diff. Pin the order explicitly.
        let report = sample_report();

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        let ambiguous = rendered
            .find("\n  ambiguous residual")
            .expect("an ambiguous residual row");
        let unresolved = rendered
            .find("\n  unresolved account ")
            .expect("an unresolved account row");
        assert!(
            ambiguous < unresolved,
            "ambiguous residual charges 2 legs against unresolved account's 1, so it must sort \
             first: got:\n{rendered}"
        );
    }

    #[test]
    fn render_plan_says_nothing_was_written() {
        let report = sample_report();

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        assert!(rendered.contains("nothing was written"), "got:\n{rendered}");
    }

    #[test]
    fn render_plan_marks_a_zero_net_account_as_touched() {
        let report = sample_report();

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        assert!(
            rendered.contains("Expenses:Groceries") && rendered.contains("nets to zero"),
            "an account whose legs net to zero must still show a row, not be omitted: got:\n\
             {rendered}"
        );
    }

    #[test]
    fn render_plan_does_not_truncate_a_group_at_the_cap() {
        let report = report_with_many_skips(3);

        let rendered = render_plan(&report, "bank-a-checking", "csv");

        assert!(
            !rendered.contains('…'),
            "a group at or under the cap must print in full with no ellipsis: got:\n{rendered}"
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

        fn validate(&self, _config: &bc_core::ImportConfig) -> Result<(), bc_core::ImportError> {
            Ok(())
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
            commodities: bc_core::CommodityService::new(pool.clone()),
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
                dry_run: false,
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
                dry_run: false,
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

    #[tokio::test]
    async fn dry_run_with_json_runs_the_json_branch() {
        // Drives `execute_run` itself, not just `PlanReport::to_json` in
        // isolation, so the `ctx.json` check inside the dry-run branch is
        // what is actually exercised — a report built and serialised
        // correctly is no proof the CLI ever reaches that code path.
        let home = tempfile::tempdir().expect("tempdir");
        let (mut ctx, backup_dir) = context_in(home.path(), true).await;
        ctx.json = true;

        super::execute_run(
            super::RunArgs {
                profile: "nightly".to_owned(),
                dry_run: true,
            },
            &ctx,
        )
        .await
        .expect("dry run with --json succeeds");

        assert_eq!(
            pre_import_snapshots(&backup_dir),
            0,
            "a dry run takes no snapshot, whether or not --json is set"
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

        let header = rendered.lines().next().expect("a header row");
        for column in ["BATCH", "IMPORTER", "PROFILE", "STARTED", "OUTCOME"] {
            assert!(header.contains(column), "the {column} column is named");
        }
        assert!(rendered.contains("incomplete"), "an open run says so");
        assert!(rendered.contains("discarded"), "a discarded run says so");

        // Scoped to the csv row rather than searched for across the whole
        // table: the batch IDs and timestamps are random per run, so a bare
        // `contains("12")` passes on a stray digit pair and stops discriminating.
        let done_line = rendered
            .lines()
            .find(|line| line.contains("csv"))
            .expect("the completed run has a row");
        assert!(
            done_line.contains("12 new, 3 attached"),
            "a completed run reports what it did: {done_line}"
        );
        let done_started_at = listed
            .iter()
            .find(|batch| batch.id == done)
            .expect("the completed run is listed")
            .started_at
            .to_string();
        assert!(
            done_line.contains(&done_started_at),
            "each row carries its run's start time: {done_line}"
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
    #[expect(clippy::indexing_slicing, reason = "test code with known structure")]
    async fn list_to_json_covers_each_outcome_state(pool: sqlx::SqlitePool) {
        // Mirrors `list_renders_each_outcome_state`, but on the machine
        // surface: `counts.map(...)` fans a single `Option<Counts>` out to
        // six independent JSON fields, and a typo'd key or a wrong fanout
        // would still leave the human-readable table looking fine.
        let batches = bc_core::ImportBatchService::new(pool.clone());

        let done = batches.open(None, "csv").await.expect("open");
        let mut done_counts = bc_core::ImportBatchCounts::default();
        done_counts.new_transactions = 12;
        done_counts.attached_postings = 3;
        done_counts.unresolved_account_postings = 1;
        done_counts.unresolved_commodity_postings = 4;
        done_counts.other_skipped_postings = 2;
        batches.close(&done, done_counts).await.expect("close");

        let thrown_away = batches.open(None, "ofx").await.expect("open");
        batches
            .close(&thrown_away, bc_core::ImportBatchCounts::default())
            .await
            .expect("close");
        batches.discard(&thrown_away).await.expect("discard");

        let open = batches.open(None, "ledger").await.expect("open");

        let listed = batches.list().await.expect("list");
        let payload = super::list_to_json(&listed);
        let entries = payload["batches"].as_array().expect("batches array");
        let find = |id: &bc_models::ImportBatchId| {
            entries
                .iter()
                .find(|entry| entry["id"] == id.to_string())
                .unwrap_or_else(|| panic!("no entry for batch {id}"))
        };

        let done_entry = find(&done);
        pretty_assertions::assert_eq!(done_entry["new_transactions"], serde_json::json!(12_usize));
        pretty_assertions::assert_eq!(done_entry["attached_postings"], serde_json::json!(3_usize));
        pretty_assertions::assert_eq!(done_entry["skipped_postings"], serde_json::json!(7_usize));
        pretty_assertions::assert_eq!(
            done_entry["unresolved_account_postings"],
            serde_json::json!(1_usize)
        );
        pretty_assertions::assert_eq!(
            done_entry["unresolved_commodity_postings"],
            serde_json::json!(4_usize)
        );
        pretty_assertions::assert_eq!(
            done_entry["other_skipped_postings"],
            serde_json::json!(2_usize)
        );
        pretty_assertions::assert_eq!(done_entry["discarded_at"], serde_json::Value::Null);

        let discarded_entry = find(&thrown_away);
        pretty_assertions::assert_ne!(
            discarded_entry["discarded_at"],
            serde_json::Value::Null,
            "a discarded batch records when"
        );
        pretty_assertions::assert_eq!(
            discarded_entry["new_transactions"],
            serde_json::json!(0_usize),
            "a completed-but-discarded batch still reports its recorded counts"
        );

        let open_entry = find(&open);
        pretty_assertions::assert_eq!(
            open_entry["new_transactions"],
            serde_json::Value::Null,
            "an incomplete batch reports null counts, not zeros"
        );
        pretty_assertions::assert_eq!(
            open_entry["skipped_postings"],
            serde_json::Value::Null,
            "the derived skipped total must follow the same None, not become 0"
        );
        pretty_assertions::assert_eq!(open_entry["finished_at"], serde_json::Value::Null);
        pretty_assertions::assert_eq!(open_entry["discarded_at"], serde_json::Value::Null);
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
        assert!(!rendered.contains("other reference"));
    }

    /// Creates a top-level asset account and returns its ID.
    async fn account(pool: &SqlitePool, name: &str) -> AccountId {
        bc_core::AccountService::new(pool.clone())
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    /// Inserts an unreconciled transaction on `account_id` holding
    /// `posting_count` postings, each carrying `amount` AUD.
    ///
    /// Every posting shares the one amount because a reference has to record
    /// what its posting holds. The amount varies *between* transactions instead:
    /// it is part of the fingerprint, which does not otherwise differ from one
    /// scenario to the next, so two scenarios sharing it would contend for the
    /// same dedup slot.
    async fn transaction_with_postings(
        pool: &SqlitePool,
        account_id: &AccountId,
        posting_count: usize,
        amount: i64,
    ) -> (TransactionId, Vec<PostingId>) {
        let posting_ids: Vec<PostingId> = core::iter::repeat_with(PostingId::new)
            .take(posting_count)
            .collect();
        let tx = bc_models::Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("ACME")
            .postings(
                posting_ids
                    .iter()
                    .map(|posting_id| {
                        bc_models::Posting::builder()
                            .id(posting_id.clone())
                            .account_id(account_id.clone())
                            .amount(Amount::new(
                                Decimal::from(amount),
                                CommodityCode::new("AUD"),
                            ))
                            .build()
                    })
                    .collect(),
            )
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        bc_core::TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create tx");
        (id, posting_ids)
    }

    /// What a test-only [`SourceRef`] attachment needs beyond the transaction
    /// and posting it names — bundled so `attach_at` stays under the
    /// argument-count lint rather than earning a suppression.
    struct AttachSpec {
        /// Whether the attaching batch created the posting.
        owns_posting: bool,
        /// Disambiguates same-day, same-amount rows on one account.
        occurrence: u32,
        /// Must match the named posting's own amount, as a real import's
        /// reference would.
        amount: i64,
    }

    /// Attaches a reference owned by `batch`, pointing at `posting_id`, per
    /// `spec` — a distinct `occurrence` lets several references share an
    /// account and fingerprint without colliding on the dedup slot.
    async fn attach_at(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        transaction_id: &TransactionId,
        posting_id: &PostingId,
        account_id: &AccountId,
        spec: &AttachSpec,
    ) -> bc_models::SourceRefId {
        let id = bc_models::SourceRefId::new();
        let source = SourceRef::builder()
            .id(id.clone())
            .transaction_id(transaction_id.clone())
            .posting_id(Some(posting_id.clone()))
            .account_id(account_id.clone())
            .date(date(2026, 1, 15))
            .narration("ACME")
            .amount(Some(Amount::new(
                Decimal::from(spec.amount),
                CommodityCode::new("AUD"),
            )))
            .reference(None)
            .occurrence(spec.occurrence)
            .import_batch_id(Some(batch.clone()))
            .owns_posting(spec.owns_posting)
            .created_at(Timestamp::now())
            .build();
        bc_core::SourceService::new(pool.clone())
            .attach(&source)
            .await
            .expect("attach");
        id
    }

    /// Tombstones a reference the way an edit does: clears its posting link,
    /// leaving the reference itself as history.
    async fn tombstone(pool: &SqlitePool, reference: &bc_models::SourceRefId) {
        sqlx::query("UPDATE transaction_sources SET posting_id = NULL WHERE id = ?")
            .bind(reference.to_string())
            .execute(pool)
            .await
            .expect("tombstone");
    }

    /// Deletes a posting row outright, the way a fully-applied leg deletion
    /// does once its reference has already been tombstoned.
    async fn delete_posting(pool: &SqlitePool, posting_id: &PostingId) {
        sqlx::query("DELETE FROM postings WHERE id = ?")
            .bind(posting_id.to_string())
            .execute(pool)
            .await
            .expect("delete posting");
    }

    /// Recategorises a posting, the way a user's edit does, so its reference no
    /// longer agrees with what it recorded at attach time.
    async fn recategorise(pool: &SqlitePool, posting_id: &PostingId, elsewhere: &AccountId) {
        sqlx::query("UPDATE postings SET account_id = ? WHERE id = ?")
            .bind(elsewhere.to_string())
            .bind(posting_id.to_string())
            .execute(pool)
            .await
            .expect("recategorise");
    }

    /// Marks a transaction reconciled, the way confirming it against a
    /// statement does.
    async fn reconcile(pool: &SqlitePool, transaction_id: &TransactionId) {
        set_reconciliation(pool, transaction_id, "reconciled").await;
    }

    /// Marks a transaction flagged, the way sending it back for review does.
    async fn flag(pool: &SqlitePool, transaction_id: &TransactionId) {
        set_reconciliation(pool, transaction_id, "flagged").await;
    }

    /// Puts a transaction into the given stored reconciliation state.
    async fn set_reconciliation(pool: &SqlitePool, transaction_id: &TransactionId, state: &str) {
        sqlx::query("UPDATE transactions SET reconciliation = ? WHERE id = ?")
            .bind(state)
            .bind(transaction_id.to_string())
            .execute(pool)
            .await
            .expect("set reconciliation");
    }

    /// Plain: `count` owned postings on one transaction, untouched — a
    /// baseline contribution to `removed_postings`/`removed_transactions`
    /// with no side effect on any other count.
    async fn plain_scenario(pool: &SqlitePool, batch: &ImportBatchId, acct: &AccountId) {
        let (tx, postings) = transaction_with_postings(pool, acct, 2, 50).await;
        for (index, posting_id) in postings.iter().enumerate() {
            let spec = AttachSpec {
                owns_posting: true,
                occurrence: u32::try_from(index).expect("small index"),
                amount: 50,
            };
            attach_at(pool, batch, &tx, posting_id, acct, &spec).await;
        }
    }

    /// Edited: one owned posting, recategorised before discard — contributes
    /// `edited_postings` alongside `removed_postings`/`removed_transactions`.
    async fn edited_scenario(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        acct: &AccountId,
        elsewhere: &AccountId,
    ) {
        let (tx, postings) = transaction_with_postings(pool, acct, 1, 51).await;
        let posting = postings.first().expect("one posting");
        let spec = AttachSpec {
            owns_posting: true,
            occurrence: 0,
            amount: 51,
        };
        attach_at(pool, batch, &tx, posting, acct, &spec).await;
        recategorise(pool, posting, elsewhere).await;
    }

    /// Reconciled: two owned postings on one transaction, reconciled before
    /// discard — contributes `reconciled_postings` alongside
    /// `removed_postings`/`removed_transactions`.
    async fn reconciled_scenario(pool: &SqlitePool, batch: &ImportBatchId, acct: &AccountId) {
        let (tx, postings) = transaction_with_postings(pool, acct, 2, 52).await;
        for (index, posting_id) in postings.iter().enumerate() {
            let spec = AttachSpec {
                owns_posting: true,
                occurrence: u32::try_from(index).expect("small index"),
                amount: 52,
            };
            attach_at(pool, batch, &tx, posting_id, acct, &spec).await;
        }
        reconcile(pool, &tx).await;
    }

    /// Adopted: three references pointing at postings the batch did not
    /// create, each on its own transaction (and its own amount) so none
    /// collide on a slot — contributes only `detached_adopted`.
    async fn adopted_scenario(pool: &SqlitePool, batch: &ImportBatchId, acct: &AccountId) {
        for amount in [53_i64, 54, 55] {
            let (tx, postings) = transaction_with_postings(pool, acct, 1, amount).await;
            let posting = postings.first().expect("one posting");
            let spec = AttachSpec {
                owns_posting: false,
                occurrence: 0,
                amount,
            };
            attach_at(pool, batch, &tx, posting, acct, &spec).await;
        }
    }

    /// Flagged: four owned postings on one transaction, flagged for review
    /// before discard — contributes `flagged_postings` alongside
    /// `removed_postings`/`removed_transactions`, and nothing to
    /// `reconciled_postings`.
    async fn flagged_scenario(pool: &SqlitePool, batch: &ImportBatchId, acct: &AccountId) {
        let (tx, postings) = transaction_with_postings(pool, acct, 4, 66).await;
        for (index, posting_id) in postings.iter().enumerate() {
            let spec = AttachSpec {
                owns_posting: true,
                occurrence: u32::try_from(index).expect("small index"),
                amount: 66,
            };
            attach_at(pool, batch, &tx, posting_id, acct, &spec).await;
        }
        flag(pool, &tx).await;
    }

    /// Surviving collateral: one transaction holding eight postings the batch
    /// owns, each also carrying a reference from `other_batch` that merely
    /// adopted it, plus a ninth posting nothing references.
    ///
    /// The ninth keeps the transaction alive through the sweep, so the other
    /// batch's eight references are left naming deleted postings rather than
    /// going away with their transaction — contributing
    /// `other_batch_references_tombstoned` (not `..._removed`) alongside
    /// `removed_postings`, and nothing to `removed_transactions`.
    async fn surviving_collateral_scenario(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        other_batch: &ImportBatchId,
        acct: &AccountId,
    ) {
        let (tx, postings) = transaction_with_postings(pool, acct, 9, 67).await;
        let (_survivor, owned) = postings.split_first().expect("at least one posting");
        for (index, posting_id) in owned.iter().enumerate() {
            let slot = u32::try_from(index).expect("small index");
            let mine = AttachSpec {
                owns_posting: true,
                occurrence: slot.checked_mul(2).expect("small index"),
                amount: 67,
            };
            attach_at(pool, batch, &tx, posting_id, acct, &mine).await;
            let theirs = AttachSpec {
                owns_posting: false,
                occurrence: slot
                    .checked_mul(2)
                    .and_then(|even| even.checked_add(1))
                    .expect("small index"),
                amount: 67,
            };
            attach_at(pool, other_batch, &tx, posting_id, acct, &theirs).await;
        }
    }

    /// Tombstoned: nine references the batch owns, already orphaned before
    /// discard — the posting and its transaction are otherwise untouched, so
    /// this contributes only `freed_tombstones`.
    async fn tombstoned_scenario(pool: &SqlitePool, batch: &ImportBatchId, acct: &AccountId) {
        for amount in [56_i64, 57, 58, 59, 60, 62, 63, 64, 65] {
            let (tx, postings) = transaction_with_postings(pool, acct, 1, amount).await;
            let posting = postings.first().expect("one posting");
            let spec = AttachSpec {
                owns_posting: true,
                occurrence: 0,
                amount,
            };
            let reference = attach_at(pool, batch, &tx, posting, acct, &spec).await;
            tombstone(pool, &reference).await;
        }
    }

    /// Collateral: one transaction holding the discarded batch's own posting
    /// plus seven already-orphaned references belonging to `other_batch`.
    /// Once the discarded batch's posting is removed the transaction empties
    /// and is deleted, sweeping the other batch's leftover tombstones with
    /// it — contributing `other_batch_references_removed` alongside
    /// `removed_postings`/`removed_transactions`.
    async fn collateral_scenario(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        other_batch: &ImportBatchId,
        acct: &AccountId,
    ) {
        let (tx, postings) = transaction_with_postings(pool, acct, 8, 61).await;
        let (mine, others) = postings.split_first().expect("at least one posting");
        let spec = AttachSpec {
            owns_posting: true,
            occurrence: 0,
            amount: 61,
        };
        attach_at(pool, batch, &tx, mine, acct, &spec).await;
        for (index, other_posting) in others.iter().enumerate() {
            let occurrence = u32::try_from(index)
                .expect("small index")
                .checked_add(1)
                .expect("small index");
            let other_spec = AttachSpec {
                owns_posting: true,
                occurrence,
                amount: 61,
            };
            let reference =
                attach_at(pool, other_batch, &tx, other_posting, acct, &other_spec).await;
            tombstone(pool, &reference).await;
            delete_posting(pool, other_posting).await;
        }
    }

    /// Builds a [`bc_core::DiscardOutcome`] (via a real discard) with every
    /// optional count non-zero and pairwise distinct, so a transposed pair in
    /// the report would be visible rather than coincidentally matching.
    ///
    /// The nine counts, by construction:
    /// - `edited_postings` = 1 (one owned posting, recategorised before discard)
    /// - `reconciled_postings` = 2 (two owned postings in one reconciled transaction)
    /// - `detached_adopted` = 3 (three adopted-only references)
    /// - `flagged_postings` = 4 (four owned postings in one flagged transaction)
    /// - `removed_transactions` = 5 (plain, edited, reconciled, flagged, collateral)
    /// - `other_batch_references_removed` = 7 (seven orphaned references from
    ///   another batch, swept when the collateral transaction empties)
    /// - `other_batch_references_tombstoned` = 8 (eight adopting references
    ///   from another batch, left naming deleted postings on a transaction
    ///   that survived)
    /// - `freed_tombstones` = 9 (nine references tombstoned before discard)
    /// - `removed_postings` = 18 (2 plain + 1 edited + 2 reconciled + 4 flagged
    ///   + 1 collateral + 8 surviving-collateral)
    async fn discard_everything_at_once(
        pool: &SqlitePool,
    ) -> (bc_core::DiscardOutcome, bc_core::ImportBatch) {
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let other_batch = batches.open(None, "ofx").await.expect("open other");
        let acct = account(pool, "Checking").await;
        let elsewhere = account(pool, "Savings").await;

        plain_scenario(pool, &batch, &acct).await;
        edited_scenario(pool, &batch, &acct, &elsewhere).await;
        reconciled_scenario(pool, &batch, &acct).await;
        flagged_scenario(pool, &batch, &acct).await;
        adopted_scenario(pool, &batch, &acct).await;
        tombstoned_scenario(pool, &batch, &acct).await;
        collateral_scenario(pool, &batch, &other_batch, &acct).await;
        surviving_collateral_scenario(pool, &batch, &other_batch, &acct).await;

        let record = batches.find_by_id(&batch).await.expect("find");
        let outcome = batches.discard(&batch).await.expect("discard");
        (outcome, record)
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn every_optional_discard_line_fires_with_a_distinct_count(pool: SqlitePool) {
        let (outcome, record) = discard_everything_at_once(&pool).await;

        pretty_assertions::assert_eq!(outcome.removed_postings, 18);
        pretty_assertions::assert_eq!(outcome.removed_transactions, 5);
        pretty_assertions::assert_eq!(outcome.edited_postings, 1);
        pretty_assertions::assert_eq!(outcome.reconciled_postings, 2);
        pretty_assertions::assert_eq!(outcome.flagged_postings, 4);
        pretty_assertions::assert_eq!(outcome.detached_adopted, 3);
        pretty_assertions::assert_eq!(outcome.freed_tombstones, 9);
        pretty_assertions::assert_eq!(outcome.other_batch_references_removed, 7);
        pretty_assertions::assert_eq!(outcome.other_batch_references_tombstoned, 8);

        // The header names the batch ID and start time, both fresh per test
        // run; redact them to fixed placeholders so the snapshot is stable.
        let stabilised = super::render_discard(&outcome, &record)
            .replace(&outcome.batch_id.to_string(), "BATCH_ID")
            .replace(&record.started_at.to_string(), "STARTED_AT");
        insta::assert_snapshot!(stabilised);

        let payload = super::discard_to_json(&outcome, &record);
        pretty_assertions::assert_eq!(
            payload,
            serde_json::json!({
                "batch": outcome.batch_id.to_string(),
                "importer": "csv",
                "started_at": record.started_at.to_string(),
                "removed_postings": 18_usize,
                "removed_transactions": 5_usize,
                "detached_adopted": 3_usize,
                "freed_tombstones": 9_usize,
                "other_batch_references_removed": 7_usize,
                "other_batch_references_tombstoned": 8_usize,
                "edited_postings": 1_usize,
                "reconciled_postings": 2_usize,
                "flagged_postings": 4_usize,
            }),
            "the JSON surface must derive from the same outcome as the human report, \
             not a separately maintained count"
        );
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn a_mixed_discard_gates_each_line_on_its_own_count(pool: SqlitePool) {
        // Only `reconciled_postings` is non-zero; every other optional count
        // is zero. A line gated on the wrong field (say, the reconciled line
        // keyed off `edited_postings`) would pass both the all-zero and
        // all-non-zero cases above but fail here.
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;

        reconciled_scenario(&pool, &batch, &acct).await;

        let record = batches.find_by_id(&batch).await.expect("find");
        let outcome = batches.discard(&batch).await.expect("discard");

        pretty_assertions::assert_eq!(outcome.edited_postings, 0);
        pretty_assertions::assert_eq!(outcome.reconciled_postings, 2);
        pretty_assertions::assert_eq!(outcome.detached_adopted, 0);
        pretty_assertions::assert_eq!(outcome.freed_tombstones, 0);
        pretty_assertions::assert_eq!(outcome.other_batch_references_removed, 0);

        let rendered = super::render_discard(&outcome, &record);
        assert!(
            rendered.contains("reconciled"),
            "the one non-zero optional line must fire"
        );
        assert!(!rendered.contains("edited"));
        assert!(!rendered.contains("adopted"));
        assert!(!rendered.contains("freed"));
        assert!(!rendered.contains("other reference"));
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn a_single_collateral_reference_reads_grammatically(pool: SqlitePool) {
        // Drive a real discard with exactly one other-batch reference riding
        // on the swept transaction. An inverted comparison in the
        // pronoun/suffix ternaries would flip both branches at once and still
        // read as grammatical nonsense, so this has to check the actual words,
        // not just that a line fired.
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let other_batch = batches.open(None, "ofx").await.expect("open other");
        let acct = account(&pool, "Checking").await;

        let (tx, postings) = transaction_with_postings(&pool, &acct, 2, 62).await;
        let (mine, others) = postings.split_first().expect("two postings");
        let other_posting = others.first().expect("one other posting");
        let spec = AttachSpec {
            owns_posting: true,
            occurrence: 0,
            amount: 62,
        };
        attach_at(&pool, &batch, &tx, mine, &acct, &spec).await;
        let other_spec = AttachSpec {
            owns_posting: true,
            occurrence: 1,
            amount: 62,
        };
        let reference =
            attach_at(&pool, &other_batch, &tx, other_posting, &acct, &other_spec).await;
        tombstone(&pool, &reference).await;
        delete_posting(&pool, other_posting).await;

        let record = batches.find_by_id(&batch).await.expect("find");
        let outcome = batches.discard(&batch).await.expect("discard");

        pretty_assertions::assert_eq!(outcome.other_batch_references_removed, 1);

        let rendered = super::render_discard(&outcome, &record);
        assert!(
            rendered.contains("1 other reference removed with its transaction"),
            "a singular count must read 'its transaction', not 'their transactions': {rendered}"
        );
        assert!(
            !rendered.contains("their"),
            "the plural pronoun must not appear for a singular count: {rendered}"
        );
    }

    #[sqlx::test(migrations = "../bc-core/migrations")]
    async fn a_flagged_transaction_is_not_reported_as_reconciled(pool: SqlitePool) {
        // `flagged` and `reconciled` are both "not unreconciled", so a
        // predicate testing for that would fold them together and tell the
        // user their flagged rows had been confirmed against a statement.
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let batch = batches.open(None, "csv").await.expect("open");
        let acct = account(&pool, "Checking").await;

        flagged_scenario(&pool, &batch, &acct).await;

        let record = batches.find_by_id(&batch).await.expect("find");
        let outcome = batches.discard(&batch).await.expect("discard");

        pretty_assertions::assert_eq!(outcome.flagged_postings, 4);
        pretty_assertions::assert_eq!(
            outcome.reconciled_postings,
            0,
            "a flagged transaction was never reconciled"
        );

        let rendered = super::render_discard(&outcome, &record);
        assert!(rendered.contains("4 of them sat in flagged transactions"));
        assert!(
            !rendered.contains("reconciled"),
            "nothing here was reconciled: {rendered}"
        );
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

    #[tokio::test]
    async fn a_repeat_discard_does_not_snapshot() {
        // The already-discarded guard lives in core's `discard`, but it must
        // be checked here too, before the snapshot: otherwise a repeat
        // discard pays for a full database copy and then errors anyway.
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir, batch_id) = context_with_a_batch(home.path(), true).await;

        super::execute_discard(
            super::DiscardArgs {
                batch: batch_id.to_string(),
            },
            &ctx,
        )
        .await
        .expect("first discard");
        assert_eq!(pre_discard_snapshots(&backup_dir), 1);

        let result = super::execute_discard(
            super::DiscardArgs {
                batch: batch_id.to_string(),
            },
            &ctx,
        )
        .await;

        assert!(
            matches!(
                result,
                Err(crate::error::CliError::Core(
                    bc_core::BcError::InvalidInput(_)
                ))
            ),
            "a repeat discard must still surface core's own error"
        );
        assert_eq!(
            pre_discard_snapshots(&backup_dir),
            1,
            "the repeat must not write a second, wasted snapshot"
        );
    }

    #[tokio::test]
    async fn an_unparsable_batch_id_is_an_argument_error() {
        // Parsing happens before any database work, so a typo does not
        // snapshot. Driving the whole command is what pins that ordering:
        // parsing the ID in isolation would pass with the parse moved below
        // the snapshot.
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir, _batch_id) = context_with_a_batch(home.path(), true).await;

        let result = super::execute_discard(
            super::DiscardArgs {
                batch: "not-a-batch-id".to_owned(),
            },
            &ctx,
        )
        .await;

        assert!(
            matches!(result, Err(crate::error::CliError::Arg(_))),
            "a malformed ID is the user's argument error, not a core failure"
        );
        assert_eq!(
            pre_discard_snapshots(&backup_dir),
            0,
            "a typo must not cost a full database copy"
        );
    }

    #[tokio::test]
    async fn an_unknown_batch_id_does_not_snapshot() {
        // Same ordering guarantee one step later: the ID parses, but names no
        // batch. Resolution has to happen before the snapshot too.
        let home = tempfile::tempdir().expect("tempdir");
        let (ctx, backup_dir, _batch_id) = context_with_a_batch(home.path(), true).await;

        let result = super::execute_discard(
            super::DiscardArgs {
                batch: bc_models::ImportBatchId::new().to_string(),
            },
            &ctx,
        )
        .await;

        assert!(
            matches!(
                result,
                Err(crate::error::CliError::Core(bc_core::BcError::NotFound(_)))
            ),
            "an unknown batch must be reported as not found"
        );
        assert_eq!(
            pre_discard_snapshots(&backup_dir),
            0,
            "an unknown batch must not cost a full database copy"
        );
    }
}
