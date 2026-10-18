//! `transaction apply`: applies an edited working export as one import batch.
//!
//! The file is parsed by the beancount plugin, its handles resolved through
//! the sidecar written beside it, and the differences written as a batch that
//! `import discard` takes back.

use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use bc_core::ImportConfig;
use bc_core::ImportOutput;
use bc_core::Importer as _;
use bc_core::roundtrip::Action;
use bc_core::roundtrip::ApplyOutcome;
use bc_core::roundtrip::ApplyPlan;
use bc_core::roundtrip::Sidecar;
use bc_core::roundtrip::SkipReason;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// The plugin that reads a working export back.
const FORMAT: &str = "beancount";

// MARK: Arguments

/// Arguments for `transaction apply`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The edited working export, with its `.bc.json` sidecar beside it.
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Report what apply would do, without writing anything or taking a
    /// backup.
    #[arg(long)]
    pub dry_run: bool,
}

// MARK: Execution

/// Executes `transaction apply`.
///
/// Prints one line per transaction in the file, then the warnings, then the
/// totals. A real run takes a pre-import backup first, when the policy asks
/// for one and the plan writes anything, and ends with the batch to discard.
/// A refused transaction leaves the exit code at 0.
///
/// # Arguments
///
/// * `args` - The file and whether to write.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when the sidecar is missing or the beancount
/// plugin is not loaded, and [`CliError::Preflight`] after printing every
/// problem when the file is refused whole. Returns [`CliError::Core`] on a
/// database failure; a run that stops part-way logs the batch it left open.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    let sidecar_path = Sidecar::path_for(&args.file);
    if !sidecar_path.is_file() {
        return Err(CliError::Arg(format!(
            "no sidecar at {}; export with --level working first",
            sidecar_path.display()
        )));
    }
    let sidecar = Sidecar::read(&sidecar_path)?;
    let output = parse(ctx, &args.file)?;
    let plan = match ctx.round_trip.plan_file(&output, &sidecar).await? {
        Ok(plan) => plan,
        Err(refused) => return Err(refuse(&refused.problems)?),
    };
    let mut lines = lines(&output, &plan);

    if args.dry_run {
        let totals = Totals::planned(&plan);
        return print(&lines, &plan.warnings, &[totals.dry_run()]);
    }

    if ctx.auto_pre_import && writes(&plan) {
        let taken = ctx
            .backup
            .backup(bc_core::BackupKind::PreImport, None)
            .await?;
        tracing::info!(path = %taken.path.display(), "pre-import snapshot taken");
    }
    let planned_creates = lines
        .iter()
        .filter(|line| line.verb == Verb::Create)
        .count();
    let planned_refusals = refusals(&plan);
    let outcome = ctx.round_trip.apply(plan).await?;
    let unattributed = mark_write_refusals(&mut lines, planned_refusals, &outcome);

    let mut tail = Vec::new();
    let created_or_refused = outcome.created.saturating_add(unattributed.len());
    if created_or_refused == 0 {
        // A rerun: an earlier apply stored every new transaction.
        for line in lines.iter_mut().filter(|line| line.verb == Verb::Create) {
            line.verb = Verb::Skip;
            line.reason = Some(skip_reason(SkipReason::AlreadyApplied));
        }
    } else if created_or_refused < planned_creates {
        // The outcome does not say which new transactions were stored before.
        tail.push(format!(
            "{} already applied; skipped",
            plural(
                planned_creates.saturating_sub(created_or_refused),
                "new transaction"
            )
        ));
    }
    tail.push(Totals::applied(&outcome));
    if let Some(ref id) = outcome.batch_id {
        tail.push(format!("batch {id}"));
        tail.push(format!("undo with: borrow-checker import discard {id}"));
    } else {
        tail.push("nothing to write; no batch opened".to_owned());
    }
    lines.extend(unattributed);
    print(&lines, &outcome.warnings, &tail)
}

