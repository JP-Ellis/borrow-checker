//! `sync` — run one import profile or every profile through the shared engine.
//!
//! `import run` is the detailed single-profile tool; this is the sweep: one
//! row per profile, blockers under each row in a dry run, a totals line, and
//! the snapshot path when one was taken. The rows are a view over
//! [`bc_core::SyncReport`] that tests can build directly, since the engine's
//! result types are `#[non_exhaustive]`.

use std::fmt::Write as _;
use std::path::Path;

use crate::commands::import::PlanReport;
use crate::commands::import::Report;
use crate::commands::import::failure_error;
use crate::commands::import::plural;
use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Arguments for `sync`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
#[command(group = clap::ArgGroup::new("target").required(true))]
pub struct Args {
    /// Run one profile, by name.
    #[arg(long, value_name = "NAME", group = "target")]
    pub profile: Option<String>,

    /// Run every profile, in name order.
    #[arg(long, group = "target")]
    pub all: bool,

    /// Report what each profile would do, without writing anything.
    ///
    /// Takes no snapshot and opens no batch. Exits non-zero when any profile
    /// has a blocker — an unresolved account, an unresolved commodity, or a
    /// posting skipped for any other cause — so a script can gate on it.
    #[arg(long)]
    pub dry_run: bool,
}

/// Executes `sync`.
///
/// # Arguments
///
/// * `args` - The parsed arguments.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns [`CliError`] if `--profile` names no profile, the snapshot cannot
/// be written, or — after the report has printed — any profile failed, or
/// under `--dry-run` any profile has a blocker. Committed batches stay
/// committed either way; the report names the snapshot and each batch id.
///
/// A single profile's failure is the same error `import run` returns for
/// it, so an engine-stage error keeps its exit code under either command.
/// A sweep's failures share one exit code: several profiles have no single
/// error to forward.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    let single = args.profile.is_some();
    let selection = match args.profile {
        Some(name) => bc_core::ImportSelection::One(name),
        None => bc_core::ImportSelection::All,
    };
    let mode = if args.dry_run {
        bc_core::ImportMode::DryRun
    } else {
        bc_core::ImportMode::Commit
    };

    let report = ctx.engine.sync(selection, mode).await?;
    let rows: Vec<Row> = report.profiles.iter().map(Row::from).collect();
    let summary = Summary::new(rows, report.snapshot.as_deref(), args.dry_run);

    if ctx.json {
        crate::output::print_json(&to_json(&report, args.dry_run))?;
    } else {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            print!("{}", summary.render());
        }
    }

    let exit = summary.exit_error();
    if single
        && let Some(bc_core::ProfileResult {
            result: Err(failure),
            ..
        }) = report.profiles.into_iter().next()
    {
        return Err(failure_error(failure));
    }
    exit.map_or(Ok(()), Err)
}

/// One profile's line in the human report.
struct Row {
    /// The profile's name.
    profile: String,
    /// What happened to it.
    outcome: RowOutcome,
}

/// The three shapes a profile's row can take.
enum RowOutcome {
    /// A committed run.
    Imported {
        /// Transactions created.
        new_transactions: usize,
        /// Legs attached to earlier runs' transactions.
        attached_postings: usize,
        /// Postings skipped, whatever the cause.
        skipped_postings: usize,
        /// Advisory warnings raised.
        warnings: usize,
        /// The batch id, for `import discard`.
        batch: String,
    },
    /// A dry run.
    Planned {
        /// Transactions the run would create.
        new_transactions: usize,
        /// Legs it would attach.
        attached_postings: usize,
        /// Postings it would skip.
        skipped_postings: usize,
        /// Advisory warnings raised (a lower bound; see `import run`).
        warnings: usize,
        /// Each blocker's label and its items.
        blockers: Vec<(String, Vec<String>)>,
    },
    /// The profile produced nothing, or stopped part-way through.
    Failed {
        /// The stage label.
        stage: String,
        /// The failure message.
        message: String,
        /// The batch a stopped run left open, for `import discard`.
        batch: Option<String>,
    },
}

impl From<&bc_core::ProfileResult> for Row {
    #[inline]
    fn from(result: &bc_core::ProfileResult) -> Self {
        Self {
            profile: result.profile.name.clone(),
            outcome: RowOutcome::from(&result.result),
        }
    }
}

