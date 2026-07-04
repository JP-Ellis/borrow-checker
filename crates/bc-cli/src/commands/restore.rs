//! `restore` subcommand: replace the database from a backup file.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `restore` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Path to the backup file to restore from.
    pub path: PathBuf,
}

/// Executes the `restore` subcommand.
///
/// Validates the candidate, snapshots the current database as a safety backup,
/// then overwrites the live database file with the candidate. The CLI process
/// holds no long-lived writers, so the swap is safe to do in place.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the candidate is invalid or a
/// filesystem/database operation fails.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    bc_core::BackupService::validate(&args.path).await?;
    ctx.backup
        .backup(bc_core::BackupKind::Automatic, None)
        .await?;
    std::fs::copy(&args.path, &ctx.db_path)?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Restored database from {}", args.path.display());
    }
    Ok(())
}