/// Parses `file` with the beancount plugin, rooted at the file's directory.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when the plugin is not loaded or the path names
/// no file, and [`CliError::Preflight`] when the plugin rejects the file
/// whole.
fn parse(ctx: &AppContext, file: &Path) -> CliResult<ImportOutput> {
    // The registry hands out the plugin under its exporter name; the same
    // plugin reads the file back.
    let plugin = ctx
        .plugin_registry
        .exporter(FORMAT)
        .ok_or_else(|| CliError::Arg(format!("the {FORMAT} plugin is not loaded")))?;
    let absolute = std::path::absolute(file)?;
    let (Some(dir), Some(name)) = (
        absolute.parent(),
        absolute.file_name().and_then(|name| name.to_str()),
    ) else {
        return Err(CliError::Arg(format!(
            "{} does not name a file with a UTF-8 name",
            file.display()
        )));
    };
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": name }));
    plugin
        .with_documents_root(dir.to_path_buf())
        .import(&config)
        .or_else(|error| Err(refuse(&[error.to_string()])?))
}

/// Prints each problem refusing the file, and returns the error to exit with.
///
/// # Errors
///
/// Returns [`CliError::Io`] when stdout cannot be written.
fn refuse(problems: &[String]) -> CliResult<CliError> {
    let mut stdout = std::io::stdout().lock();
    for problem in problems {
        writeln!(stdout, "{problem}")?;
    }
    Ok(CliError::Preflight(format!(
        "{} refused the file; nothing was written",
        plural(problems.len(), "problem")
    )))
}

/// Whether `plan` writes anything.
fn writes(plan: &ApplyPlan) -> bool {
    plan.actions.iter().any(|action| {
        matches!(
            *action,
            Action::Edit { .. } | Action::Create { .. } | Action::Delete { .. }
        )
    })
}

/// The refusals `plan` itself makes, as the outcome records them.
fn refusals(plan: &ApplyPlan) -> Vec<(Option<String>, String)> {
    plan.actions
        .iter()
        .filter_map(|action| match *action {
            Action::Refuse {
                ref handle,
                ref reason,
            } => Some((handle.clone(), reason.clone())),
            Action::Skip { .. }
            | Action::Edit { .. }
            | Action::Create { .. }
            | Action::Delete { .. }
            | _ => None,
        })
        .collect()
}

/// Marks each transaction a write refused on its line, and returns a line for
/// each such refusal that names no handle.
///
/// The outcome lists the plan's own refusals as well as the write's; the
/// plan's are already on their lines.
fn mark_write_refusals(
    lines: &mut [Line],
    mut planned: Vec<(Option<String>, String)>,
    outcome: &ApplyOutcome,
) -> Vec<Line> {
    let mut unattributed = Vec::new();
    for refusal in &outcome.refused {
        if let Some(at) = planned.iter().position(|seen| seen == refusal) {
            planned.swap_remove(at);
            continue;
        }
        let (ref handle, ref reason) = *refusal;
        let marked = handle.as_ref().and_then(|wanted| {
            lines
                .iter_mut()
                .find(|line| line.handle.as_ref() == Some(wanted) && line.verb != Verb::Refused)
        });
        if let Some(line) = marked {
            line.verb = Verb::Refused;
            line.reason = Some(reason.clone());
        } else {
            unattributed.push(Line {
                verb: Verb::Refused,
                handle: handle.clone(),
                heading: String::new(),
                reason: Some(reason.clone()),
            });
        }
    }
    unattributed
}

/// Writes the report: the lines, the warnings, then `tail`.
///
/// # Errors
///
/// Returns [`CliError::Io`] when stdout cannot be written.
fn print(lines: &[Line], warnings: &[String], tail: &[String]) -> CliResult<()> {
    let mut stdout = std::io::stdout().lock();
    for line in lines {
        writeln!(stdout, "{line}")?;
    }
    for warning in warnings {
        writeln!(stdout, "warning: {warning}")?;
    }
    for line in tail {
        writeln!(stdout, "{line}")?;
    }
    Ok(())
}

// MARK: Report

/// What apply does with one transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    /// Replaces the stored transaction.
    Edit,
    /// Stores a new transaction.
    Create,
    /// Deletes the stored transaction.
    Delete,
    /// Writes nothing.
    Skip,
    /// Leaves the transaction alone and reports why.
    Refused,
}