impl From<&Result<bc_core::ProfileRun, bc_core::ProfileFailure>> for RowOutcome {
    #[inline]
    fn from(result: &Result<bc_core::ProfileRun, bc_core::ProfileFailure>) -> Self {
        match result {
            Ok(bc_core::ProfileRun::Imported(outcome)) => Self::Imported {
                new_transactions: outcome.new_transactions,
                attached_postings: outcome.attached_postings,
                skipped_postings: outcome.skipped_postings,
                warnings: outcome.warnings.len(),
                batch: outcome.batch_id.to_string(),
            },
            Ok(bc_core::ProfileRun::Planned(plan)) => Self::Planned {
                new_transactions: plan.new_transactions,
                attached_postings: plan.attached_postings,
                skipped_postings: plan.skipped_postings,
                warnings: plan.warnings.len(),
                blockers: plan
                    .blockers()
                    .iter()
                    .map(|blocker| (blocker.label().to_owned(), blocker.items()))
                    .collect(),
            },
            Ok(other) => Self::Failed {
                stage: "engine".to_owned(),
                message: format!("unexpected engine result: {other:?}"),
                batch: None,
            },
            // The stage label already names the layer, so the row carries
            // the bare message and not `Display`'s `import error: ` prefix.
            Err(failure) => Self::Failed {
                stage: failure.stage.label().to_owned(),
                message: failure.message.clone(),
                batch: failure.batch_id.as_ref().map(ToString::to_string),
            },
        }
    }
}

/// The whole human report: rows, the snapshot, and the exit policy.
struct Summary {
    /// One per profile, in the order they ran.
    rows: Vec<Row>,
    /// The pre-import snapshot, when one was taken.
    snapshot: Option<String>,
    /// Whether this was a dry run, which changes the columns and the exit rule.
    dry_run: bool,
}

/// Width of the numeric columns.
const NUM_WIDTH: usize = 8;

impl Summary {
    /// Builds the summary.
    fn new(rows: Vec<Row>, snapshot: Option<&Path>, dry_run: bool) -> Self {
        Self {
            rows,
            snapshot: snapshot.map(|path| path.display().to_string()),
            dry_run,
        }
    }

