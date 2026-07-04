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
    let path = rec.path.display().to_string();
    if ctx.json {
        return crate::output::print_json(&path);
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Backup written to {path}");
    }
    Ok(())
}
