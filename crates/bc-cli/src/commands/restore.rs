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
/// closes the live connection pool, then swaps the candidate in as the live
/// database, keeping the live ledger ID and clearing any stale WAL sidecars.
/// The pool is closed before the swap so no WAL connection can checkpoint
/// stale frames onto the restored file; `restore` is terminal, so closing the
/// shared pool is safe.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the candidate is invalid, another
/// process holds the database's lock ([`bc_core::BcError::DatabaseInUse`]),
/// or a filesystem/database operation fails.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    bc_core::BackupService::validate(&args.path).await?;
    // Refuses while the desktop app or the server has the database open.
    let _lock = bc_core::DbLock::acquire(&ctx.db_path)?;
    ctx.backup.pre_restore_snapshot().await?;
    ctx.backup.close_pool().await;
    bc_core::BackupService::swap_in(&args.path, &ctx.db_path).await?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Restored database from {}", args.path.display());
    }
    Ok(())
}
