//! `backup` subcommand: snapshot, list, delete and rekey the ledger's backups.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `backup` subcommand.
///
/// Without a subcommand, takes a manual snapshot.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    /// Write the snapshot to this exact path instead of the managed backup
    /// directory (and skip rotation).
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// A backup operation other than taking a snapshot.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Backup operations other than taking a snapshot.
#[derive(Debug, clap::Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List this ledger's backups, newest first.
    List,
    /// Delete one of this ledger's backups by file name.
    Delete {
        /// The backup's file name, as `backup list` shows it.
        file_name: String,
    },
    /// Give this database a fresh ledger ID, starting a new backup pool.
    ///
    /// Run this on a copy of a database file, which otherwise shares its
    /// source's backups.
    Rekey,
}

/// JSON representation of a completed backup, mirroring `bc-app`'s
/// `BackupInfo` mapping for cross-surface consistency.
#[non_exhaustive]
#[derive(Debug, serde::Serialize)]
struct BackupRecordJson {
    /// The backup's file name, accepted by `backup delete`.
    file_name: String,
    /// Filesystem path the snapshot was written to.
    path: String,
    /// Backup kind suffix (e.g. `"manual"`).
    kind: String,
    /// Timestamp the snapshot was taken, formatted as a string.
    created_at: String,
    /// Size of the snapshot file in bytes.
    size_bytes: u64,
}

impl From<&bc_core::BackupRecord> for BackupRecordJson {
    fn from(rec: &bc_core::BackupRecord) -> Self {
        Self {
            file_name: rec
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: rec.path.display().to_string(),
            kind: rec.kind.suffix().to_owned(),
            created_at: rec.created_at.to_string(),
            size_bytes: rec.size_bytes,
        }
    }
}

/// Executes the `backup` subcommand.
///
/// # Errors
///
/// Returns a [`crate::error::CliError`] if the operation fails.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        None => snapshot(ctx, args.output.as_deref()).await,
        Some(Command::List) => list(ctx),
        Some(Command::Delete { file_name }) => delete(ctx, &file_name),
        Some(Command::Rekey) => rekey(ctx).await,
    }
}

/// Takes a manual snapshot.
async fn snapshot(ctx: &AppContext, output: Option<&std::path::Path>) -> CliResult<()> {
    let rec = ctx
        .backup
        .backup(bc_core::BackupKind::Manual, output)
        .await?;
    if ctx.json {
        return crate::output::print_json(&BackupRecordJson::from(&rec));
    }
    print_records(ctx, &[rec], true)
}

/// Lists this ledger's backups.
fn list(ctx: &AppContext) -> CliResult<()> {
    let records = ctx.backup.list()?;
    print_records(ctx, &records, false)
}

/// Prints records as a JSON array or a table.
///
/// The first table column is the full path when `show_path` is set (a
/// snapshot reports where it wrote) and the file name otherwise (`delete`
/// takes that name).
fn print_records(
    ctx: &AppContext,
    records: &[bc_core::BackupRecord],
    show_path: bool,
) -> CliResult<()> {
    let rows: Vec<BackupRecordJson> = records.iter().map(BackupRecordJson::from).collect();
    if ctx.json {
        return crate::output::print_json(&rows);
    }
    crate::output::print_table(
        &[
            if show_path { "Path" } else { "File" },
            "Kind",
            "Created",
            "Size",
        ],
        &rows
            .into_iter()
            .map(|r| {
                let first = if show_path { r.path } else { r.file_name };
                vec![first, r.kind, r.created_at, r.size_bytes.to_string()]
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}

/// Deletes one backup by file name.
fn delete(ctx: &AppContext, file_name: &str) -> CliResult<()> {
    ctx.backup.delete(file_name)?;
    if ctx.json {
        return crate::output::print_json(&file_name);
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Deleted backup {file_name}");
    }
    Ok(())
}

/// Mints a fresh ledger ID for the open database.
async fn rekey(ctx: &AppContext) -> CliResult<()> {
    // A running app or server would keep writing to the old pool.
    let _lock = bc_core::DbLock::acquire(ctx.backup.db_path())?;
    let (old, new) = ctx.backup.rekey().await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "old": old.to_string(),
            "new": new.to_string(),
        }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Ledger ID {old} -> {new}");
    }
    Ok(())
}