    /// Profiles that produced nothing.
    fn failed(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.outcome, RowOutcome::Failed { .. }))
            .count()
    }

    /// Planned profiles with at least one blocker.
    fn blocked(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(&row.outcome, RowOutcome::Planned { blockers, .. } if !blockers.is_empty()))
            .count()
    }

    /// The error `execute` returns after printing, or `None` for exit 0.
    ///
    /// A failed profile fails the command in either mode. A blocker fails it
    /// only under `--dry-run`: that is the gate a script sequences before the
    /// real sweep. A committed profile that skipped legs is reported, not
    /// fatal — the re-run after `account create` attaches them.
    fn exit_error(&self) -> Option<CliError> {
        let failed = self.failed();
        let blocked = if self.dry_run { self.blocked() } else { 0 };
        let mut parts = Vec::new();
        if failed > 0 {
            parts.push(format!("{} failed", plural(failed, "profile")));
        }
        if blocked > 0 {
            parts.push(format!("{} blocked", plural(blocked, "profile")));
        }
        if parts.is_empty() {
            None
        } else {
            Some(CliError::Arg(parts.join(", ")))
        }
    }

    /// Renders the report, newline-terminated.
    #[expect(
        clippy::let_underscore_must_use,
        reason = "write! to a String is infallible; there is no error to surface from a \
                   function returning String rather than fmt::Result"
    )]
    fn render(&self) -> String {
        // Implicit format arguments capture locals, not consts.
        let num_width = NUM_WIDTH;
        let name_width = self
            .rows
            .iter()
            .map(|row| row.profile.chars().count())
            .max()
            .unwrap_or(7)
            .max(7);

        let mut lines: Vec<String> = Vec::new();
        let mut header = format!(
            "{:<name_width$}  {:>num_width$}  {:>num_width$}  {:>num_width$}  {:>num_width$}",
            "profile", "txns", "attached", "skipped", "warnings"
        );
        if !self.dry_run {
            header.push_str("  batch");
        }
        lines.push(header);

        let mut total_txns = 0_usize;
        let mut total_skipped = 0_usize;
        for row in &self.rows {
            match &row.outcome {
                RowOutcome::Imported {
                    new_transactions,
                    attached_postings,
                    skipped_postings,
                    warnings,
                    batch,
                } => {
                    total_txns = total_txns.saturating_add(*new_transactions);
                    total_skipped = total_skipped.saturating_add(*skipped_postings);
                    lines.push(format!(
                        "{:<name_width$}  {new_transactions:>num_width$}  {attached_postings:>num_width$}  {skipped_postings:>num_width$}  {warnings:>num_width$}  {batch}",
                        row.profile
                    ));
                }
                RowOutcome::Planned {
                    new_transactions,
                    attached_postings,
                    skipped_postings,
                    warnings,
                    blockers,
                } => {
                    total_txns = total_txns.saturating_add(*new_transactions);
                    total_skipped = total_skipped.saturating_add(*skipped_postings);
                    lines.push(format!(
                        "{:<name_width$}  {new_transactions:>num_width$}  {attached_postings:>num_width$}  {skipped_postings:>num_width$}  {warnings:>num_width$}",
                        row.profile
                    ));
                    for (label, items) in blockers {
                        for item in items {
                            lines.push(format!("    {label:<22}{item}"));
                        }
                    }
                }
                RowOutcome::Failed {
                    stage,
                    message,
                    batch,
                } => {
                    let dash = format!("{:>num_width$}", "—");
                    lines.push(format!(
                        "{:<name_width$}  {dash}  {dash}  {dash}  {dash}  failed ({stage}): {message}",
                        row.profile
                    ));
                    if let Some(open) = batch {
                        lines.push(format!(
                            "    batch {open} left open; `import discard {open}` undoes it"
                        ));
                    }
                }
            }
        }

        lines.push(String::new());
        let verb = if self.dry_run {
            "would be imported"
        } else {
            "imported"
        };
        let mut total = plural(self.rows.len(), "profile");
        let failed = self.failed();
        if failed > 0 {
            let _ = write!(total, ", {failed} failed");
        }
        let blocked = self.blocked();
        if self.dry_run && blocked > 0 {
            let _ = write!(total, ", {blocked} blocked");
        }
        let _ = write!(
            total,
            ". {} {verb}, {} skipped.",
            plural(total_txns, "transaction"),
            plural(total_skipped, "posting")
        );
        lines.push(total);
        if let Some(snapshot) = &self.snapshot {
            lines.push(format!("snapshot: {snapshot}"));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

/// One blocker for `--json`.
///
/// `items` are the identifiers a script acts on: account paths and
/// commodity codes as strings, and for `other_skips` a `{cause, count}`
/// object per cause — the human line's `<cause> ×<count>` is not for
/// parsing.
fn blocker_to_json(blocker: &bc_core::Blocker) -> serde_json::Value {
    let items = match blocker {
        bc_core::Blocker::OtherSkips(charged) => charged
            .iter()
            .map(|(cause, count)| {
                serde_json::json!({
                    "cause": cause.label(),
                    "count": count,
                })
            })
            .collect(),
        bc_core::Blocker::UnresolvedAccounts(_)
        | bc_core::Blocker::UnresolvedCommodities(_)
        | _ => blocker
            .items()
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    };
    serde_json::json!({
        "kind": blocker.kind(),
        "items": serde_json::Value::Array(items),
    })
}

/// Builds the `--json` payload: the per-profile object is the `import run`
/// payload for that mode, so a script that reads one reads the other.
pub(crate) fn to_json(report: &bc_core::SyncReport, dry_run: bool) -> serde_json::Value {
    let profiles: Vec<serde_json::Value> = report
        .profiles
        .iter()
        .map(|result| {
            let mut object = serde_json::json!({
                "profile": result.profile.name,
                "importer": result.profile.importer,
                "ok": result.result.is_ok(),
                "failure": serde_json::Value::Null,
            });
            let Some(map) = object.as_object_mut() else {
                return object;
            };
            match &result.result {
                Ok(bc_core::ProfileRun::Imported(outcome)) => {
                    let payload = Report::from(outcome).to_json(&outcome.batch_id.to_string());
                    if let Some(fields) = payload.as_object() {
                        map.extend(fields.clone());
                    }
                }
                Ok(bc_core::ProfileRun::Planned(plan)) => {
                    let payload = PlanReport::from(plan).to_json();
                    if let Some(fields) = payload.as_object() {
                        map.extend(fields.clone());
                    }
                    map.insert(
                        "blockers".to_owned(),
                        serde_json::Value::Array(
                            plan.blockers().iter().map(blocker_to_json).collect(),
                        ),
                    );
                }
                Ok(other) => {
                    map.insert("ok".to_owned(), serde_json::Value::Bool(false));
                    map.insert(
                        "failure".to_owned(),
                        serde_json::json!({
                            "stage": "engine",
                            "message": format!("unexpected engine result: {other:?}"),
                            "batch": serde_json::Value::Null,
                        }),
                    );
                }
                Err(failure) => {
                    map.insert(
                        "failure".to_owned(),
                        serde_json::json!({
                            "stage": failure.stage.label(),
                            "message": failure.message,
                            "batch": failure.batch_id.as_ref().map(ToString::to_string),
                        }),
                    );
                }
            }
            object
        })
        .collect();

    serde_json::json!({
        "dry_run": dry_run,
        "snapshot": report.snapshot.as_ref().map(|path| path.display().to_string()),
        "profiles": profiles,
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use clap::Parser as _;
    use pretty_assertions::assert_eq;

    use super::Args;
    use super::Row;
    use super::RowOutcome;
    use super::Summary;

    /// Wrapper so `Args` can be parsed on its own.
    #[derive(Debug, clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: Args,
    }

    fn imported(name: &str, txns: usize, skipped: usize, warnings: usize) -> Row {
        Row {
            profile: name.to_owned(),
            outcome: RowOutcome::Imported {
                new_transactions: txns,
                attached_postings: 0,
                skipped_postings: skipped,
                warnings,
                batch: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned(),
            },
        }
    }

    /// `skipped` is given, not derived: an `other skips` item carries its
    /// own `×N`, so the count is not the number of items.
    fn planned(name: &str, txns: usize, skipped: usize, blockers: Vec<(&str, Vec<&str>)>) -> Row {
        Row {
            profile: name.to_owned(),
            outcome: RowOutcome::Planned {
                new_transactions: txns,
                attached_postings: 0,
                skipped_postings: skipped,
                warnings: 0,
                blockers: blockers
                    .into_iter()
                    .map(|(label, items)| {
                        (
                            label.to_owned(),
                            items.into_iter().map(str::to_owned).collect(),
                        )
                    })
                    .collect(),
            },
        }
    }

    fn failed(name: &str, stage: &str, message: &str) -> Row {
        Row {
            profile: name.to_owned(),
            outcome: RowOutcome::Failed {
                stage: stage.to_owned(),
                message: message.to_owned(),
                batch: None,
            },
        }
    }

    /// A run that stopped after opening its batch.
    fn stopped(name: &str, message: &str, batch: &str) -> Row {
        Row {
            profile: name.to_owned(),
            outcome: RowOutcome::Failed {
                stage: "engine".to_owned(),
                message: message.to_owned(),
                batch: Some(batch.to_owned()),
            },
        }
    }

    #[test]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "Wrapper derives Debug for the CLI, which makes unwrap_err's bound \
                   satisfiable here; is_err reads as a parse-rejection check, not a \
                   value inspection"
    )]
    fn profile_and_all_are_mutually_exclusive() {
        assert!(Wrapper::try_parse_from(["bc", "--profile", "a", "--all"]).is_err());
    }

    #[test]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "Wrapper derives Debug for the CLI, which makes unwrap/unwrap_err's bound \
                   satisfiable here; is_ok/is_err read as parse-outcome checks, not value \
                   inspection"
    )]
    fn one_of_profile_or_all_is_required() {
        assert!(Wrapper::try_parse_from(["bc"]).is_err());
        assert!(Wrapper::try_parse_from(["bc", "--all"]).is_ok());
        assert!(Wrapper::try_parse_from(["bc", "--profile", "a", "--dry-run"]).is_ok());
    }

    #[test]
    fn a_failed_row_carries_the_bare_message() {
        let result = Err(bc_core::ProfileFailure::new(
            bc_core::FailureStage::Importer,
            "no such file: nab.csv",
        ));

        let outcome = RowOutcome::from(&result);

        assert!(
            matches!(
                &outcome,
                RowOutcome::Failed { stage, message, batch: None }
                    if stage == "importer" && message == "no such file: nab.csv"
            ),
            "the stage label names the layer; the message must not repeat it"
        );
    }

    #[test]
    fn other_skips_serialise_as_cause_and_count() {
        let blocker = bc_core::Blocker::OtherSkips(vec![
            (bc_core::SkipCause::AmbiguousResidual, 3_usize),
            (bc_core::SkipCause::BlankCommodity, 1_usize),
        ]);

        let json = super::blocker_to_json(&blocker);

        assert_eq!(
            json,
            serde_json::json!({
                "kind": "other_skips",
                "items": [
                    {"cause": "ambiguous residual", "count": 3_usize},
                    {"cause": "blank commodity code", "count": 1_usize},
                ],
            })
        );
    }

    #[test]
    fn unresolved_accounts_serialise_as_paths() {
        let blocker = bc_core::Blocker::UnresolvedAccounts(vec!["Expenses:Food".to_owned()]);

        let json = super::blocker_to_json(&blocker);

        assert_eq!(
            json,
            serde_json::json!({
                "kind": "unresolved_account",
                "items": ["Expenses:Food"],
            })
        );
    }

    #[test]
    fn a_stopped_run_renders_the_batch_it_left_open() {
        let summary = Summary::new(
            vec![
                imported("nab-credit", 88, 0, 0),
                stopped(
                    "ubank-everyday",
                    "database error: disk full",
                    "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                ),
            ],
            None,
            false,
        );
        insta::assert_snapshot!(summary.render());
        assert_eq!(
            summary.exit_error().map(|e| e.to_string()),
            Some("1 profile failed".to_owned())
        );
    }

    #[test]
    fn a_clean_commit_sweep_renders_rows_totals_and_the_snapshot() {
        let summary = Summary::new(
            vec![
                imported("nab-credit", 88, 0, 0),
                imported("ubank-everyday", 412, 0, 2),
            ],
            Some(std::path::Path::new(
                "/backups/db.2026-09-13.pre-import.sqlite",
            )),
            false,
        );
        insta::assert_snapshot!(summary.render());
        assert!(summary.exit_error().is_none());
    }

    #[test]
    fn a_failed_profile_renders_and_fails_the_exit_code() {
        let summary = Summary::new(
            vec![
                failed("nab-credit", "importer", "no such file: nab.csv"),
                imported("ubank-everyday", 412, 0, 0),
            ],
            Some(std::path::Path::new("/backups/db.pre-import.sqlite")),
            false,
        );
        insta::assert_snapshot!(summary.render());
        assert_eq!(
            summary.exit_error().map(|e| e.to_string()),
            Some("1 profile failed".to_owned())
        );
    }

    #[test]
    fn a_dry_run_lists_blockers_under_each_row_and_fails_the_exit_code() {
        let summary = Summary::new(
            vec![
                planned(
                    "amp-saver",
                    31,
                    5,
                    vec![
                        (
                            "unresolved account",
                            vec!["Expenses:Food:Cafes", "Income:Interest"],
                        ),
                        ("other skips", vec!["ambiguous residual ×3"]),
                    ],
                ),
                planned("ubank-everyday", 412, 0, vec![]),
            ],
            None,
            true,
        );
        insta::assert_snapshot!(summary.render());
        assert_eq!(
            summary.exit_error().map(|e| e.to_string()),
            Some("1 profile blocked".to_owned())
        );
    }

    #[test]
    fn a_clean_dry_run_exits_zero() {
        let summary = Summary::new(vec![planned("a", 1, 0, vec![])], None, true);
        assert!(summary.exit_error().is_none());
    }

    #[test]
    fn blocked_and_failed_are_both_named_in_the_exit_error() {
        let summary = Summary::new(
            vec![
                planned("a", 1, 1, vec![("unresolved account", vec!["X:Y"])]),
                failed("b", "importer", "boom"),
                failed("c", "engine", "boom"),
            ],
            None,
            true,
        );
        assert_eq!(
            summary.exit_error().map(|e| e.to_string()),
            Some("2 profiles failed, 1 profile blocked".to_owned())
        );
    }

    #[test]
    fn a_commit_sweep_ignores_skips_for_the_exit_code() {
        // Warn, don't block: a committed profile that skipped legs is reported,
        // and the re-run after `account create` attaches them.
        let summary = Summary::new(vec![imported("a", 10, 4, 0)], None, false);
        assert!(summary.exit_error().is_none());
    }

    #[test]
    fn an_empty_sweep_renders_a_zero_total() {
        let summary = Summary::new(Vec::new(), None, false);
        insta::assert_snapshot!(summary.render());
        assert!(summary.exit_error().is_none());
    }
}
