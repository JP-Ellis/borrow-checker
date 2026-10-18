//! Export sub-command: renders the ledger through an exporter plugin.
//!
//! A working export writes one file and, beside it, the sidecar that apply
//! resolves handles through. An archive export writes every file the plugin
//! returns under one directory.

use std::io::Write as _;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use bc_core::ExportError;
use bc_core::ExportFile;
use bc_core::ExportLevel;
use bc_core::Exporter as _;
use bc_core::roundtrip::Sidecar;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;
use crate::period::PeriodArg;
use crate::period::PeriodInputs;

// MARK: Arguments

/// How much bookkeeping the export carries.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum LevelArg {
    /// What a person edits day to day, with a sidecar for apply.
    Working,
    /// Everything, for a complete and durable copy.
    Archive,
}

impl From<LevelArg> for ExportLevel {
    #[inline]
    fn from(level: LevelArg) -> Self {
        match level {
            LevelArg::Working => Self::Working,
            LevelArg::Archive => Self::Archive,
        }
    }
}

/// Arguments for the `export` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Output format: the name of a loaded exporter plugin (e.g. `beancount`).
    #[arg(long)]
    pub format: String,

    /// How much of the ledger's bookkeeping to export.
    #[arg(long, value_enum, default_value = "working")]
    pub level: LevelArg,

    /// Export the period instance containing `--date`.
    #[arg(long, value_enum, conflicts_with_all = ["from", "until"])]
    pub period: Option<PeriodArg>,

    /// A date within the `--period` (YYYY-MM-DD). Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD", requires = "period")]
    pub date: Option<String>,

    /// Export transactions on or after this date (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub from: Option<String>,

    /// Export transactions before this date (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub until: Option<String>,

    /// Export only transactions matching this query.
    #[arg(long, value_name = "QUERY")]
    pub query: Option<String>,

    /// Where to write. A file for a working export, a directory for an
    /// archive. An archive of a single file may go to stdout when omitted.
    #[arg(long, short = 'o', value_name = "OUTPUT")]
    pub output: Option<PathBuf>,
}

// MARK: Execution

/// Executes the `export` subcommand.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for an unknown or non-exporting format, a
/// working export without `-o`, an archive of several files without `-o`, a
/// name the format cannot spell (listing every one) and an exported path that
/// would leave the output directory. Returns [`CliError::Core`] for a bad
/// query or a database failure, and [`CliError::Io`] when a file cannot be
/// written.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    let exporter = ctx
        .plugin_registry
        .exporter(&args.format)
        .ok_or_else(|| CliError::Arg(format!("no exporter named '{}' is loaded", args.format)))?;
    let level = ExportLevel::from(args.level);
    if matches!(level, ExportLevel::Working) && args.output.is_none() {
        return Err(CliError::Arg(
            "a working export needs -o <file>: the sidecar is written beside it".to_owned(),
        ));
    }

    let (from, until) = window(&args, ctx)?;
    let query = ctx
        .transactions
        .parse_query(args.query.as_deref().unwrap_or_default(), from, until)
        .await?;
    if matches!(level, ExportLevel::Archive)
        && (from.is_some() || until.is_some() || args.query.is_some())
    {
        crate::output::warn_all(&[
            "the archive's balance assertions cover only the exported transactions",
        ]);
    }

    let export = ctx.round_trip.export(&query, level).await?;
    let files = exporter
        .export(&export.snapshot)
        .map_err(|error| match error {
            ExportError::Unspellable(names) => CliError::Arg(format!(
                "{} cannot express these names: {}",
                args.format,
                names.join(", ")
            )),
            ExportError::Unsupported => {
                CliError::Arg(format!("'{}' does not support export", args.format))
            }
            other @ (ExportError::AccountNotFound(_)
            | ExportError::CommodityNotFound(_)
            | ExportError::Write(_)
            | _) => CliError::Arg(other.to_string()),
        })?;

    match (level, args.output.as_deref()) {
        (ExportLevel::Working, Some(path)) => {
            let [file] = files.as_slice() else {
                return Err(CliError::Arg(format!(
                    "a working export must be one file, but '{}' produced {}",
                    args.format,
                    files.len()
                )));
            };
            write_file(path, &file.contents)?;
            if let Some(sidecar) = &export.sidecar {
                sidecar.write(&Sidecar::path_for(path))?;
            }
            report(
                ctx,
                export.snapshot.transactions.len(),
                &path.display().to_string(),
                &[path.to_path_buf()],
            )
        }
        (_, Some(dir)) => {
            let written = write_tree(dir, &files)?;
            report(
                ctx,
                export.snapshot.transactions.len(),
                &dir.display().to_string(),
                &written,
            )
        }
        (_, None) => match files.as_slice() {
            [file] => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(file.contents.as_bytes())?;
                Ok(())
            }
            _ => Err(CliError::Arg(format!(
                "this export is {} files; give -o <directory>",
                files.len()
            ))),
        },
    }
}

// MARK: Helpers

/// Resolves `--period`/`--date` or `--from`/`--until` into a date window.
fn window(
    args: &Args,
    ctx: &AppContext,
) -> CliResult<(Option<jiff::civil::Date>, Option<jiff::civil::Date>)> {
    if let Some(period) = args.period {
        if matches!(period, PeriodArg::Custom) {
            return Err(CliError::Arg(
                "--period custom is not supported by export; use --from and --until".to_owned(),
            ));
        }
        let inputs = PeriodInputs {
            fortnightly_anchor: ctx.fortnightly_anchor,
            duration_days: None,
            duration_weeks: None,
            duration_months: None,
            fy_start_month: ctx.fy_start_month,
            fy_start_day: ctx.fy_start_day,
        };
        let anchor = crate::commands::parse_date_or_today(args.date.as_deref())?;
        let (start, end) = crate::period::resolve(period, &inputs)?.range_containing(anchor);
        return Ok((Some(start), Some(end)));
    }
    let parse = |value: Option<&str>| {
        value
            .map(|v| crate::commands::parse_date_or_today(Some(v)))
            .transpose()
    };
    Ok((parse(args.from.as_deref())?, parse(args.until.as_deref())?))
}

/// Writes `contents` to `path`, creating its parent directories.
fn write_file(path: &Path, contents: &str) -> CliResult<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

/// Writes every file under `dir`, refusing a path that leaves it.
///
/// Every path is checked before the first write.
fn write_tree(dir: &Path, files: &[ExportFile]) -> CliResult<Vec<PathBuf>> {
    let targets: Vec<PathBuf> = files
        .iter()
        .map(|file| {
            let relative = Path::new(&file.path);
            let contained = relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
            if contained {
                Ok(dir.join(relative))
            } else {
                Err(CliError::Arg(format!(
                    "the exporter returned the path '{}', which is outside the output directory",
                    file.path
                )))
            }
        })
        .collect::<CliResult<_>>()?;
    for (target, file) in targets.iter().zip(files) {
        write_file(target, &file.contents)?;
    }
    Ok(targets)
}

/// Reports what was written to `destination`.
fn report(
    ctx: &AppContext,
    transactions: usize,
    destination: &str,
    paths: &[PathBuf],
) -> CliResult<()> {
    if ctx.json {
        let files: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        crate::output::print_json(&serde_json::json!({
            "transactions": transactions,
            "files": files,
        }))?;
    } else {
        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "exported {transactions} transaction(s) to {destination}"
        )?;
    }
    Ok(())
}
