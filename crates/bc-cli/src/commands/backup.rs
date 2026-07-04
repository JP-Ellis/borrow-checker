//! `backup` subcommand: snapshot the database.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `backup` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Write the snapshot to this exact path instead of the managed backup
    /// directory (and skip rotation).
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// JSON representation of a completed backup, mirroring `bc-app`'s
/// `BackupInfo` mapping for cross-surface consistency.
#[non_exhaustive]
#[derive(Debug, serde::Serialize)]
struct BackupRecordJson {
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
/// Returns a [`crate::error::CliError`] if the snapshot fails.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    let rec = ctx
        .backup
        .backup(bc_core::BackupKind::Manual, args.output.as_deref())
        .await?;
    let record_json = BackupRecordJson::from(&rec);
    if ctx.json {
        return crate::output::print_json(&record_json);
    }
    crate::output::print_table(
        &["Path", "Kind", "Created", "Size"],
        &[vec![
            record_json.path,
            record_json.kind,
            record_json.created_at,
            record_json.size_bytes.to_string(),
        ]],
    );
    Ok(())
}