impl Verb {
    /// The word the report prints.
    const fn word(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Skip => "skip",
            Self::Refused => "refused",
        }
    }
}

/// One report line: `verb  handle  date description  reason`.
#[derive(Debug, Clone)]
struct Line {
    /// What apply does.
    verb: Verb,
    /// The transaction's handle; a new transaction has none.
    handle: Option<String>,
    /// The date and description, as the file states them.
    heading: String,
    /// Why, for a skip or a refusal.
    reason: Option<String>,
}

impl core::fmt::Display for Line {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:<7}  {}",
            self.verb.word(),
            self.handle.as_deref().unwrap_or("-")
        )?;
        if !self.heading.is_empty() {
            write!(f, "  {}", self.heading)?;
        }
        if let Some(ref reason) = self.reason {
            write!(f, "  {reason}")?;
        }
        Ok(())
    }
}

/// One line per action in `plan`, headed by the file transaction it is for.
///
/// The plan holds one action per file transaction, in file order.
fn lines(output: &ImportOutput, plan: &ApplyPlan) -> Vec<Line> {
    let headings = output
        .directives
        .iter()
        .filter_map(|directive| match *directive {
            bc_core::Directive::Transaction(ref raw) => {
                Some(format!("{} {}", raw.date, raw.description))
            }
            bc_core::Directive::Open(_) | bc_core::Directive::Close(_) | _ => None,
        });
    plan.actions
        .iter()
        .zip(headings)
        .map(|(action, heading)| {
            let (verb, handle, reason) = match *action {
                Action::Skip { ref handle, reason } => {
                    (Verb::Skip, Some(handle.clone()), Some(skip_reason(reason)))
                }
                Action::Edit { ref handle, .. } => (Verb::Edit, Some(handle.clone()), None),
                Action::Delete { ref handle, .. } => (Verb::Delete, Some(handle.clone()), None),
                Action::Refuse {
                    ref handle,
                    ref reason,
                } => (Verb::Refused, handle.clone(), Some(reason.clone())),
                Action::Create { .. } | _ => (Verb::Create, None, None),
            };
            Line {
                verb,
                handle,
                heading,
                reason,
            }
        })
        .collect()
}

/// Why a skip writes nothing, as the report prints it.
fn skip_reason(reason: SkipReason) -> String {
    match reason {
        SkipReason::AlreadyApplied => "already applied",
        SkipReason::Unchanged | _ => "unchanged",
    }
    .to_owned()
}

/// Counts of each verb.
#[derive(Debug, Default)]
struct Totals {
    /// Edits.
    edit: usize,
    /// Creates.
    create: usize,
    /// Deletes.
    delete: usize,
    /// Skips.
    skip: usize,
    /// Refusals.
    refused: usize,
}

impl Totals {
    /// Counts what `plan` would do.
    fn planned(plan: &ApplyPlan) -> Self {
        let mut totals = Self::default();
        for action in &plan.actions {
            let count = match *action {
                Action::Edit { .. } => &mut totals.edit,
                Action::Delete { .. } => &mut totals.delete,
                Action::Skip { .. } => &mut totals.skip,
                Action::Refuse { .. } => &mut totals.refused,
                Action::Create { .. } | _ => &mut totals.create,
            };
            *count = count.saturating_add(1);
        }
        totals
    }

    /// The dry run's totals line.
    fn dry_run(&self) -> String {
        format!(
            "dry run: {} edit, {} create, {} delete, {} skip, {} refused; nothing was written",
            self.edit, self.create, self.delete, self.skip, self.refused
        )
    }

    /// The totals line of a run that wrote `outcome`.
    fn applied(outcome: &ApplyOutcome) -> String {
        format!(
            "applied: {} edited, {} created, {} deleted, {} skipped, {} refused",
            outcome.edited,
            outcome.created,
            outcome.deleted,
            outcome.skipped,
            outcome.refused.len()
        )
    }
}

/// Renders `count` of `noun`, adding an `s` unless it is one.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}
